// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FLR-SERVER-SCOPE · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-POLICY · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-FLR-SNAPSHOT · 정:RULE-CONC-SIX · 연:RULE-FLR-TIMERS · 연:FLR.wire · model: claude-opus-5-5
//! 발언권 — 판정 로직(`core`) 둘레의 방마다 그릇: 허가 번호(`grant_no` · 0 에서 · 허가마다 +1) · 타이머 만료 시각(가장 이른 것에 깨어난다 · `T1` 은 마지막 RTP 부터 · `T2` 는 첫 RTP 부터) · MBCP 와 판정 메시지 사이 번역.

pub mod bind;
pub mod core;

use std::collections::BTreeMap;

use oxsig::mbcp;

use self::core::{Env, Fx, G, Msg, PTimer, R, Srv, T, Timer, ms};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    G(Timer),
    P(String, PTimer),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Bind(String, u64),
    Release(u64, Option<String>),
}

#[derive(Debug, Clone)]
pub struct Stepped {
    pub r: R,
    pub from: G,
    pub to: G,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone)]
pub struct Floor {
    pub srv: Srv,
    pub grant_no: u64,
    current: Option<u64>,
    deadlines: BTreeMap<Key, u64>,
    t1_base: u64,
    first_rtp_ms: u64,
    pub qmax: usize,
    pub t2_secs: u32,
    pub speaker_since_ms: u64,
}

impl Floor {
    pub fn new(qmax: usize, t2_secs: u32) -> Floor {
        Floor { srv: Srv::new(), grant_no: 0, current: None, deadlines: BTreeMap::new(), t1_base: 0, first_rtp_ms: 0, qmax, t2_secs, speaker_since_ms: 0 }
    }

    fn t2_ms(&self) -> u64 {
        u64::from(self.t2_secs) * 1_000
    }

    fn env(&self, now_ms: u64) -> Env {
        let remain = if self.srv.first_rtp { (self.first_rtp_ms + self.t2_ms()).saturating_sub(now_ms) / 1_000 } else { u64::from(self.t2_secs) };
        Env { qmax: self.qmax, t2_secs: self.t2_secs, t2_remain_secs: remain as u32 }
    }

    pub fn step(&mut self, now_ms: u64, f: impl FnOnce(&mut Srv, Env) -> R) -> Stepped {
        let from = self.srv.g;
        let env = self.env(now_ms);
        let r = f(&mut self.srv, env);
        let mut effects = Vec::new();
        for fx in &r.fx {
            match fx {
                Fx::Bind(u) => {
                    let g = self.grant_no;
                    self.grant_no += 1;
                    self.current = Some(g);
                    self.speaker_since_ms = now_ms;
                    effects.push(Effect::Bind(u.clone(), g));
                }
                Fx::Release(prev) => {
                    if let Some(g) = self.current.take() {
                        effects.push(Effect::Release(g, prev.clone()));
                    }
                }
            }
        }
        self.rearm(now_ms);
        Stepped { to: self.srv.g, from, r, effects }
    }

    fn dur(t: Timer, t2: u64) -> u64 {
        match t {
            Timer::T1 => ms::T1,
            Timer::T2 => t2,
            Timer::T3 => ms::T3,
            Timer::T7 => ms::T7,
            Timer::T20 => ms::T20,
        }
    }

    fn pdur(t: PTimer) -> u64 {
        match t {
            PTimer::T8 => ms::T8,
            PTimer::T9 => ms::T9,
            PTimer::T21 => ms::T21,
        }
    }

