// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-FIRST-NOTICE · 정:CAND-FWD-PRIME-SOURCE · 정:CAND-FWD-PRIME-EPISODE · 정:CAND-FLR-RELEASE-SILENCE-SPEAKER · 정:CAND-RTCP-RTX-GATE-WINDOW · 정:IMPL-AUX-DC-SEND · 정:RULE-CONC-SIX · 연:RULE-FLR-TRANSPORT-DC · 연:RULE-FLR-MSG-ACK · 연:RULE-FLR-ROOM-KEY · 연:TRN.frame.DC · 운영:RULE-LOG-TRANSITION · 연:RULE-FLR-SLOT-SPEAKER · 연:RULE-FLR-PREEMPT · 연:DEV-FLR-ROOM-GONE · model: claude-opus-5-5
//! 발언권 집행 — 판정(락 안)이 돌려준 보낼 것 · 효과를 락 밖에서 집행한다: DC 송신 · 미디어 결합(번호 규칙 · 예열 CN · 해제 무음 · PLI burst · RTX 게이트) · 방마다 타이머 구동(가장 이른 만료에 깨어난다) · RTP 관측 알림(사람마다 500ms).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::types::{Duplex, Kind};
use tokio::sync::{Notify, mpsc};

use super::{Sfu, now_ms};
use crate::flr::core::G;
use crate::flr::{Effect, Stepped};
use crate::fwd::hot::{Bound, Downlink, FloorIn, RoomHot};
use crate::link::Transport;
use crate::ops::drops::{DROPS, bump};
use crate::r#pub::Stream;

pub const PRIME: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const PRIME_MAX: u32 = 250;
pub const SILENCE_FRAMES: u32 = 3;
pub const FRAME_MS: u64 = 20;
pub const PLI_BURST: [u64; 3] = [0, 500, 1_500];

pub struct FloorRun {
    pub room: String,
    pub hot: Arc<RoomHot>,
    pub s: Stepped,
}

#[derive(Default)]
pub struct Exec {
    pub floors: Vec<FloorRun>,
    pub pli: Vec<Arc<Downlink>>,
    pub layer_pli: Vec<Arc<Downlink>>,
    pub burst: Vec<Arc<Stream>>,
    pub close: Vec<Arc<Transport>>,
}

pub fn log_step(room: &str, entry: &str, user: Option<&str>, s: &Stepped) {
    match s.r.labels.first() {
        Some(l) => common::ev!(spec = *l, area = "FLR", key = %room, from = s.from.as_str(), to = s.to.as_str(), cause = %entry, room = %room, in.u = user.unwrap_or("-")),
        None => common::ev!(area = "FLR", key = %room, cause = %entry, impossible = 1, room = %room, in.u = user.unwrap_or("-")),
    }
}

impl Exec {
    pub fn floor(&mut self, room: &str, hot: Arc<RoomHot>, entry: &str, user: Option<&str>, s: Stepped) {
        log_step(room, entry, user, &s);
        self.floors.push(FloorRun { room: room.to_string(), hot, s });
    }

    pub async fn run(self, sfu: &Arc<Sfu>) {
        let now = now_ms();
        for f in self.floors {
            let mut targets: Vec<(Arc<Transport>, Vec<u8>)> = Vec::new();
            let mut speaker_video: Vec<Arc<Stream>> = Vec::new();
            if let Ok(st) = sfu.state.lock() {
                for o in &f.s.r.out {
                    let Some(p) = st.peers.get(&o.to) else { continue };
                    let Some(m) = crate::flr::encode(&o.m, &f.room) else { continue };
                    let Some(frame) = oxsig::dc::encode(oxsig::dc::SVC_FLOOR, &m) else { continue };
                    targets.push((p.publish.clone(), frame));
                }
                for e in &f.s.effects {
                    if let Effect::Bind(u, _) = e
                        && let Some(p) = st.peers.get(u)
                    {
                        speaker_video.extend(p.streams.values().filter(|s| s.kind == Kind::Video && s.duplex() == Duplex::Half).cloned());
                    }
                }
            }
            for (t, frame) in targets {
                t.dc_send(frame);
            }
            for e in &f.s.effects {
                media(&f.hot, e, f.s.from, &speaker_video, now);
            }
            wake(sfu, &f.room);
        }
        for dl in self.pli {
            crate::fwd::egress::forward_pli(&dl, now, true);
        }
        for dl in self.layer_pli {
            crate::fwd::egress::target_layer_pli(&dl, now);
        }
        for s in self.burst {
            crate::rtcp::keyframe_burst(&s, &[0]);
        }
        for t in self.close {
            t.close();
        }
    }
}

