// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-FLR-LOCK 정:IMPL-FLR-RTP-OBSERVE 정:IMPL-FLR-POLICY 정:IMPL-FLR-FIRST-NOTICE 정:RULE-FLR-SNAPSHOT 정:RULE-CONC-SIX 정:RULE-GC-FOUR 운영:RULE-READ-ROOM-DETAIL 연:RULE-FLR-TIMERS 연:ASM-FLR-RTP-INSTANT · model: claude-opus-5-5
//! 방마다 한 락 — 판정은 락 안(생성 판정 로직), 보낼 것 · 효과는 목록으로 돌려 락 밖에서 집행 · 타이머는 만료 시각으로(가장 이른 시각에 깨어난다) · RTP 는 사람마다 500ms 에 한 번(첫 RTP 즉시).

pub mod core;

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use self::core::{Cfg, Fx, G, M, Out, P, R, Srv};

pub const RTP_NOTIFY_MS: u64 = 500;

pub struct Step {
    pub out: Vec<Out>,
    pub fx: Vec<Fx>,
    pub labels: Vec<core::Lab>,
    pub cause: &'static str,
    pub from: G,
    pub to: G,
    pub user: Option<String>,
}

impl Step {
    pub fn empty(&self) -> bool {
        self.out.is_empty() && self.fx.is_empty()
    }
}