    fn rearm(&mut self, now_ms: u64) {
        let restarted = std::mem::take(&mut self.srv.restarted);
        let restarted_p = std::mem::take(&mut self.srv.restarted_p);
        let armed = self.srv.armed();
        let t2 = self.t2_ms();
        for t in restarted.iter().copied() {
            if t == Timer::T1 {
                self.t1_base = now_ms;
            }
            if t == Timer::T2 {
                self.first_rtp_ms = now_ms;
            }
        }
        self.deadlines.retain(|k, _| match k {
            Key::G(t) => armed.contains(t),
            Key::P(..) => true,
        });
        for t in &armed {
            let k = Key::G(*t);
            if restarted.contains(t) || !self.deadlines.contains_key(&k) {
                let base = match t {
                    Timer::T1 => self.t1_base,
                    Timer::T2 => self.first_rtp_ms,
                    _ => now_ms,
                };
                self.deadlines.insert(k, base + Self::dur(*t, t2));
            }
        }
        let people: Vec<String> = self.srv.people.keys().cloned().collect();
        for u in people {
            let on = self.srv.armed_p(&u);
            for t in [PTimer::T8, PTimer::T9, PTimer::T21] {
                let k = Key::P(u.clone(), t);
                if !on.contains(&t) {
                    self.deadlines.remove(&k);
                    continue;
                }
                if restarted_p.iter().any(|(x, y)| *x == u && *y == t) || !self.deadlines.contains_key(&k) {
                    self.deadlines.insert(k, now_ms + Self::pdur(t));
                }
            }
        }
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.deadlines.values().min().copied()
    }

    pub fn due(&self, now_ms: u64) -> Vec<Key> {
        self.deadlines.iter().filter(|(_, d)| **d <= now_ms).map(|(k, _)| k.clone()).collect()
    }

    pub fn fire(&mut self, k: &Key, now_ms: u64, last_rtp_ms: u64) -> Option<Stepped> {
        let d = self.deadlines.get(k).copied()?;
        if d > now_ms {
            return None;
        }
        if *k == Key::G(Timer::T1) {
            let real = self.t1_base.max(last_rtp_ms) + ms::T1;
            if real > now_ms {
                self.deadlines.insert(k.clone(), real);
                return None;
            }
        }
        self.deadlines.remove(k);
        let s = match k {
            Key::G(Timer::T1) => self.step(now_ms, core::t1),
            Key::G(Timer::T2) => self.step(now_ms, core::t2),
            Key::G(Timer::T3) => self.step(now_ms, core::t3),
            Key::G(Timer::T7) => self.step(now_ms, core::t7),
            Key::G(Timer::T20) => self.step(now_ms, core::t20),
            Key::P(u, PTimer::T8) => self.step(now_ms, |s, e| core::t8(s, e, u)),
            Key::P(u, PTimer::T9) => self.step(now_ms, |s, e| core::t9(s, e, u)),
            Key::P(u, PTimer::T21) => self.step(now_ms, |s, e| core::t21(s, e, u)),
        };
        let periodic = matches!(k, Key::G(Timer::T7) | Key::G(Timer::T20) | Key::P(_, PTimer::T8) | Key::P(_, PTimer::T21));
        if periodic && !self.deadlines.contains_key(k) {
            let still = match k {
                Key::G(t) => self.srv.armed().contains(t),
                Key::P(u, t) => self.srv.armed_p(u).contains(t),
            };
            if still {
                let dur = match k {
                    Key::G(t) => Self::dur(*t, self.t2_ms()),
                    Key::P(_, t) => Self::pdur(*t),
                };
                self.deadlines.insert(k.clone(), now_ms + dur);
            }
        }
        Some(s)
    }

    pub fn timers(&self, last_rtp_ms: u64) -> Vec<(String, Option<String>, u64)> {
        self.deadlines
            .iter()
            .map(|(k, d)| match k {
                Key::G(Timer::T1) => ("T1".to_string(), None, self.t1_base.max(last_rtp_ms) + ms::T1),
                Key::G(t) => (format!("{t:?}"), None, *d),
                Key::P(u, t) => (format!("{t:?}"), Some(u.clone()), *d),
            })
            .collect()
    }

    pub fn first_rtp_ms(&self) -> Option<u64> {
        self.srv.first_rtp.then_some(self.first_rtp_ms)
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let q: Vec<serde_json::Value> = self.srv.queue.iter().map(|w| serde_json::json!({"user_id": w.u, "priority": w.p})).collect();
        serde_json::json!({
            "speaker": self.srv.holder(),
            "queue": q,
            "since": if self.srv.holder().is_some() { Some(self.speaker_since_ms) } else { None },
        })
    }
}

pub fn decode(m: &mbcp::Msg) -> Option<(Msg, Option<String>)> {
    let t = T::from_wire(m.msg_type)?;
    let room = m.room();
    let n = match t {
        T::Request => u32::from(m.get_u8spare(mbcp::F_PRIORITY).unwrap_or(0)),
        T::Ack => u32::from(m.get_u8spare(mbcp::F_ACK_TYPE).unwrap_or(0)),
        _ => 0,
    };
    let q = m.seq().unwrap_or(0);
    Some((Msg { t, a: None, n, d: 0, q }, room))
}