fn media(hot: &Arc<RoomHot>, e: &Effect, from: G, speaker_video: &[Arc<Stream>], now: u64) {
    match e {
        Effect::Bind(u, g) => {
            let mut bound_any = false;
            for kind in [Kind::Audio, Kind::Video] {
                let slot = hot.slot(kind);
                let applied = slot.bind.lock().map(|mut b| b.apply(&crate::flr::bind::Effect::Bind(u.clone(), *g))).unwrap_or(false);
                if !applied {
                    continue;
                }
                bound_any = true;
                slot.bound.store(Some(Arc::new(Bound { user: Arc::from(u.as_str()), g: *g, at_ms: now })));
                slot.speaker_first_ms.store(0, Ordering::Release);
                slot.speaker_last_ms.store(0, Ordering::Release);
                if kind == Kind::Video {
                    for dl in slot.live() {
                        dl.rtx_gate.store(true, Ordering::Release);
                    }
                }
                if kind == Kind::Audio && from == G::Idle && hot.audio_codec.eq_ignore_ascii_case("opus") {
                    prime(hot.clone(), u.clone(), *g);
                }
            }
            if bound_any {
                for s in speaker_video {
                    crate::rtcp::keyframe_burst(s, &PLI_BURST);
                }
            }
        }
        Effect::Release(g, prev) => {
            for kind in [Kind::Audio, Kind::Video] {
                let slot = hot.slot(kind);
                let applied = slot.bind.lock().map(|mut b| b.apply(&crate::flr::bind::Effect::Release(*g))).unwrap_or(false);
                if !applied {
                    continue;
                }
                slot.bound.store(None);
                slot.priming.store(false, Ordering::Release);
                match kind {
                    Kind::Audio => silence(hot.clone(), prev.clone()),
                    Kind::Video => {
                        for dl in slot.live() {
                            dl.rtx_gate.store(true, Ordering::Release);
                        }
                    }
                }
            }
        }
    }
}

fn synth_round(hot: &RoomHot, skip: Option<&str>, key: u64, seq: u16, ts: u32, now: u64) {
    for dl in hot.audio.live() {
        if skip.is_some_and(|u| *dl.user == *u) || !dl.sendable.load(Ordering::Acquire) {
            continue;
        }
        crate::fwd::egress::synthetic(&dl, key, seq, ts, &PRIME, now);
    }
}

fn prime(hot: Arc<RoomHot>, speaker: String, g: u64) {
    let key = hot.audio.new_episode();
    hot.audio.priming.store(true, Ordering::Release);
    tokio::spawn(async move {
        let mut seq: u16 = 0;
        let mut ts: u32 = 0;
        for _ in 0..PRIME_MAX {
            let still = hot.audio.bound.load().as_ref().is_some_and(|b| b.g == g);
            if !hot.audio.priming.load(Ordering::Acquire) || !still {
                break;
            }
            synth_round(&hot, Some(&speaker), key, seq, ts, now_ms());
            seq = seq.wrapping_add(1);
            ts = ts.wrapping_add(960);
            tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
        }
        hot.audio.priming.store(false, Ordering::Release);
    });
}

fn silence(hot: Arc<RoomHot>, prev: Option<String>) {
    let key = hot.audio.new_episode();
    tokio::spawn(async move {
        let mut ts: u32 = 0;
        for seq in 0..SILENCE_FRAMES as u16 {
            if hot.audio.bound.load().is_some() {
                break;
            }
            synth_round(&hot, prev.as_deref(), key, seq, ts, now_ms());
            ts = ts.wrapping_add(960);
            tokio::time::sleep(Duration::from_millis(FRAME_MS)).await;
        }
    });
}

pub fn wake(sfu: &Sfu, room: &str) {
    if let Ok(m) = sfu.floor_wake.lock()
        && let Some(n) = m.get(room)
    {
        n.notify_one();
    }
}

