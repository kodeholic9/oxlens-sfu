// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-FLR-LOCK 정:IMPL-FLR-MEDIA-BIND 정:IMPL-FLR-RTP-OBSERVE 정:CAND-FWD-PRIME-SOURCE 정:CAND-FWD-PRIME-EPISODE 정:CAND-RTCP-RTX-GATE-WINDOW 정:IMPL-AUX-DC-SEND 운영:RULE-LOG-TRANSITION 연:RULE-FLR-TLV 연:RULE-FLR-MSG-TYPES 연:RULE-FLR-ROOM-KEY 연:RULE-FLR-MSG-REQUEST 연:TRN.frame.DC · model: claude-opus-5-5
//! 발언권 집행 — 판정이 돌려준 목록을 락 밖에서: DC 송신(방 TLV 늘) · 미디어 결합(슬롯 · priming CN · PLI burst [0, 500, 1500]) · 해제(무음 3 프레임) · 전이 줄. DC 수신 고리.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::frame::{DC_SVC_FLOOR, dc_decode, dc_encode};
use oxsig::mbcp::{self, Msg, tlv};
use oxsig::msg::Kind;

use super::Sfu;
use super::peer::Peer;
use crate::flr::Step;
use crate::flr::core::{Fx, G, M, Out, T};
use crate::fwd::Bound;
use crate::fwd::shell::Synth;
use crate::fwd::shell::now_ms;
use crate::room::Room;

pub const PRIME_FRAMES: u32 = 250;
pub const PRIME_MS: u64 = 20;
pub const RELEASE_SILENCE: u32 = 3;
pub const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const PLI_BURST: [u64; 3] = [0, 500, 1500];

fn encode(room: &str, o: &Out) -> Option<Vec<u8>> {
    let m = &o.m;
    let msg = match m.t {
        T::Granted => Msg::new(mbcp::GRANTED, true).with_u8spare(tlv::PRIORITY, m.n as u8).with_u16(tlv::DURATION, m.d as u16).with_u16(tlv::SEQ, m.q as u16),
        T::Taken => Msg::new(mbcp::TAKEN, true).with_str(tlv::SPEAKER, m.a.as_deref().unwrap_or("")).with_u16(tlv::SEQ, m.q as u16),
        T::Deny => {
            let d = Msg::new(mbcp::DENY, true).with_u8(tlv::CAUSE, m.n as u8);
            if m.n == u32::from(mbcp::deny::OTHER_REASON) { d.with_str(tlv::REASON, "not_in_room") } else { d }
        }
        T::Idle => {
            let i = Msg::new(mbcp::IDLE, false);
            let i = match m.a.as_deref() {
                Some(p) => i.with_str(tlv::PREV, p),
                None => i,
            };
            i.with_u16(tlv::SEQ, m.q as u16)
        }
        T::Revoke => Msg::new(mbcp::REVOKE, false).with_u8(tlv::CAUSE, m.n as u8),
        T::Qinfo => Msg::new(mbcp::QINFO, false).with_pair(tlv::QINFO, m.n.min(255) as u8, m.d.min(255) as u8),
        _ => return None,
    };
    let b = mbcp::encode(&msg.with_str(tlv::ROOM, room))?;
    dc_encode(DC_SVC_FLOOR, &b)
}

pub fn decode(m: &Msg) -> Option<M> {
    let t = match m.msg_type {
        mbcp::REQUEST => T::Request,
        mbcp::RELEASE => T::Release,
        mbcp::QPOS_REQ => T::QposReq,
        mbcp::ACK => T::Ack,
        _ => return None,
    };
    let n = match t {
        T::Request => u32::from(m.priority()),
        T::Ack => u32::from(m.u8_of(tlv::ACKTYPE).unwrap_or(0)),
        _ => 0,
    };
    Some(M { t, a: None, n, d: 0, q: u32::from(m.seq().unwrap_or(0)) })
}