pub fn encode(m: &Msg, room: &str) -> Option<Vec<u8>> {
    let wire_t = m.t.wire()?;
    let ack = matches!(m.t, T::Granted | T::Taken | T::Deny);
    let mut out = mbcp::Msg::new(wire_t).with_ack(ack);
    match m.t {
        T::Granted => {
            out = out.put_u8spare(mbcp::F_PRIORITY, m.n as u8).put_u16(mbcp::F_DURATION, m.d as u16).put_u16(mbcp::F_SEQ, m.q);
        }
        T::Taken => {
            out = out.put_str(mbcp::F_GRANTED_PARTY, m.a.as_deref().unwrap_or("")).put_u16(mbcp::F_SEQ, m.q);
        }
        T::Idle => {
            if let Some(p) = &m.a {
                out = out.put_str(mbcp::F_PREV_SPEAKER, p);
            }
            out = out.put_u16(mbcp::F_SEQ, m.q);
        }
        T::Deny => {
            out = out.put_u8(mbcp::F_CAUSE, m.n as u8);
            if m.n == core::deny::OTHER_REASON {
                out = out.put_str(mbcp::F_CAUSE_TEXT, "not_in_room");
            }
        }
        T::Revoke => {
            out = out.put_u8(mbcp::F_CAUSE, m.n as u8);
        }
        T::Qinfo => {
            out.tlvs.push((mbcp::F_QUEUE_INFO, vec![m.n as u8, m.d as u8]));
        }
        T::Ack => {
            out = out.put_u8spare(mbcp::F_ACK_TYPE, m.n as u8);
            if m.q != 0 {
                out = out.put_u16(mbcp::F_SEQ, m.q);
            }
        }
        _ => {}
    }
    out = out.put_str(mbcp::F_ROOM, room);
    mbcp::encode(&out)
}

#[cfg(test)]
mod tests {
    use super::core::{ws_join, ws_pub_select};
    use super::*;

    const E: Env = Env { qmax: 10, t2_secs: 30, t2_remain_secs: 30 };

    #[test]
    fn grant_numbers_and_t1_from_last_rtp() {
        let mut f = Floor::new(10, 30);
        for u in ["a", "b"] {
            ws_join(&mut f.srv, E, u, true, true);
            ws_pub_select(&mut f.srv, E, u);
        }
        let s = f.step(0, |s, e| core::server_rx(s, e, "a", &Msg::new(T::Request, None, 0, 0)));
        assert_eq!(s.effects, vec![Effect::Bind("a".into(), 0)]);
        assert_eq!(f.next_deadline(), Some(ms::T21), "TAKEN 을 받은 b 의 T21 이 가장 이르다");
        assert!(f.timers(0).contains(&("T1".to_string(), None, ms::T1)));
        assert!(f.fire(&Key::G(Timer::T1), 4_000, 3_000).is_none());
        let s = f.fire(&Key::G(Timer::T1), 7_000, 3_000).expect("t1");
        assert_eq!(s.effects, vec![Effect::Release(0, Some("a".into()))]);
        let s = f.step(7_100, |s, e| core::server_rx(s, e, "b", &Msg::new(T::Request, None, 0, 0)));
        assert_eq!(s.effects, vec![Effect::Bind("b".into(), 1)]);
    }

    #[test]
    fn mbcp_round_trip() {
        let m = Msg { t: T::Taken, a: Some("u2".into()), n: 0, d: 0, q: 7 };
        let b = encode(&m, "r1").expect("enc");
        let d = mbcp::decode(&b).expect("dec");
        assert_eq!(d.room().as_deref(), Some("r1"));
        assert!(d.ack);
        let req = mbcp::encode(&mbcp::Msg::new(mbcp::REQUEST).put_u8spare(mbcp::F_PRIORITY, 5).put_str(mbcp::F_ROOM, "r1")).expect("req");
        let (m, room) = decode(&mbcp::decode(&req).expect("d")).expect("m");
        assert_eq!((m.t, m.n, room.as_deref()), (T::Request, 5, Some("r1")));
    }
}
