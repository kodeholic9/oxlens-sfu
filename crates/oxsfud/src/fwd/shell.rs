// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:IMPL-FWD-ROOM · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-SUB-READY-GATE · 정:IMPL-SIM-SWITCH · 정:IMPL-PUB-ACCEPT · 정:RULE-CONC-SIX · 정:RULE-CONC-PERF-AXES · model: claude-opus-5-5
//! 전달 핫패스 — 업링크 스트림 찾기(시뮬캐스트는 rid 로 첫 RTP 에 붙인다) · 방은 매 패킷 `pub_room` · 반이중은 prefan 게이트(화자 = 발행자) · 줄 검사(확인된 차례) · 재기록 · 확장 다시 짓기 · 예열 CN · PLI burst.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use bytes::Bytes;
use oxsig::trk::MediaKind;

use super::core::{self as fc, Egress, Rtp};
use crate::link::table::Class;
use crate::link::{lock, Down, Peer, Transport};
use crate::r#pub::{PubStream, PubTrack};
use crate::sub::ext::{self as ext, ExtTable, ExtValues, InIds};

pub const PRIME_FRAMES: u32 = 250;
pub const PRIME_GAP_MS: u64 = 20;
pub const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const PLI_BURST_MS: [u64; 3] = [0, 500, 1500];
const PRIME_BASE: u64 = 0xffff_0000_0000_0000;
static PRIME_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn prime_key() -> u64 {
    PRIME_BASE | PRIME_SEQ.fetch_add(1, Ordering::Relaxed)
}
const GATE_MS: u64 = 5000;
const PROBE_PAD: usize = 224;

fn now_ms() -> u64 {
    common::clock::wall_ms()
}

fn in_ids(s: &PubStream) -> InIds {
    InIds { mid: s.ext.mid, rid: s.ext.rid, rrid: s.ext.rrid, audio_level: s.ext.audio_level, twcc: s.ext.twcc }
}

fn src_key(p: &PubTrack) -> u64 {
    p.ssrc as u64
}

fn learn(t: &Arc<Transport>, r: &Rtp<'_>) -> Option<(Arc<PubTrack>, bool)> {
    let peer = t.peer.upgrade()?;
    let pubs: Vec<Arc<PubStream>> = lock(&peer.pubs).iter().filter(|p| p.simulcast && !p.gone.load(Ordering::Acquire)).cloned().collect();
    let first = pubs.first()?;
    let px = ext::parse(r.profile, r.ext, &in_ids(first));
    let mid = px.mid.as_ref().map(|m| String::from_utf8_lossy(m).to_string());
    let s = pubs.iter().find(|p| Some(&p.mid) == mid.as_ref()).or(if pubs.len() == 1 { pubs.first() } else { None })?;
    if let Some(rr) = px.rrid {
        let rid = String::from_utf8_lossy(&rr).to_string();
        let phys = s.track_of_rid(&rid)?;
        phys.rtx_ssrc.store(r.ssrc as u64, Ordering::Release);
        t.rtx_in.insert(r.ssrc, phys.clone());
        return Some((phys, true));
    }
    let rid = String::from_utf8_lossy(&px.rid?).to_string();
    if let Some(p) = s.track_of_rid(&rid) {
        t.ingress.insert(r.ssrc, p.clone());
        return Some((p, false));
    }
    let p = Arc::new(PubTrack::new(r.ssrc, Some(rid.clone()), s));
    s.add_phys(p.clone());
    t.ingress.insert(r.ssrc, p.clone());
    tracing::info!(kind = "ev", spec = "SV-STATE-PUBLISH-STATE", user = %peer.user_id, track = %s.track_id, ssrc = r.ssrc, rid = %rid, "업링크 스트림 학습");
    Some((p, false))
}