#[derive(Default)]
struct Inner {
    s: Srv,
    dl: HashMap<(&'static str, Option<String>), u64>,
    granted_at: u64,
    rtp_notified: HashMap<String, u64>,
}

pub struct Floor {
    pub room: String,
    qmax: u32,
    t2_secs: u32,
    inner: std::sync::Mutex<Inner>,
    pub first_rtp_at: AtomicU64,
    pub last_rtp_at: AtomicU64,
    pub wake: tokio::sync::Notify,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QRow {
    pub user_id: String,
    pub priority: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TimerRow {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub at: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct View {
    pub state: &'static str,
    pub speaker: Option<String>,
    pub priority: u32,
    pub queue: Vec<QRow>,
    pub first_rtp_at: Option<u64>,
    pub last_rtp_at: Option<u64>,
    pub timers: Vec<TimerRow>,
}

fn opt(v: u64) -> Option<u64> {
    (v != 0).then_some(v)
}

impl Floor {
    pub fn new(room: &str, queue_max: u8, t2_secs: u16) -> Floor {
        Floor {
            room: room.to_string(),
            qmax: u32::from(queue_max),
            t2_secs: u32::from(t2_secs),
            inner: std::sync::Mutex::new(Inner::default()),
            first_rtp_at: AtomicU64::new(0),
            last_rtp_at: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn cfg(&self, now: u64) -> Cfg {
        let first = self.first_rtp_at.load(Ordering::Acquire);
        let used = if first == 0 { 0 } else { now.saturating_sub(first) / 1000 };
        Cfg { qmax: self.qmax, t2_remain_secs: (u64::from(self.t2_secs)).saturating_sub(used) as u32 }
    }

    fn t1_base(&self, g: &Inner) -> u64 {
        g.granted_at.max(self.last_rtp_at.load(Ordering::Acquire))
    }

    fn apply(&self, g: &mut Inner, r: R, now: u64, cause: &'static str, user: Option<&str>, fired: Option<(&'static str, Option<String>)>) -> Step {
        let before = g.s.clone();
        let from = before.g;
        g.s = r.s;
        let holder_changed = before.holder() != g.s.holder() || (before.g == G::Idle) != (g.s.g == G::Idle);
        if holder_changed && g.s.g == G::Taken && before.speaker != g.s.speaker {
            g.granted_at = now;
            self.first_rtp_at.store(0, Ordering::Release);
            self.last_rtp_at.store(0, Ordering::Release);
        }
        let mut want: Vec<(&'static str, Option<String>)> = self::core::armed_server(&g.s).into_iter().map(|n| (n, None)).collect();
        let users: BTreeSet<String> = g.s.part.keys().cloned().collect();
        for u in &users {
            for n in self::core::armed_participant(&g.s, u) {
                want.push((n, Some(u.clone())));
            }
        }
        g.dl.retain(|k, _| want.contains(k));
        for k in want {
            let restart = fired.as_ref() == Some(&k) || self.counter_reset(&before, &g.s, &k);
            if restart || !g.dl.contains_key(&k) {
                let at = match k.0 {
                    "T1" => self.t1_base(g) + self::core::timer_ms("T1"),
                    "T2" => {
                        let f = self.first_rtp_at.load(Ordering::Acquire);
                        (if f == 0 { now } else { f }) + u64::from(self.t2_secs) * 1000
                    }
                    n => now + self::core::timer_ms(n),
                };
                g.dl.insert(k, at);
            }
        }
        self.wake.notify_one();
        Step { out: r.o, fx: r.fx, labels: r.l, cause, from, to: g.s.g, user: user.map(str::to_string) }
    }

    fn counter_reset(&self, a: &Srv, b: &Srv, k: &(&'static str, Option<String>)) -> bool {
        let get = |m: &HashMap<String, u32>, u: &str| -> u32 { m.get(u).copied().unwrap_or(0) };
        match (k.0, k.1.as_deref()) {
            ("T21", Some(u)) => get(&b.c21, u) == 1 && (get(&a.c21, u) != 1 || a.tq.get(u) != b.tq.get(u)),
            ("T8", Some(u)) => get(&b.c8, u) == 1 && get(&a.c8, u) != 1,
            ("T7", None) => b.c7 == 1 && a.c7 != 1,
            ("T20", None) => b.c20 == 1 && a.c20 != 1,
            ("T3", None) => b.g == G::Prevoke && a.g != G::Prevoke,
            _ => false,
        }
    }

    pub fn join(&self, u: &str, is_pub: bool, half: bool, perm: bool) {
        let mut g = self.lock();
        let s = &mut g.s;
        s.members.insert(u.to_string());
        s.pub_.insert(u.to_string(), is_pub);
        s.half.insert(u.to_string(), half);
        s.perm.insert(u.to_string(), perm);
        s.retry.insert(u.to_string(), false);
        let p = if s.holder().is_some() { P::NpTaken } else { P::NpIdle };
        s.part.insert(u.to_string(), p);
        s.g_taken.insert(u.to_string(), p == P::NpTaken);
        s.c21.insert(u.to_string(), 0);
        s.c8.insert(u.to_string(), 0);
        s.released.insert(u.to_string(), false);
    }

    pub fn set_half(&self, u: &str, half: bool, perm: bool, now: u64) -> Option<Step> {
        let mut g = self.lock();
        g.s.perm.insert(u.to_string(), perm);
        if !half {
            return None;
        }
        let r = self::core::ws_reg_half(g.s.clone(), u);
        Some(self.apply(&mut g, r, now, "reg_half", Some(u), None))
    }

    pub fn rx(&self, u: &str, m: &M, now: u64) -> Step {
        let c = self.cfg(now);
        let mut g = self.lock();
        let r = self::core::server_rx(g.s.clone(), u, m, &c);
        self.apply(&mut g, r, now, "dc", Some(u), None)
    }

    pub fn observe_rtp_gated(&self, u: &str, now: u64, pass: bool) -> Option<Step> {
        let holder = { self.lock().s.holder() };
        if pass && holder.as_deref() == Some(u) {
            self.last_rtp_at.store(now, Ordering::Release);
            let first = self.first_rtp_at.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire).is_ok();
            let mut g = self.lock();
            let last = g.rtp_notified.get(u).copied().unwrap_or(0);
            if !first && now.saturating_sub(last) < RTP_NOTIFY_MS {
                return None;
            }
            g.rtp_notified.insert(u.to_string(), now);
            let c = self.cfg(now);
            let r = self::core::server_rx(g.s.clone(), u, &M::new(self::core::T::Rtp, None, 0, 0), &c);
            return Some(self.apply(&mut g, r, now, "rtp", Some(u), None));
        }
        let mut g = self.lock();
        let last = g.rtp_notified.get(u).copied().unwrap_or(0);
        if now.saturating_sub(last) < RTP_NOTIFY_MS {
            return None;
        }
        g.rtp_notified.insert(u.to_string(), now);
        let c = self.cfg(now);
        let r = self::core::server_rx(g.s.clone(), u, &M::new(self::core::T::Rtp, None, 0, 0), &c);
        Some(self.apply(&mut g, r, now, "rtp", Some(u), None))
    }

    pub fn release(&self, u: &str, now: u64) -> Step {
        let c = self.cfg(now);
        let mut g = self.lock();
        let r = self::core::away(g.s.clone(), u, &c);
        self.apply(&mut g, r, now, "release", Some(u), None)
    }

    pub fn ready(&self, u: &str, now: u64) -> Step {
        let mut g = self.lock();
        let r = self::core::ready(g.s.clone(), u);
        self.apply(&mut g, r, now, "ready", Some(u), None)
    }

    pub fn perm(&self, u: &str, ok: bool, now: u64) -> Step {
        let c = self.cfg(now);
        let mut g = self.lock();
        let r = if ok { self::core::perm_up(g.s.clone(), u) } else { self::core::perm_down(g.s.clone(), u, &c) };
        self.apply(&mut g, r, now, "perm", Some(u), None)
    }

    pub fn pub_select(&self, u: &str, now: u64) -> Step {
        let mut g = self.lock();
        let r = self::core::ws_pub_select(g.s.clone(), u);
        self.apply(&mut g, r, now, "pub_select", Some(u), None)
    }

    pub fn pub_away(&self, u: &str, now: u64) -> Step {
        let c = self.cfg(now);
        let mut g = self.lock();
        let r = self::core::ws_pub_away(g.s.clone(), u, &c);
        self.apply(&mut g, r, now, "pub_away", Some(u), None)
    }

    pub fn leave(&self, u: &str, now: u64) -> Step {
        let c = self.cfg(now);
        let mut g = self.lock();
        let r = self::core::ws_leave(g.s.clone(), u, &c);
        g.rtp_notified.remove(u);
        self.apply(&mut g, r, now, "leave", Some(u), None)
    }

    pub fn next_deadline(&self) -> Option<u64> {
        self.lock().dl.values().min().copied()
    }

    pub fn due(&self, now: u64) -> Vec<Step> {
        let mut steps = Vec::new();
        for _ in 0..64 {
            let mut g = self.lock();
            let Some((k, at)) = g.dl.iter().min_by_key(|(_, v)| **v).map(|(k, v)| (k.clone(), *v)) else { break };
            if at > now {
                break;
            }
            if k.0 == "T1" {
                let real = self.t1_base(&g) + self::core::timer_ms("T1");
                if real > now {
                    g.dl.insert(k, real);
                    continue;
                }
            }
            g.dl.remove(&k);
            let c = self.cfg(now);
            let s = g.s.clone();
            let r = match (k.0, k.1.as_deref()) {
                ("T1", None) => self::core::srv_t1(s, &c),
                ("T2", None) => self::core::srv_t2(s, &c),
                ("T3", None) => self::core::srv_t3(s, &c),
                ("T7", None) => self::core::srv_t7(s, &c),
                ("T20", None) => self::core::srv_t20(s, &c),
                ("T8", Some(u)) => self::core::srv_t8(s, u),
                ("T21", Some(u)) => self::core::srv_t21(s, u),
                ("T9", Some(u)) => self::core::srv_t9(s, u),
                _ => continue,
            };
            let user = k.1.clone();
            steps.push(self.apply(&mut g, r, now, k.0, user.as_deref(), Some(k)));
        }
        steps
    }

    pub fn holder(&self) -> Option<String> {
        self.lock().s.holder()
    }

    pub fn state(&self) -> G {
        self.lock().s.g
    }

    pub fn part_of(&self, u: &str) -> P {
        self.lock().s.part(u)
    }

    pub fn in_queue(&self, u: &str) -> bool {
        self.lock().s.queue.iter().any(|w| w.u == u)
    }

    pub fn view(&self) -> View {
        let g = self.lock();
        let s = &g.s;
        let mut timers: Vec<TimerRow> = g.dl.iter().map(|((n, u), at)| TimerRow { name: (*n).to_string(), user_id: u.clone(), at: *at }).collect();
        timers.sort_by(|a, b| a.at.cmp(&b.at).then(a.name.cmp(&b.name)));
        View {
            state: s.g.name(),
            speaker: s.holder(),
            priority: if s.g == G::Taken { s.sprio } else { s.rprio },
            queue: s.queue.iter().map(|w| QRow { user_id: w.u.clone(), priority: w.p }).collect(),
            first_rtp_at: opt(self.first_rtp_at.load(Ordering::Acquire)),
            last_rtp_at: opt(self.last_rtp_at.load(Ordering::Acquire)),
            timers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flr::core::T;

    #[test]
    fn deadlines_follow_ledger() {
        let f = Floor::new("r", 10, 30);
        f.join("a", true, true, true);
        f.join("b", true, true, true);
        let st = f.rx("a", &M::new(T::Request, None, 0, 0), 1000);
        assert_eq!(st.to, G::Taken);
        let v = f.view();
        let t1 = v.timers.iter().find(|t| t.name == "T1").map(|t| t.at);
        assert_eq!(t1, Some(5000), "T1 = 허가 뒤 4000ms");
        let t21 = v.timers.iter().find(|t| t.name == "T21").map(|t| t.at);
        assert_eq!(t21, Some(2000), "T21 = 1000ms");
        assert!(f.observe_rtp_gated("a", 1500, true).is_some(), "첫 RTP 는 즉시");
        assert!(f.observe_rtp_gated("a", 1600, true).is_none(), "500ms 에 한 번");
        let steps = f.due(5000);
        assert!(steps.iter().all(|s| s.to == G::Taken), "RTP 가 T1 을 밀었다");
        let steps = f.due(5600);
        assert!(steps.iter().any(|s| s.labels.iter().any(|(l, _)| *l == "FLR.S.G.TAKEN.T1")));
        assert_eq!(f.state(), G::Idle);
    }

    #[test]
    fn t2_from_first_rtp() {
        let f = Floor::new("r", 10, 30);
        f.join("a", true, true, true);
        f.join("b", true, true, true);
        f.rx("a", &M::new(T::Request, None, 0, 0), 0);
        f.observe_rtp_gated("a", 2_000, true);
        let v = f.view();
        assert_eq!(v.timers.iter().find(|t| t.name == "T2").map(|t| t.at), Some(32_000), "S-sym-IMPL-FLR-RTP-OBSERVE-1 T2 는 첫 RTP 부터");
    }
}