pub fn spawn_driver(sfu: &Arc<Sfu>, room: &str, mut rx: mpsc::UnboundedReceiver<FloorIn>) {
    let notify = Arc::new(Notify::new());
    if let Ok(mut m) = sfu.floor_wake.lock() {
        m.insert(room.to_string(), notify.clone());
    }
    let sfu = sfu.clone();
    let room = room.to_string();
    tokio::spawn(async move {
        let base = common::time::start();
        loop {
            let next = match sfu.state.lock() {
                Ok(st) => match st.rooms.get(&room) {
                    Some(r) => r.floor.next_deadline(),
                    None => break,
                },
                Err(_) => break,
            };
            let sleep = async {
                match next {
                    Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(base + Duration::from_millis(d))).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let mut exec = Exec::default();
            tokio::select! {
                _ = sleep => fire(&sfu, &room, &mut exec),
                _ = notify.notified() => {}
                ev = rx.recv() => match ev {
                    Some(FloorIn::Rtp(u)) => rtp(&sfu, &room, &u, &mut exec),
                    None => break,
                },
            }
            exec.run(&sfu).await;
        }
        if let Ok(mut m) = sfu.floor_wake.lock() {
            m.remove(&room);
        }
    });
}

fn fire(sfu: &Sfu, room: &str, exec: &mut Exec) {
    let now = now_ms();
    let Ok(mut st) = sfu.state.lock() else { return };
    let Some(r) = st.rooms.get_mut(room) else { return };
    let last = r.hot.audio.speaker_last_ms.load(Ordering::Acquire).max(r.hot.video.speaker_last_ms.load(Ordering::Acquire));
    for k in r.floor.due(now) {
        if let Some(s) = r.floor.fire(&k, now, last) {
            let (entry, user) = match &k {
                crate::flr::Key::G(t) => (format!("{t:?}"), None),
                crate::flr::Key::P(u, t) => (format!("{t:?}"), Some(u.clone())),
            };
            exec.floor(room, r.hot.clone(), &entry, user.as_deref(), s);
        }
    }
}

fn rtp(sfu: &Sfu, room: &str, user: &str, exec: &mut Exec) {
    let now = now_ms();
    let Ok(mut st) = sfu.state.lock() else { return };
    let Some(r) = st.rooms.get_mut(room) else { return };
    if !r.members.contains_key(user) {
        return;
    }
    let s = r.floor.step(now, |s, e| crate::flr::core::rtp(s, e, user));
    exec.floor(room, r.hot.clone(), "rtp", Some(user), s);
}

pub async fn on_dc_frame(sfu: &Arc<Sfu>, t: &Arc<Transport>, b: &[u8]) {
    let Some(f) = oxsig::dc::decode(b) else {
        bump(&DROPS.dc_bad_frame);
        return;
    };
    if f.svc != oxsig::dc::SVC_FLOOR {
        bump(&DROPS.dc_unknown_svc);
        return;
    }
    let Some(m) = oxsig::mbcp::decode(f.payload) else {
        bump(&DROPS.dc_bad_frame);
        return;
    };
    let Some((msg, room)) = crate::flr::decode(&m) else {
        bump(&DROPS.dc_bad_frame);
        return;
    };
    let Some(room) = room else {
        bump(&DROPS.dc_no_room);
        return;
    };
    let user = t.user.to_string();
    if m.ack && oxsig::mbcp::ack_allowed(m.msg_type) {
        let ack = crate::flr::core::Msg { t: crate::flr::core::T::Ack, a: None, n: u32::from(m.msg_type), d: 0, q: m.seq().unwrap_or(0) };
        if let Some(bytes) = crate::flr::encode(&ack, &room)
            && let Some(frame) = oxsig::dc::encode(oxsig::dc::SVC_FLOOR, &bytes)
        {
            t.dc_send(frame);
        }
    }
    let mut exec = Exec::default();
    {
        let now = now_ms();
        let Ok(mut st) = sfu.state.lock() else { return };
        let Some(r) = st.rooms.get_mut(&room) else {
            bump(&DROPS.dc_no_room);
            return;
        };
        if !r.members.contains_key(&user) {
            bump(&DROPS.dc_no_room);
        }
        let s = r.floor.step(now, |s, e| crate::flr::core::server_rx(s, e, &user, &msg));
        exec.floor(&room, r.hot.clone(), msg.t.name(), Some(&user), s);
    }
    exec.run(sfu).await;
}