pub async fn exec(sfu: &Arc<Sfu>, room: &Arc<Room>, st: Step) {
    for (label, ins) in &st.labels {
        common::log::transition(&common::log::Step {
            area: "FLR",
            key: &room.id,
            from: st.from.name(),
            to: Some(st.to.name()),
            cause: st.cause,
            label: Some(label),
            ins,
            room: Some(&room.id),
            user: st.user.as_deref(),
            arev: None,
        });
    }
    for o in &st.out {
        let Some(f) = encode(&room.id, o) else { continue };
        match sfu.peer(&o.to) {
            Some(p) => p.dc_push(f),
            None => {
                sfu.drops.dc_no_room.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    for fx in &st.fx {
        match fx {
            Fx::Bind(u) => bind(sfu, room, u, st.from == G::Idle).await,
            Fx::Release(_) => release(sfu, room).await,
        }
    }
}

fn key(episode: u64) -> u64 {
    (episode << 33) | (1 << 32)
}

async fn bind(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str, prime: bool) {
    let Some(rm) = sfu.plane.room(&room.id) else { return };
    let now = now_ms();
    let mut episodes = [0u64; 2];
    for (i, kind) in [Kind::Audio, Kind::Video].into_iter().enumerate() {
        let slot = rm.slot(kind);
        let e = slot.episode.fetch_add(1, Ordering::AcqRel) + 1;
        episodes[i] = e;
        slot.bound.store(Some(Arc::new(Bound { user: user.to_string(), episode: e, at_ms: now, primed: Default::default() })));
        slot.switch_at.store(now, Ordering::Release);
        if kind == Kind::Video {
            for w in slot.subs.load().iter() {
                if let Some(e) = w.upgrade() {
                    e.rtx_block.store(true, Ordering::Release);
                }
            }
        }
    }
    if let Some(p) = sfu.peer(user) {
        let vids: Vec<_> = p.streams().iter().filter(|s| s.half() && s.kind == Kind::Video).flat_map(|s| p.ingress_of_track(&s.track_id)).collect();
        if !vids.is_empty() {
            let s2 = sfu.clone();
            let rm2 = rm.clone();
            let ep = episodes[1];
            tokio::spawn(async move {
                let mut last = 0;
                for at in PLI_BURST {
                    tokio::time::sleep(Duration::from_millis(at - last)).await;
                    last = at;
                    if rm2.video.bound.load().as_ref().is_none_or(|b| b.episode != ep) {
                        break;
                    }
                    for i in &vids {
                        crate::rtcp::shell::pli(&s2, i, now_ms(), true);
                    }
                }
            });
        }
    }
    if prime && rm.audio_codec.eq_ignore_ascii_case("opus") {
        let s2 = sfu.clone();
        let rm2 = rm.clone();
        let ep = episodes[0];
        let speaker = user.to_string();
        tokio::spawn(async move {
            for k in 0..PRIME_FRAMES {
                let cur = rm2.audio.bound.load_full();
                let Some(b) = cur.filter(|b| b.episode == ep) else { break };
                if b.primed.load(Ordering::Acquire) {
                    break;
                }
                let x =
                    Synth { kind: Kind::Audio, key: key(ep), seq: (k & 0xffff) as u16, ts: k.wrapping_mul(960), payload: &OPUS_SILENCE, skip: Some(&speaker) };
                crate::fwd::shell::send_synth(&s2, &rm2, &x);
                tokio::time::sleep(Duration::from_millis(PRIME_MS)).await;
            }
        });
    }
}

async fn release(sfu: &Arc<Sfu>, room: &Arc<Room>) {
    let Some(rm) = sfu.plane.room(&room.id) else { return };
    let now = now_ms();
    let mut ep_audio = 0;
    let prev = rm.audio.bound.load_full().map(|b| b.user.clone());
    for kind in [Kind::Audio, Kind::Video] {
        let slot = rm.slot(kind);
        slot.bound.store(None);
        let e = slot.episode.fetch_add(1, Ordering::AcqRel) + 1;
        slot.switch_at.store(now, Ordering::Release);
        if kind == Kind::Audio {
            ep_audio = e;
        } else {
            for w in slot.subs.load().iter() {
                if let Some(x) = w.upgrade() {
                    x.rtx_block.store(true, Ordering::Release);
                }
            }
        }
    }
    if rm.audio_codec.eq_ignore_ascii_case("opus") {
        let s2 = sfu.clone();
        tokio::spawn(async move {
            for k in 0..RELEASE_SILENCE {
                if rm.audio.episode.load(Ordering::Acquire) != ep_audio {
                    break;
                }
                let x = Synth { kind: Kind::Audio, key: key(ep_audio), seq: k as u16, ts: k * 960, payload: &OPUS_SILENCE, skip: prev.as_deref() };
                crate::fwd::shell::send_synth(&s2, &rm, &x);
                tokio::time::sleep(Duration::from_millis(PRIME_MS)).await;
            }
        });
    }
}

pub async fn dc_loop(sfu: Arc<Sfu>, peer: Arc<Peer>) {
    let link = peer.pub_conn().link.clone();
    let Some(mut rx) = link.dc_in.lock().ok().and_then(|mut g| g.take()) else { return };
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            _ = link.cancel.cancelled() => break,
            _ = tick.tick() => peer.dc_flush(),
            m = rx.recv() => {
                let Some(m) = m else { break };
                peer.dc_flush();
                on_dc(&sfu, &peer, &m).await;
            }
        }
    }
}

async fn on_dc(sfu: &Arc<Sfu>, peer: &Arc<Peer>, b: &[u8]) {
    let Some((svc, payload)) = dc_decode(b) else {
        sfu.drops.dc_bad_frame.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if svc != DC_SVC_FLOOR {
        sfu.drops.dc_unknown_svc.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Some(msg) = mbcp::decode(payload) else {
        sfu.drops.dc_bad_frame.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(room_id) = msg.room() else {
        sfu.drops.dc_no_room.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(m) = decode(&msg) else {
        sfu.drops.dc_bad_frame.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(room) = sfu.room(&room_id) else {
        if m.t == T::Request
            && let Some(f) = encode(&room_id, &Out { to: peer.user.clone(), m: M::new(T::Deny, None, u32::from(mbcp::deny::OTHER_REASON), 0) })
        {
            peer.dc_push(f);
        }
        return;
    };
    let step = room.floor.rx(&peer.user, &m, now_ms());
    exec(sfu, &room, step).await;
}
