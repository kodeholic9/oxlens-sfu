// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-POLICY · 정:IMPL-FLR-FIRST-NOTICE · 정:RULE-FLR-SNAPSHOT · 연:FLR.wire · 연:RULE-FLR-HEADER · 연:RULE-FLR-TIMERS · 연:ASM-FLR-RTP-INSTANT · 연:ASM-FLR-T20-OVER-RTP · model: claude-opus-5-5
//! 영역 FLR — 방마다 한 락이 판정 상태 · 참가자 상태 · 큐 · 만료 시각을 지킨다. 판정은 락 안, 송신 · 미디어 결합은 부르는 쪽이 락 밖에서. 타이머는 가장 이른 만료 시각에 깨어난다.

pub mod core;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use oxsig::mbcp::{self, tlv};
use serde_json::{json, Value};

use self::core::{armed_participant, armed_server, Cfg, Fx, Msg, Out, Srv, Step, Tm, Tp, G, T};

pub const T1_MS: u64 = 4_000;
pub const T3_MS: u64 = 3_000;
pub const T7_MS: u64 = 1_000;
pub const T8_MS: u64 = 1_000;
pub const T9_MS: u64 = 3_000;
pub const T20_MS: u64 = 1_000;
pub const T21_MS: u64 = 1_000;
pub const RTP_NOTE_MS: u64 = 500;

type Due = (Option<Tm>, Option<(String, Tp)>);

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub outs: Vec<Out>,
    pub fx: Vec<Fx>,
    pub labels: Vec<&'static str>,
}

#[derive(Debug, Default)]
struct Inner {
    srv: Srv,
    server_due: BTreeMap<Tm, u64>,
    part_due: BTreeMap<(String, Tp), u64>,
    t1_start: u64,
    first_rtp_at: u64,
    granted_at: u64,
    holder_since: u64,
    started_at_wall: u64,
    c7: u32,
    c20: u32,
    revoking: Option<String>,
    marks: BTreeMap<String, (u32, u32, u16, bool)>,
}