pub fn ingress(t: &Arc<Transport>, pkt: Bytes) {
    let Some(r) = fc::parse(&pkt) else { return };
    let found = match t.ingress.get(&r.ssrc) {
        Some(p) => Some((p.clone(), false)),
        None => match t.rtx_in.get(&r.ssrc) {
            Some(p) => Some((p.clone(), true)),
            None => learn(t, &r),
        },
    };
    let Some((phys, rtx)) = found else {
        if let Some(c) = crate::cfg::get() {
            c.drops.unknown_ssrc.fetch_add(1, Ordering::Relaxed);
        }
        return;
    };
    let Some(stream) = phys.stream.upgrade() else { return };
    if rtx {
        if r.payload.len() < 2 {
            return;
        }
        let osn = u16::from_be_bytes([r.payload[0], r.payload[1]]);
        let mut orig = Vec::with_capacity(pkt.len());
        fc::build(&Egress { byte1: pkt[1], pt: stream.pt, seq: osn, ts: r.ts, ssrc: phys.ssrc, csrc: r.csrc, ext: ext_block(&pkt, &r), payload: &r.payload[2..], padding: &[] }, &mut orig);
        let orig = Bytes::from(orig);
        let Some(o) = fc::parse(&orig) else { return };
        lock(&phys.nack).on_packet(o.seq, now_ms());
        lock(&phys.cache).put(o.seq, orig.clone());
        dispatch(t, &phys, &stream, &o, true);
        return;
    }
    let now = now_ms();
    let fresh = lock(&phys.stats).update(r.seq, r.ts, now, phys.clock);
    if !fresh
        && let Some(c) = crate::cfg::get() {
            c.drops.probation.fetch_add(1, Ordering::Relaxed);
        }
    lock(&phys.nack).on_packet(r.seq, now);
    lock(&phys.cache).put(r.seq, pkt.clone());
    phys.packets.fetch_add(1, Ordering::Relaxed);
    phys.bytes.fetch_add(pkt.len() as u64, Ordering::Relaxed);
    phys.last_rtp_ms.store(now, Ordering::Release);
    let px = ext::parse(r.profile, r.ext, &in_ids(&stream));
    if let Some(seq) = px.twcc {
        t.twcc_id.store(stream.ext.twcc.unwrap_or(0), Ordering::Release);
        lock(&t.twcc_in).record(seq, common::clock::wall_us());
    }
    if stream.kind == MediaKind::Audio
        && let Some(l) = px.audio_level
            && (l & 0x7f) < 50 {
                stream.last_talk_ms.store(now, Ordering::Release);
            }
    if r.pt != stream.pt && Some(r.pt) != stream.rtx_pt && !stream.pt_warned.swap(true, Ordering::AcqRel) {
        tracing::warn!(kind = "diag", user = %stream.user_id, track = %stream.track_id, declared = stream.pt, actual = r.pt, "[PT:MISMATCH]");
    }
    if stream.first_rtp_at.compare_exchange(0, now, Ordering::AcqRel, Ordering::Acquire).is_ok() {
        crate::ops::first_rtp(&stream);
    }
    dispatch(t, &phys, &stream, &r, false);
}

fn ext_block<'a>(pkt: &'a [u8], r: &Rtp<'a>) -> &'a [u8] {
    if r.ext.is_empty() {
        return &[];
    }
    let start = 12 + r.csrc.len();
    &pkt[start..start + 4 + r.ext.len()]
}