pub struct Floor {
    pub room: String,
    qmax: usize,
    t2_ms: u64,
    inner: Mutex<Inner>,
    pub last_rtp_ms: AtomicU64,
    last_note_ms: AtomicU64,
    pub wake: tokio::sync::Notify,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Floor {
    pub fn new(room: &str, qmax: u8, t2_secs: u16) -> Floor {
        Floor {
            room: room.to_string(),
            qmax: qmax as usize,
            t2_ms: (t2_secs.max(1) as u64) * 1000,
            inner: Mutex::new(Inner::default()),
            last_rtp_ms: AtomicU64::new(0),
            last_note_ms: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
        }
    }

    fn cfg(&self, i: &Inner, now: u64) -> Cfg {
        let t2s = (self.t2_ms / 1000) as u32;
        let remain = if i.first_rtp_at > 0 { (i.first_rtp_at + self.t2_ms).saturating_sub(now).div_ceil(1000) as u32 } else { t2s };
        Cfg { qmax: self.qmax, t2_secs: t2s, t2_remain_secs: remain }
    }

    fn run(&self, now: u64, f: impl FnOnce(&mut Step<'_>)) -> Outcome {
        let mut g = lock(&self.inner);
        let before_holder = Step::holder(&g.srv);
        let before_g = g.srv.g;
        let cfg = self.cfg(&g, now);
        let r = {
            let srv: &mut Srv = &mut g.srv;
            let mut st = Step::new(srv, cfg);
            f(&mut st);
            st.r
        };
        let after_holder = Step::holder(&g.srv);
        if g.srv.g != before_g && matches!(g.srv.g, G::Taken | G::Prevoke) && !matches!(before_g, G::Taken | G::Prevoke) {
            g.t1_start = now;
        }
        if after_holder != before_holder {
            g.t1_start = now;
            g.first_rtp_at = 0;
            if g.srv.g == G::Taken {
                g.granted_at = now;
                g.holder_since = common::time::wall_ms();
            }
            self.last_rtp_ms.store(0, Ordering::Release);
            self.last_note_ms.store(0, Ordering::Release);
        }
        if g.srv.first_rtp && g.first_rtp_at == 0 {
            g.first_rtp_at = now;
        }
        if !g.srv.first_rtp {
            g.first_rtp_at = 0;
        }
        self.rearm(&mut g, now, false);
        drop(g);
        self.wake.notify_one();
        Outcome { outs: r.o, fx: r.fx, labels: r.l }
    }

    fn rearm(&self, g: &mut Inner, now: u64, _fired: bool) {
        let armed: Vec<Tm> = armed_server(&g.srv);
        g.server_due.retain(|k, _| armed.contains(k));
        let revoking_changed = g.revoking != g.srv.revoking;
        for t in armed {
            let restart = match t {
                Tm::T7 => g.c7 != g.srv.c7 && g.srv.c7 == 1,
                Tm::T20 => g.c20 != g.srv.c20 && g.srv.c20 == 1,
                Tm::T3 => revoking_changed,
                _ => false,
            };
            if restart || !g.server_due.contains_key(&t) {
                let d = match t {
                    Tm::T1 => T1_MS,
                    Tm::T2 => self.t2_ms,
                    Tm::T3 => T3_MS,
                    Tm::T7 => T7_MS,
                    Tm::T20 => T20_MS,
                };
                g.server_due.insert(t, now + d);
            }
        }
        g.c7 = g.srv.c7;
        g.c20 = g.srv.c20;
        g.revoking = g.srv.revoking.clone();
        let users: Vec<String> = g.srv.u.keys().cloned().collect();
        let mut keep: Vec<(String, Tp)> = Vec::new();
        for u in users {
            let p = g.srv.u.get(&u).cloned().unwrap_or_default();
            let mark = (p.c8, p.c21, p.tq, p.retry);
            let prev = g.marks.get(&u).copied();
            for t in armed_participant(&g.srv, &u) {
                keep.push((u.clone(), t));
                let restart = match (t, prev) {
                    (Tp::T8, Some(m)) => m.0 != p.c8 && p.c8 == 1,
                    (Tp::T21, Some(m)) => (m.1 != p.c21 && p.c21 == 1) || m.2 != p.tq,
                    (Tp::T9, Some(m)) => !m.3,
                    _ => false,
                };
                let key = (u.clone(), t);
                if restart || !g.part_due.contains_key(&key) {
                    let d = match t {
                        Tp::T8 => T8_MS,
                        Tp::T9 => T9_MS,
                        Tp::T21 => T21_MS,
                    };
                    g.part_due.insert(key, now + d);
                }
            }
            g.marks.insert(u, mark);
        }
        g.part_due.retain(|k, _| keep.contains(k));
    }

    fn t1_due(&self, g: &Inner) -> Option<u64> {
        g.server_due.get(&Tm::T1)?;
        let base = g.t1_start.max(self.last_rtp_ms.load(Ordering::Acquire));
        Some(base + T1_MS)
    }

    pub fn next_due(&self) -> Option<u64> {
        let g = lock(&self.inner);
        let mut best: Option<u64> = None;
        for (k, v) in &g.server_due {
            let d = if *k == Tm::T1 { self.t1_due(&g).unwrap_or(*v) } else if *k == Tm::T2 { g.first_rtp_at + self.t2_ms } else { *v };
            best = Some(best.map(|b| b.min(d)).unwrap_or(d));
        }
        for v in g.part_due.values() {
            best = Some(best.map(|b| b.min(*v)).unwrap_or(*v));
        }
        best
    }

    pub fn fire(&self, now: u64) -> Vec<Outcome> {
        let mut out = Vec::new();
        for _ in 0..64 {
            let due: Option<Due> = {
                let g = lock(&self.inner);
                let mut pick = None;
                for (k, v) in &g.server_due {
                    let d = if *k == Tm::T1 { self.t1_due(&g).unwrap_or(*v) } else if *k == Tm::T2 { g.first_rtp_at + self.t2_ms } else { *v };
                    if d <= now {
                        pick = Some((Some(*k), None));
                        break;
                    }
                }
                if pick.is_none() {
                    for (k, v) in &g.part_due {
                        if *v <= now {
                            pick = Some((None, Some(k.clone())));
                            break;
                        }
                    }
                }
                pick
            };
            let Some(due) = due else { break };
            let o = match due {
                (Some(t), _) => {
                    {
                        let mut g = lock(&self.inner);
                        g.server_due.remove(&t);
                    }
                    self.run(now, |st| match t {
                        Tm::T1 => st.t1(),
                        Tm::T2 => st.t2(),
                        Tm::T3 => st.t3(),
                        Tm::T7 => st.t7(),
                        Tm::T20 => st.t20(),
                    })
                }
                (None, Some((u, t))) => {
                    {
                        let mut g = lock(&self.inner);
                        g.part_due.remove(&(u.clone(), t));
                    }
                    self.run(now, |st| match t {
                        Tp::T8 => st.t8(&u),
                        Tp::T9 => st.t9(&u),
                        Tp::T21 => st.t21(&u),
                    })
                }
                _ => break,
            };
            out.push(o);
        }
        out
    }

    pub fn rx(&self, u: &str, m: Msg, now: u64) -> Outcome {
        self.run(now, |st| st.server_rx(u, m))
    }

    pub fn ready(&self, u: &str, now: u64) -> Outcome {
        self.run(now, |st| st.ready(u))
    }

    pub fn join(&self, u: &str, perm: bool, half: bool, is_pub: bool, now: u64) -> Outcome {
        self.run(now, |st| st.join(u, perm, half, is_pub))
    }

    pub fn leave(&self, u: &str, now: u64) -> Outcome {
        self.run(now, |st| st.ws_leave(u))
    }

    pub fn release_for(&self, u: &str, now: u64) -> Outcome {
        self.run(now, |st| st.release(u))
    }

    pub fn pub_select(&self, u: &str, now: u64) -> Outcome {
        self.run(now, |st| st.ws_pub_select(u))
    }

    pub fn pub_away(&self, u: &str, now: u64) -> Outcome {
        self.run(now, |st| st.ws_pub_away(u))
    }

    pub fn reg_half(&self, u: &str, on: bool, now: u64) -> Outcome {
        self.run(now, |st| if on { st.ws_reg_half(u) } else { st.ws_unreg_half(u) })
    }

    pub fn perm(&self, u: &str, on: bool, now: u64) -> Outcome {
        self.run(now, |st| if on { st.perm_up(u) } else { st.perm_down(u) })
    }

    pub fn holder(&self) -> Option<String> {
        Step::holder(&lock(&self.inner).srv)
    }

    pub fn granted_at(&self) -> u64 {
        lock(&self.inner).granted_at
    }

    pub fn speaker_is(&self, u: &str) -> bool {
        self.holder().as_deref() == Some(u)
    }

    pub fn on_rtp(&self, u: &str, now: u64) -> Option<Outcome> {
        self.last_rtp_ms.store(now, Ordering::Release);
        let last = self.last_note_ms.load(Ordering::Acquire);
        if last != 0 && now.saturating_sub(last) < RTP_NOTE_MS {
            return None;
        }
        self.last_note_ms.store(now.max(1), Ordering::Release);
        Some(self.run(now, |st| st.server_rx(u, Msg::new(T::Rtp, None, 0, 0))))
    }

    pub fn snapshot(&self) -> Value {
        let g = lock(&self.inner);
        let speaker = Step::holder(&g.srv);
        json!({
            "speaker": speaker,
            "queue": g.srv.queue.iter().map(|w| json!({"user_id": w.u, "priority": w.p})).collect::<Vec<_>>(),
            "started_at": if speaker.is_some() { Some(g.holder_since) } else { None },
            "state": format!("{:?}", g.srv.g),
        })
    }

    pub fn started_wall(&self) -> u64 {
        lock(&self.inner).started_at_wall
    }
}

pub fn decode(b: &[u8]) -> Option<(String, Msg, bool)> {
    let m = mbcp::decode(b)?;
    let room = m.room()?;
    let t = match m.msg_type {
        mbcp::REQUEST => T::Request,
        mbcp::RELEASE => T::Release,
        mbcp::QPOS_REQ => T::QposReq,
        mbcp::ACK => T::Ack,
        _ => return None,
    };
    let n = match t {
        T::Request => m.get_u8spare(tlv::PRIORITY).or_else(|| m.get_u8(tlv::PRIORITY)).unwrap_or(0) as u32,
        T::Ack => m.get_u8spare(tlv::ACKTYPE).or_else(|| m.get_u8(tlv::ACKTYPE)).unwrap_or(0) as u32,
        _ => 0,
    };
    let q = m.get_u16(tlv::SEQ).unwrap_or(0);
    Some((room, Msg { t, a: None, n, d: 0, q }, m.ack))
}

pub fn encode(room: &str, m: &Msg) -> Option<Vec<u8>> {
    let out = match m.t {
        T::Granted => mbcp::Msg::new(mbcp::GRANTED).with_ack(true).put_u8spare(tlv::PRIORITY, m.n as u8).put_u16(tlv::DURATION, m.d as u16).put_u16(tlv::SEQ, m.q),
        T::Taken => mbcp::Msg::new(mbcp::TAKEN).with_ack(true).put_str(tlv::SPEAKER, m.a.as_deref().unwrap_or("")).put_u16(tlv::SEQ, m.q),
        T::Deny => mbcp::Msg::new(mbcp::DENY).with_ack(true).put_u8(tlv::CAUSE, m.n as u8),
        T::Idle => {
            let mut x = mbcp::Msg::new(mbcp::IDLE);
            if let Some(p) = m.a.as_deref() {
                x = x.put_str(tlv::PREV, p);
            }
            x.put_u16(tlv::SEQ, m.q)
        }
        T::Revoke => mbcp::Msg::new(mbcp::REVOKE).put_u8(tlv::CAUSE, m.n as u8),
        T::Qinfo => mbcp::Msg::new(mbcp::QINFO).put_pair(tlv::QINFO, m.n.min(255) as u8, m.d.min(255) as u8),
        _ => return None,
    };
    let body = mbcp::encode(&out.put_str(tlv::ROOM, room))?;
    oxsig::dc::DcFrame::encode(oxsig::dc::SVC_FLOOR, &body)
}

pub fn ack_for(room: &str, of: u8, seq: Option<u16>) -> Option<Vec<u8>> {
    let mut x = mbcp::Msg::new(mbcp::ACK).put_u8spare(tlv::ACKTYPE, of);
    if let Some(s) = seq {
        x = x.put_u16(tlv::SEQ, s);
    }
    let body = mbcp::encode(&x.put_str(tlv::ROOM, room))?;
    oxsig::dc::DcFrame::encode(oxsig::dc::SVC_FLOOR, &body)
}

pub fn name(t: T) -> &'static str {
    match t {
        T::Request => "CP-MSG-FLOOR-REQUEST",
        T::Granted => "CP-MSG-FLOOR-GRANTED",
        T::Taken => "CP-MSG-FLOOR-TAKEN",
        T::Deny => "CP-MSG-FLOOR-DENY",
        T::Release => "CP-MSG-FLOOR-RELEASE",
        T::Idle => "CP-MSG-FLOOR-IDLE",
        T::Revoke => "CP-MSG-FLOOR-REVOKE",
        T::QposReq => "CP-MSG-FLOOR-QUEUE-POS-REQUEST",
        T::Qinfo => "CP-MSG-FLOOR-QUEUE-INFO",
        T::Ack => "CP-MSG-FLOOR-ACK",
        T::Rtp => "IMPL-FLR-RTP-OBSERVE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_run_on_deadlines_not_ticks() {
        let f = Floor::new("r", 10, 30);
        f.join("a", true, true, true, 0);
        f.join("b", true, true, true, 0);
        let o = f.rx("a", Msg::new(T::Request, None, 0, 0), 1000);
        assert!(o.labels.contains(&"FLR.S.G.IDLE.req_grant"));
        let taken_q = o.outs.iter().find(|x| x.to == "b").map(|x| x.m.q).expect("taken");
        let ack = f.rx("b", Msg { t: T::Ack, a: None, n: 2, d: 0, q: taken_q }, 1000);
        assert!(ack.labels.contains(&"FLR.S.P.NP_TAKEN.rx_ACK_taken"));
        assert_eq!(f.next_due(), Some(1000 + T1_MS), "S-sym-IMPL-FLR-RTP-OBSERVE-1 T1 은 허가부터 — RTP 가 오면 밀린다");
        assert!(f.on_rtp("a", 2000).is_some());
        assert!(f.on_rtp("a", 2100).is_none(), "S-sym-IMPL-FLR-RTP-OBSERVE-2 RTP 사건은 500ms 에 한 번");
        assert_eq!(f.next_due(), Some((2100 + T1_MS).min(2000 + 30_000)));
        let fired = f.fire(2100 + T1_MS);
        assert!(fired.iter().any(|o| o.labels.contains(&"FLR.S.G.TAKEN.T1")), "S-sym-IMPL-FLR-RTP-OBSERVE-3 T1 은 마지막 RTP 부터");
        assert!(f.holder().is_none());
    }

    #[test]
    fn wire_round_trip() {
        let enc = encode("r1", &Msg { t: T::Taken, a: Some("u1".into()), n: 0, d: 0, q: 7 }).expect("enc");
        let frame = oxsig::dc::DcFrame::decode(&enc).expect("frame");
        let m = mbcp::decode(&frame.payload).expect("mbcp");
        assert_eq!(m.msg_type, mbcp::TAKEN);
        assert!(m.ack);
        assert_eq!(m.get_u16(tlv::SEQ), Some(7));
        let req = mbcp::encode(&mbcp::Msg::new(mbcp::REQUEST).put_u8spare(tlv::PRIORITY, 3).put_str(tlv::ROOM, "r1")).expect("req");
        let (room, msg, _) = decode(&req).expect("dec");
        assert_eq!(room, "r1");
        assert_eq!(msg.n, 3);
    }
}