fn dispatch(t: &Arc<Transport>, phys: &Arc<PubTrack>, stream: &Arc<PubStream>, r: &Rtp<'_>, rtx: bool) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(peer) = t.peer.upgrade() else { return };
    let Some(room_id) = peer.pub_room() else {
        ctx.drops.no_pub_room.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(room) = ctx.rooms.get(&room_id) else { return };
    let codec = stream.codec.clone().unwrap_or_else(|| if stream.kind == MediaKind::Audio { room.media.audio().to_string() } else { room.media.video().to_string() });
    let key = stream.kind == MediaKind::Video && fc::keyframe(&codec, r.payload);
    if key {
        phys.last_key_ms.store(now_ms(), Ordering::Release);
    }
    let al = if stream.kind == MediaKind::Audio { ext::parse(r.profile, r.ext, &in_ids(stream)).audio_level } else { None };
    let src = Src { key: src_key(phys), phys: Some(phys), ssrc: phys.ssrc, user: &stream.user_id, track: &stream.track_id, clock: phys.clock, audio_level: al };
    if stream.half.load(Ordering::Acquire) {
        let holder = room.floor.holder();
        if holder.as_deref() != Some(peer.user_id.as_str()) {
            if !rtx {
                room.floor.on_rtp(&peer.user_id, false);
            }
            ctx.drops.floor_gate.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if !rtx {
            room.floor.on_rtp(&peer.user_id, true);
        }
        let slot = room.slot(stream.kind);
        if stream.kind == MediaKind::Audio {
            slot.priming.store(0, Ordering::Release);
        }
        if slot.speaker().as_deref() != Some(peer.user_id.as_str()) {
            ctx.drops.slot_unbound.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for w in slot.subs.load().iter() {
            let Some(d) = w.upgrade() else { continue };
            let Some(dp) = d.peer.upgrade() else { continue };
            if dp.user_id == peer.user_id || !sendable(&d, &dp) {
                continue;
            }
            if !send_down(&d, &dp, &src, r, key) && stream.kind == MediaKind::Video {
                crate::rtcp::shell::pli(phys, false);
            }
        }
        return;
    }
    if stream.room_id() != room_id {
        ctx.drops.room_moved.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let now = now_ms();
    let layer = phys.layer.load(Ordering::Acquire);
    for w in stream.subs.load().iter() {
        let Some(d) = w.upgrade() else { continue };
        let Some(dp) = d.peer.upgrade() else { continue };
        if !sendable(&d, &dp) || d.loaded().map(|l| l.track_id != stream.track_id).unwrap_or(true) {
            continue;
        }
        if stream.kind == MediaKind::Video {
            if let Some(at) = dp.gate_closed.load().get(&room_id).copied() {
                if now.saturating_sub(at) < GATE_MS {
                    ctx.drops.gate.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                crate::sub::gate_timeout(&dp, &room_id);
            }
            if stream.simulcast {
                let ok = lock(&d.st).sel.accept(layer, key);
                if !ok {
                    continue;
                }
            }
        }
        if !send_down(&d, &dp, &src, r, key) && stream.kind == MediaKind::Video {
            crate::rtcp::shell::pli(phys, false);
        }
    }
}

pub struct Src<'a> {
    pub key: u64,
    pub phys: Option<&'a Arc<PubTrack>>,
    pub ssrc: u32,
    pub user: &'a str,
    pub track: &'a str,
    pub clock: u32,
    pub audio_level: Option<u8>,
}

pub fn sendable(d: &Down, p: &Peer) -> bool {
    !p.reaped.load(Ordering::Acquire) && d.loaded().is_some() && d.since.load(Ordering::Acquire) <= p.ack.load(Ordering::Acquire)
}

pub fn ext_table(p: &Peer) -> ExtTable {
    ExtTable { mid: p.ext.mid, audio_level: p.ext.audio_level, abs_send_time: p.ext.abs_send_time, twcc: p.ext.twcc }
}

pub fn send_down(d: &Arc<Down>, p: &Peer, src: &Src<'_>, r: &Rtp<'_>, key: bool) -> bool {
    let now = now_ms();
    let (seq, ts) = {
        let mut st = lock(&d.st);
        let changed = st.src_ssrc != src.ssrc;
        if changed && d.kind == MediaKind::Video && !key && src.key < PRIME_BASE {
            return false;
        }
        let m = st.rw.map(src.key, r.seq, r.ts, src.clock, now);
        if changed {
            st.nack_closed = false;
            st.src_ssrc = src.ssrc;
            st.src_user = Some(src.user.to_string());
            st.src_track = Some(src.track.to_string());
            st.src_phys = src.phys.map(Arc::downgrade);
        }
        (m.seq, m.ts)
    };
    let t = p.sub_transport();
    let table = ext_table(p);
    let use_twcc = table.twcc.is_some();
    let twcc_seq = if use_twcc { t.twcc_out.fetch_add(1, Ordering::AcqRel) } else { 0 };
    let mid = d.mid.to_string();
    let vals = ExtValues { mid: Some(&mid), audio_level: if d.kind == MediaKind::Audio { src.audio_level } else { None }, t_us: common::clock::wall_us(), twcc_seq };
    let tb = if d.kind == MediaKind::Video { ExtTable { audio_level: None, ..table } } else { table };
    let mut eb = Vec::with_capacity(32);
    ext::build(&tb, tb.form(), &vals, &mut eb);
    let mut out = Vec::with_capacity(r.payload.len() + 64);
    let pt = d.pt.load(Ordering::Acquire);
    let byte1 = if src.key >= PRIME_BASE { 0 } else { r.marker as u8 * 0x80 };
    fc::build(&Egress { byte1, pt, seq, ts, ssrc: d.ssrc, csrc: r.csrc, ext: &eb, payload: r.payload, padding: r.padding }, &mut out);
    let ok = t.send_rtp(&out);
    if ok {
        d.sent.fetch_add(1, Ordering::Relaxed);
        d.bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
        d.last_at.store(now, Ordering::Release);
        if use_twcc {
            lock(&t.sim).sent_packet(twcc_seq, common::clock::wall_us(), out.len());
        }
    }
    true
}

pub fn probe_send(p: &Arc<Peer>, bytes: usize) -> Vec<(u16, usize)> {
    let mut out = Vec::new();
    let t = p.sub_transport();
    let downs: Vec<Arc<Down>> = lock(&p.downs).values().filter(|d| d.kind == MediaKind::Video && d.class == Class::Full && d.rtx_ssrc.is_some() && d.loaded().is_some()).cloned().collect();
    let Some(d) = downs.into_iter().find(|d| sendable(d, p) && lock(&d.st).rw.last.is_some()) else { return out };
    let (Some(rs), rpt) = (d.rtx_ssrc, d.rtx_pt.load(Ordering::Acquire)) else { return out };
    let table = ext_table(p);
    let tb = ExtTable { audio_level: None, ..table };
    let Some(_) = tb.twcc else { return out };
    let mid = d.mid.to_string();
    let mut sent = 0;
    while sent < bytes {
        let (seq, ts, osn) = {
            let mut st = lock(&d.st);
            st.rtx_seq = st.rtx_seq.wrapping_add(1);
            let l = st.rw.last.unwrap_or(crate::fwd::core::Last { seq: 0, ts: 0, at_ms: 0 });
            (st.rtx_seq, l.ts, l.seq)
        };
        let twcc_seq = t.twcc_out.fetch_add(1, Ordering::AcqRel);
        let mut eb = Vec::with_capacity(24);
        ext::build(&tb, tb.form(), &ExtValues { mid: Some(&mid), audio_level: None, t_us: common::clock::wall_us(), twcc_seq }, &mut eb);
        let mut pad = vec![0u8; PROBE_PAD - 1];
        pad.push(PROBE_PAD as u8);
        let mut pkt = Vec::with_capacity(PROBE_PAD + 40);
        fc::build(&Egress { byte1: 0, pt: rpt, seq, ts, ssrc: rs, csrc: &[], ext: &eb, payload: &osn.to_be_bytes(), padding: &pad }, &mut pkt);
        if !t.send_rtp(&pkt) {
            break;
        }
        lock(&t.sim).sent_packet(twcc_seq, common::clock::wall_us(), pkt.len());
        out.push((twcc_seq, pkt.len()));
        sent += pkt.len();
    }
    out
}

pub fn on_connected(t: &Arc<Transport>) {
    let Some(p) = t.peer.upgrade() else { return };
    if t.role == crate::link::Role::Publish {
        return;
    }
    let downs: Vec<Arc<Down>> = lock(&p.downs).values().cloned().collect();
    for d in downs.iter().filter(|d| d.kind == MediaKind::Video) {
        key_for(d);
    }
}

pub fn key_for(d: &Arc<Down>) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(l) = d.loaded() else { return };
    if d.class == Class::Slot {
        let Some(room) = ctx.rooms.get(&l.room_id) else { return };
        if let Some(u) = room.video.speaker() {
            pli_user(&u, true);
        }
        return;
    }
    for p in ctx.links.all() {
        if let Some(s) = p.pub_by_id(&l.track_id) {
            for ph in s.phys.load().iter() {
                crate::rtcp::shell::pli(ph, true);
            }
        }
    }
}

pub fn pli_user(user: &str, infra: bool) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(p) = ctx.links.peer(user) else { return };
    let pubs: Vec<Arc<PubStream>> = lock(&p.pubs).iter().filter(|s| s.kind == MediaKind::Video && s.half.load(Ordering::Acquire)).cloned().collect();
    for s in pubs {
        for ph in s.phys.load().iter() {
            crate::rtcp::shell::pli(ph, infra);
        }
    }
}

fn video_codec_ok(user: &str, room_video: &str) -> bool {
    let Some(ctx) = crate::cfg::get() else { return false };
    let Some(p) = ctx.links.peer(user) else { return false };
    let g = lock(&p.pubs);
    let v: Vec<&Arc<PubStream>> = g.iter().filter(|s| s.kind == MediaKind::Video && s.half.load(Ordering::Acquire)).collect();
    v.is_empty() || v.iter().any(|s| s.codec.as_deref().map(|c| c.eq_ignore_ascii_case(room_video)).unwrap_or(false))
}

fn close_nack(slot: &crate::room::Slot) {
    for w in slot.subs.load().iter() {
        if let Some(d) = w.upgrade() {
            lock(&d.st).nack_closed = true;
        }
    }
}

pub fn floor_bind(room_id: &str, user: &str, prime: bool) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(room) = ctx.rooms.get(room_id) else { return };
    close_nack(&room.video);
    let now = now_ms();
    room.audio.speaker.store(Some(Arc::new(user.to_string())));
    room.audio.bound_at.store(now, Ordering::Release);
    if video_codec_ok(user, room.media.video()) {
        room.video.speaker.store(Some(Arc::new(user.to_string())));
        room.video.bound_at.store(now, Ordering::Release);
    } else {
        room.video.speaker.store(None);
        tracing::warn!(kind = "diag", room = %room_id, user = %user, "video 슬롯 미결합 — 코덱 불일치");
    }
    tracing::info!(kind = "ev", spec = "SV-STATE-FLOOR-BIND", room = %room_id, user = %user, epoch = %ctx.boot.epoch, prime, "슬롯 결합");
    if prime && room.media.audio().eq_ignore_ascii_case("opus") {
        let token = common::ident::random_u64() | 1;
        room.audio.priming.store(token, Ordering::Release);
        let r = Arc::downgrade(&room);
        let u = user.to_string();
        tokio::spawn(async move { prime_run(r, u, token).await });
    }
    let r = Arc::downgrade(&room);
    let u = user.to_string();
    tokio::spawn(async move {
        let mut at = 0;
        for ms in PLI_BURST_MS {
            tokio::time::sleep(Duration::from_millis(ms - at)).await;
            at = ms;
            let Some(room) = r.upgrade() else { return };
            if room.video.speaker().as_deref() != Some(u.as_str()) {
                return;
            }
            pli_user(&u, true);
        }
    });
}

async fn prime_run(room: Weak<crate::room::Room>, speaker: String, token: u64) {
    let mut seq: u16 = (common::ident::random_u32() & 0x7fff) as u16;
    let mut ts: u32 = common::ident::random_u32();
    let mut iv = tokio::time::interval(Duration::from_millis(PRIME_GAP_MS));
    let key = prime_key();
    for _ in 0..PRIME_FRAMES {
        iv.tick().await;
        let Some(r) = room.upgrade() else { return };
        if r.audio.priming.load(Ordering::Acquire) != token {
            return;
        }
        silence(&r, &speaker, key, seq, ts);
        seq = seq.wrapping_add(1);
        ts = ts.wrapping_add(960);
    }
}

fn silence(r: &crate::room::Room, skip: &str, key: u64, seq: u16, ts: u32) {
    let pkt = Rtp { marker: false, pt: 111, seq, ts, ssrc: 0, csrc: &[], profile: 0, ext: &[], payload: &OPUS_SILENCE, padding: &[] };
    let src = Src { key, phys: None, ssrc: (key & 0xffff_ffff) as u32, user: "", track: "", clock: 48_000, audio_level: None };
    for w in r.audio.subs.load().iter() {
        let Some(d) = w.upgrade() else { continue };
        let Some(dp) = d.peer.upgrade() else { continue };
        if dp.user_id == skip || !sendable(&d, &dp) {
            continue;
        }
        send_down(&d, &dp, &src, &pkt, false);
    }
}

pub fn floor_release(room_id: &str, prev: Option<&str>) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(room) = ctx.rooms.get(room_id) else { return };
    close_nack(&room.video);
    room.audio.priming.store(0, Ordering::Release);
    room.audio.speaker.store(None);
    room.video.speaker.store(None);
    tracing::info!(kind = "ev", spec = "SV-STATE-FLOOR-BIND", room = %room_id, epoch = %ctx.boot.epoch, prev = prev.unwrap_or(""), "슬롯 해제");
    if room.media.audio().eq_ignore_ascii_case("opus") {
        let mut seq: u16 = (common::ident::random_u32() & 0x7fff) as u16;
        let mut ts: u32 = common::ident::random_u32();
        let key = prime_key();
        for _ in 0..3 {
            silence(&room, prev.unwrap_or(""), key, seq, ts);
            seq = seq.wrapping_add(1);
            ts = ts.wrapping_add(960);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_prime_burst_is_its_own_source() {
        let (a, b) = (prime_key(), prime_key());
        assert!(a >= PRIME_BASE && b >= PRIME_BASE && a != b, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 예열 · 해제 무음은 매번 새 출처 — 같은 키면 새 seq 시작이 구멍이 된다");
    }
}
