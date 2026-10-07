// author: kodeholic (powered by Claude)
// spec: floor 원천 · 3GPP TS 24.380 §6.3.4 · §6.3.5 · 연§8-4 · §11 · model: claude-opus-5-5

//! 발언권 서버 — 판정 로직(6.3.4)과 참가자마다의 상태기(6.3.5)를 원문 짜임새 그대로 둔다.
//!
//! 판정 로직이 받는 것은 셋이다 — Idle 의 요청 · 화자의 재요청 · 참가자가 넘긴 선점 요청.
//! 대기·재통보·만석·순번·반환 확인은 참가자 절이 한다. 절이 없는 칸은 버리고 머문다.
//! 시계와 소켓은 주인이 쥔다 — 여기는 사람마다 보낼 것만 돌려준다.

use std::collections::BTreeMap;

use oxsig::mbcp::{reject, revoke};

pub mod timers {
    pub const T1_MS: u64 = 4_000;
    pub const T2_MS: u64 = 30_000;
    pub const T3_MS: u64 = 3_000;
    pub const T7_MS: u64 = 1_000;
    pub const C7: u8 = 10;
    pub const T8_MS: u64 = 1_000;
    pub const C8: u8 = 3;
    pub const T20_MS: u64 = 1_000;
    pub const C20: u8 = 3;
    pub const T9_MS: u64 = 3_000;
    pub const TICK_MS: u64 = 250;
}

pub const QUEUE_MAX: usize = 10;
pub const NOT_IN_QUEUE: u8 = 254;
pub const NOT_IN_ROOM: &str = "not_in_room";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum General {
    Idle,
    Taken,
    PendingRevoke,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Participant {
    NotPermittedIdle,
    NotPermittedTaken,
    Permitted,
    PendingRevoke,
    NotPermittedSends,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rx {
    Request { priority: u8 },
    Release,
    QueuePos,
    Ack,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ctx {
    pub in_room: bool,
    pub has_half: bool,
    pub in_pub_room: bool,
    pub permitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wire {
    Granted { priority: u8, duration_s: u16 },
    Deny { cause: u8, text: Option<&'static str> },
    Revoke { cause: u8 },
    QueueInfo { position: u8, priority: u8 },
    Taken { speaker: String, seq: u16 },
    Idle { prev: Option<String>, seq: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub to: String,
    pub wire: Wire,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiter {
    user: String,
    priority: u8,
    pre: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Part {
    state: Participant,
    released: bool,
    g_taken: bool,
    c8: u8,
    t8_at: u64,
    cause: u8,
}

#[derive(Debug, Clone)]
enum Arb {
    Granted { priority: u8, duration_s: u16 },
    Taken(String),
    Idle,
    Revoke(u8),
    QueueInfo { position: u8, priority: u8 },
    Deny(u8),
}

#[derive(Debug, Clone)]
pub struct Floor {
    t2_ms: u64,
    g: General,
    speaker: Option<String>,
    sprio: u8,
    revoking: Option<String>,
    rprio: u8,
    queue: Vec<Waiter>,
    succ: bool,
    rtp_seen: bool,
    first_rtp_at: Option<u64>,
    last_rtp_at: u64,
    c20: u8,
    t20_at: u64,
    t3_at: u64,
    c7: u8,
    t7_at: u64,
    idle_bcast: bool,
    prev: Option<String>,
    parts: BTreeMap<String, Part>,
    retry: BTreeMap<String, u64>,
    seq: u16,
    out: Vec<Out>,
}

impl Floor {
    pub fn new(t2_ms: u64) -> Self {
        Self {
            t2_ms,
            g: General::Idle,
            speaker: None,
            sprio: 0,
            revoking: None,
            rprio: 0,
            queue: Vec::new(),
            succ: false,
            rtp_seen: false,
            first_rtp_at: None,
            last_rtp_at: 0,
            c20: 0,
            t20_at: 0,
            t3_at: 0,
            c7: 0,
            t7_at: 0,
            idle_bcast: false,
            prev: None,
            parts: BTreeMap::new(),
            retry: BTreeMap::new(),
            seq: 0,
            out: Vec::new(),
        }
    }

    pub fn general(&self) -> General {
        self.g
    }

    pub fn speaker(&self) -> Option<&str> {
        match self.g {
            General::Taken => self.speaker.as_deref(),
            _ => None,
        }
    }

    pub fn holder(&self) -> Option<&str> {
        match self.g {
            General::Taken => self.speaker.as_deref(),
            General::PendingRevoke => self.revoking.as_deref(),
            General::Idle => None,
        }
    }

    pub fn priority(&self) -> Option<u8> {
        match self.g {
            General::Taken => Some(self.sprio),
            General::PendingRevoke => Some(self.rprio),
            General::Idle => None,
        }
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn queue_view(&self) -> Vec<(String, u8)> {
        self.queue.iter().map(|w| (w.user.clone(), w.priority)).collect()
    }

    pub fn participant(&self, user: &str) -> Participant {
        self.parts.get(user).map(|p| p.state).unwrap_or(self.default_part())
    }

    pub fn rx(&mut self, user: &str, rx: Rx, ctx: Ctx, members: &[String], now: u64) -> Vec<Out> {
        match rx {
            Rx::Ack => {}
            Rx::Request { .. } if !ctx.in_room => {
                self.send(user, Wire::Deny { cause: reject::OTHER, text: Some(NOT_IN_ROOM) });
            }
            _ if !ctx.in_room => {}
            Rx::Request { .. } if !ctx.has_half => {
                self.send(user, Wire::Deny { cause: reject::RECEIVE_ONLY, text: None });
            }
            Rx::Request { .. } if !ctx.in_pub_room => {
                self.send(user, Wire::Deny { cause: reject::NOT_PUB_ROOM, text: None });
            }
            Rx::Request { .. } if !ctx.permitted => {
                self.send(user, Wire::Deny { cause: reject::NO_PERMISSION, text: None });
            }
            _ => self.part_c(user, rx, members, now),
        }
        std::mem::take(&mut self.out)
    }

    pub fn rtp(&mut self, user: &str, now: u64) -> Vec<Out> {
        match self.participant(user) {
            Participant::NotPermittedIdle => {
                if self.parts.get(user).is_some_and(|p| p.released) {
                    self.revoke3(user, now);
                }
            }
            Participant::NotPermittedTaken => self.revoke3(user, now),
            Participant::Permitted | Participant::PendingRevoke => self.arb_rtp(user, now),
            Participant::NotPermittedSends => {}
        }
        std::mem::take(&mut self.out)
    }

    pub fn ready(&mut self, user: &str) -> Vec<Out> {
        if self.holder() != Some(user) {
            match self.holder().map(str::to_string) {
                Some(h) => self.send(user, Wire::Taken { speaker: h, seq: 0 }),
                None => self.send(user, Wire::Idle { prev: self.prev.clone(), seq: 0 }),
            }
        }
        std::mem::take(&mut self.out)
    }

    pub fn permission_lost(&mut self, user: &str, now: u64) -> Vec<Out> {
        if self.g == General::Taken && self.speaker.as_deref() == Some(user) {
            self.prevoke_enter(user, revoke::NO_PERMISSION, now);
        } else if self.in_queue(user) {
            self.dequeue(user);
            self.part_a(user, Arb::Deny(reject::NO_PERMISSION), now);
        }
        std::mem::take(&mut self.out)
    }

    pub fn away(&mut self, user: &str, members: &[String], now: u64) -> Vec<Out> {
        if self.g == General::Taken && self.speaker.as_deref() == Some(user) {
            self.prev = Some(user.to_string());
            self.idle_enter(members, now);
        } else if self.in_queue(user) {
            self.dequeue(user);
        }
        std::mem::take(&mut self.out)
    }

    pub fn leave(&mut self, user: &str, members: &[String], now: u64) -> Vec<Out> {
        let out = self.away(user, members, now);
        self.parts.remove(user);
        self.retry.remove(user);
        out
    }

    pub fn tick(&mut self, members: &[String], now: u64) -> Vec<Out> {
        self.retry.retain(|_, until| *until > now);
        let due: Vec<String> = self
            .parts
            .iter()
            .filter(|(_, p)| {
                matches!(p.state, Participant::PendingRevoke | Participant::NotPermittedSends)
                    && p.c8 < timers::C8
                    && now >= p.t8_at
            })
            .map(|(u, _)| u.clone())
            .collect();
        for u in due {
            let cause = match self.parts.get_mut(&u) {
                Some(p) => {
                    p.c8 += 1;
                    p.t8_at = now + timers::T8_MS;
                    p.cause
                }
                None => continue,
            };
            self.send(&u, Wire::Revoke { cause });
        }
        match self.g {
            General::Taken => {
                let speaker = self.speaker.clone().unwrap_or_default();
                if self.first_rtp_at.is_some_and(|f| now.saturating_sub(f) > self.t2_ms) {
                    self.retry.insert(speaker.clone(), now + timers::T9_MS);
                    self.prevoke_enter(&speaker, revoke::BURST_TOO_LONG, now);
                } else if now.saturating_sub(self.last_rtp_at) > timers::T1_MS {
                    self.prev = Some(speaker);
                    self.idle_enter(members, now);
                } else if self.succ && !self.rtp_seen && self.c20 <= timers::C20 && now >= self.t20_at {
                    if self.c20 < timers::C20 {
                        self.part_a(&speaker, self.granted(), now);
                    }
                    self.c20 += 1;
                    self.t20_at = now + timers::T20_MS;
                }
            }
            General::PendingRevoke => {
                if now >= self.t3_at || now.saturating_sub(self.last_rtp_at) > timers::T1_MS {
                    self.prev = self.revoking.clone();
                    self.idle_enter(members, now);
                }
            }
            General::Idle => {
                if self.idle_bcast && self.c7 < timers::C7 && now >= self.t7_at {
                    self.c7 += 1;
                    self.t7_at = now + timers::T7_MS;
                    self.fan(Arb::Idle, None, members, now);
                }
            }
        }
        std::mem::take(&mut self.out)
    }

    fn default_part(&self) -> Participant {
        match self.g {
            General::Idle => Participant::NotPermittedIdle,
            _ => Participant::NotPermittedTaken,
        }
    }

    fn part_mut(&mut self, user: &str) -> &mut Part {
        let state = self.default_part();
        self.parts.entry(user.to_string()).or_insert(Part {
            state,
            released: false,
            g_taken: state == Participant::NotPermittedTaken,
            c8: 0,
            t8_at: 0,
            cause: 0,
        })
    }

    fn enter(&mut self, user: &str, state: Participant, now: u64) {
        let p = self.part_mut(user);
        p.state = state;
        match state {
            Participant::NotPermittedIdle => p.g_taken = false,
            Participant::NotPermittedTaken => p.g_taken = true,
            Participant::Permitted => p.released = false,
            Participant::PendingRevoke | Participant::NotPermittedSends => {
                p.c8 = 1;
                p.t8_at = now + timers::T8_MS;
            }
        }
    }

    fn send(&mut self, user: &str, wire: Wire) {
        let wire = match wire {
            Wire::Taken { speaker, .. } => {
                self.seq = self.seq.wrapping_add(1);
                Wire::Taken { speaker, seq: self.seq }
            }
            Wire::Idle { prev, .. } => {
                self.seq = self.seq.wrapping_add(1);
                Wire::Idle { prev, seq: self.seq }
            }
            w => w,
        };
        self.out.push(Out { to: user.to_string(), wire });
    }

    fn part_a(&mut self, user: &str, ev: Arb, now: u64) {
        match ev {
            Arb::QueueInfo { position, priority } => {
                return self.send(user, Wire::QueueInfo { position, priority });
            }
            Arb::Deny(cause) => return self.send(user, Wire::Deny { cause, text: None }),
            _ => {}
        }
        match (self.participant(user), ev) {
            (Participant::NotPermittedIdle, Arb::Granted { priority, duration_s })
            | (Participant::NotPermittedTaken, Arb::Granted { priority, duration_s }) => {
                self.enter(user, Participant::Permitted, now);
                self.send(user, Wire::Granted { priority, duration_s });
            }
            (Participant::NotPermittedIdle, Arb::Taken(s)) => {
                self.enter(user, Participant::NotPermittedTaken, now);
                self.send(user, Wire::Taken { speaker: s, seq: 0 });
            }
            (Participant::NotPermittedIdle, Arb::Idle) => {
                self.send(user, Wire::Idle { prev: self.prev.clone(), seq: 0 });
            }
            (Participant::NotPermittedTaken, Arb::Taken(s)) => {
                self.send(user, Wire::Taken { speaker: s, seq: 0 });
            }
            (Participant::NotPermittedTaken, Arb::Idle) | (Participant::PendingRevoke, Arb::Idle) => {
                self.enter(user, Participant::NotPermittedIdle, now);
                self.send(user, Wire::Idle { prev: self.prev.clone(), seq: 0 });
            }
            (Participant::Permitted, Arb::Granted { priority, duration_s }) => {
                self.send(user, Wire::Granted { priority, duration_s });
            }
            (Participant::Permitted, Arb::Idle) => {
                self.enter(user, Participant::NotPermittedIdle, now);
            }
            (Participant::Permitted, Arb::Taken(s)) | (Participant::PendingRevoke, Arb::Taken(s)) => {
                self.enter(user, Participant::NotPermittedTaken, now);
                self.send(user, Wire::Taken { speaker: s, seq: 0 });
            }
            (Participant::Permitted, Arb::Revoke(cause)) => {
                self.enter(user, Participant::PendingRevoke, now);
                self.part_mut(user).cause = cause;
                self.send(user, Wire::Revoke { cause });
            }
            (Participant::NotPermittedSends, Arb::Idle) => self.part_mut(user).g_taken = false,
            (Participant::NotPermittedSends, Arb::Taken(_)) => self.part_mut(user).g_taken = true,
            _ => {}
        }
    }

    fn fan(&mut self, ev: Arb, skip: Option<&str>, members: &[String], now: u64) {
        for m in members {
            if Some(m.as_str()) != skip {
                self.part_a(m, ev.clone(), now);
            }
        }
    }

    fn granted(&self) -> Arb {
        Arb::Granted { priority: self.sprio, duration_s: (self.t2_ms / 1_000) as u16 }
    }

    fn in_queue(&self, user: &str) -> bool {
        self.queue.iter().any(|w| w.user == user)
    }

    fn position(&self, user: &str) -> u8 {
        self.queue.iter().position(|w| w.user == user).map(|i| (i + 1).min(253) as u8).unwrap_or(NOT_IN_QUEUE)
    }

    fn queued_priority(&self, user: &str) -> u8 {
        self.queue.iter().find(|w| w.user == user).map(|w| w.priority).unwrap_or(0)
    }

    fn dequeue(&mut self, user: &str) {
        self.queue.retain(|w| w.user != user);
    }

    fn pre_queued(&self) -> bool {
        self.queue.iter().any(|w| w.pre)
    }

    fn insert(&mut self, w: Waiter) {
        let at = self
            .queue
            .iter()
            .position(|q| (w.pre && !q.pre) || (w.pre == q.pre && w.priority > q.priority))
            .unwrap_or(self.queue.len());
        self.queue.insert(at, w);
    }

    fn holder_priority(&self) -> u8 {
        if self.g == General::Taken { self.sprio } else { self.rprio }
    }

    fn enter_taken(&mut self, members: &[String], now: u64) {
        let sp = self.speaker.clone().unwrap_or_default();
        self.idle_bcast = false;
        self.last_rtp_at = now;
        if self.succ {
            self.c20 = 1;
            self.t20_at = now + timers::T20_MS;
        }
        self.part_a(&sp, self.granted(), now);
        self.fan(Arb::Taken(sp.clone()), Some(&sp), members, now);
    }

    fn idle_enter(&mut self, members: &[String], now: u64) {
        if self.queue.is_empty() {
            self.g = General::Idle;
            self.speaker = None;
            self.revoking = None;
            self.first_rtp_at = None;
            self.idle_bcast = true;
            self.c7 = 1;
            self.t7_at = now + timers::T7_MS;
            self.fan(Arb::Idle, None, members, now);
        } else {
            let w = self.queue.remove(0);
            self.g = General::Taken;
            self.speaker = Some(w.user);
            self.sprio = w.priority;
            self.revoking = None;
            self.succ = true;
            self.rtp_seen = false;
            self.first_rtp_at = None;
            self.enter_taken(members, now);
        }
    }

    fn prevoke_enter(&mut self, user: &str, cause: u8, now: u64) {
        self.g = General::PendingRevoke;
        self.revoking = Some(user.to_string());
        self.rprio = self.sprio;
        self.speaker = None;
        self.t3_at = now + timers::T3_MS;
        self.part_a(user, Arb::Revoke(cause), now);
    }

    fn arb_req_idle(&mut self, user: &str, priority: u8, members: &[String], now: u64) {
        if self.g != General::Idle {
            return;
        }
        if members.len() <= 1 {
            self.part_a(user, Arb::Deny(reject::ONLY_ONE_PARTICIPANT), now);
        } else if self.retry.contains_key(user) {
            self.part_a(user, Arb::Deny(reject::RETRY_AFTER_NOT_EXPIRED), now);
        } else {
            self.g = General::Taken;
            self.speaker = Some(user.to_string());
            self.sprio = priority;
            self.succ = false;
            self.rtp_seen = false;
            self.first_rtp_at = None;
            self.c20 = 0;
            self.enter_taken(members, now);
        }
    }

    fn arb_req_permitted(&mut self, user: &str, now: u64) {
        if self.g == General::Taken && self.speaker.as_deref() == Some(user) {
            let duration_s = match self.first_rtp_at {
                Some(f) => (self.t2_ms.saturating_sub(now.saturating_sub(f)) / 1_000) as u16,
                None => (self.t2_ms / 1_000) as u16,
            };
            self.part_a(user, Arb::Granted { priority: self.sprio, duration_s }, now);
        }
    }

    fn arb_preempt(&mut self, user: &str, priority: u8, now: u64) {
        if self.g != General::Taken {
            return;
        }
        let old = self.speaker.clone().unwrap_or_default();
        self.queue.insert(0, Waiter { user: user.to_string(), priority, pre: true });
        self.prevoke_enter(&old, revoke::PREEMPTED, now);
        self.part_a(user, Arb::QueueInfo { position: 1, priority }, now);
    }

    fn arb_rel(&mut self, user: &str, members: &[String], now: u64) {
        let ours = match self.g {
            General::Taken => self.speaker.as_deref() == Some(user),
            General::PendingRevoke => self.revoking.as_deref() == Some(user),
            General::Idle => false,
        };
        if ours {
            self.prev = Some(user.to_string());
            self.idle_enter(members, now);
        }
    }

    fn arb_rtp(&mut self, user: &str, now: u64) {
        match self.g {
            General::Taken if self.speaker.as_deref() == Some(user) => {
                self.rtp_seen = true;
                self.first_rtp_at.get_or_insert(now);
                self.last_rtp_at = now;
            }
            General::PendingRevoke if self.revoking.as_deref() == Some(user) => self.last_rtp_at = now,
            _ => {}
        }
    }

    fn taken_req(&mut self, user: &str, priority: u8, now: u64) {
        if self.retry.contains_key(user) {
            self.send(user, Wire::Deny { cause: reject::RETRY_AFTER_NOT_EXPIRED, text: None });
        } else if self.in_queue(user) {
            let (position, priority) = (self.position(user), self.queued_priority(user));
            self.send(user, Wire::QueueInfo { position, priority });
        } else if priority > self.holder_priority() && !self.pre_queued() {
            self.arb_preempt(user, priority, now);
        } else if priority > self.holder_priority() {
        } else if self.queue.len() >= QUEUE_MAX {
            self.send(user, Wire::Deny { cause: reject::QUEUE_FULL, text: None });
        } else {
            self.insert(Waiter { user: user.to_string(), priority, pre: false });
            let position = self.position(user);
            self.send(user, Wire::QueueInfo { position, priority });
        }
    }

    fn revoke3(&mut self, user: &str, now: u64) {
        self.enter(user, Participant::NotPermittedSends, now);
        self.part_mut(user).cause = revoke::NO_PERMISSION;
        self.send(user, Wire::Revoke { cause: revoke::NO_PERMISSION });
    }

    fn part_c(&mut self, user: &str, rx: Rx, members: &[String], now: u64) {
        match (self.participant(user), rx) {
            (Participant::NotPermittedIdle, Rx::Request { priority }) => {
                self.arb_req_idle(user, priority, members, now)
            }
            (Participant::NotPermittedIdle, Rx::Release) => {
                self.dequeue(user);
                self.send(user, Wire::Idle { prev: self.prev.clone(), seq: 0 });
            }
            (Participant::NotPermittedTaken, Rx::Request { priority }) => self.taken_req(user, priority, now),
            (Participant::NotPermittedTaken, Rx::Release) => {
                self.dequeue(user);
                let h = self.holder().unwrap_or_default().to_string();
                self.send(user, Wire::Taken { speaker: h, seq: 0 });
            }
            (Participant::NotPermittedTaken, Rx::QueuePos) => {
                let (position, priority) = (self.position(user), self.queued_priority(user));
                self.send(user, Wire::QueueInfo { position, priority });
            }
            (Participant::Permitted, Rx::Release) => {
                self.part_mut(user).released = true;
                self.arb_rel(user, members, now);
            }
            (Participant::Permitted, Rx::Request { .. }) => self.arb_req_permitted(user, now),
            (Participant::PendingRevoke, Rx::Release) => self.arb_rel(user, members, now),
            (Participant::NotPermittedSends, Rx::Release) => {
                if self.parts.get(user).is_some_and(|p| p.g_taken) {
                    self.enter(user, Participant::NotPermittedTaken, now);
                    let h = self.holder().unwrap_or_default().to_string();
                    self.send(user, Wire::Taken { speaker: h, seq: 0 });
                } else {
                    self.enter(user, Participant::NotPermittedIdle, now);
                    self.send(user, Wire::Idle { prev: self.prev.clone(), seq: 0 });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Ctx = Ctx { in_room: true, has_half: true, in_pub_room: true, permitted: true };

    fn m(us: &[&str]) -> Vec<String> {
        us.iter().map(|u| u.to_string()).collect()
    }

    fn wires(out: &[Out], to: &str) -> Vec<Wire> {
        out.iter().filter(|o| o.to == to).map(|o| o.wire.clone()).collect()
    }

    fn req(f: &mut Floor, who: &str, prio: u8, ms: &[String], now: u64) -> Vec<Out> {
        f.rx(who, Rx::Request { priority: prio }, ON, ms, now)
    }

    #[test]
    fn idle_request_grants_and_tells_others() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        let out = req(&mut f, "a", 0, &ms, 0);
        assert_eq!(wires(&out, "a"), vec![Wire::Granted { priority: 0, duration_s: 30 }]);
        assert_eq!(wires(&out, "b"), vec![Wire::Taken { speaker: "a".into(), seq: 1 }]);
        assert_eq!(f.speaker(), Some("a"));
        assert_eq!(f.participant("a"), Participant::Permitted);
    }

    #[test]
    fn alone_is_denied_only_in_idle() {
        let mut f = Floor::new(timers::T2_MS);
        let out = req(&mut f, "a", 0, &m(&["a"]), 0);
        assert_eq!(wires(&out, "a"), vec![Wire::Deny { cause: reject::ONLY_ONE_PARTICIPANT, text: None }]);
    }

    #[test]
    fn gates_run_before_the_state_machine() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        let cases = [
            (Ctx { in_room: false, ..ON }, Wire::Deny { cause: reject::OTHER, text: Some(NOT_IN_ROOM) }),
            (Ctx { has_half: false, ..ON }, Wire::Deny { cause: reject::RECEIVE_ONLY, text: None }),
            (Ctx { in_pub_room: false, ..ON }, Wire::Deny { cause: reject::NOT_PUB_ROOM, text: None }),
            (Ctx { permitted: false, ..ON }, Wire::Deny { cause: reject::NO_PERMISSION, text: None }),
        ];
        for (ctx, want) in cases {
            let out = f.rx("a", Rx::Request { priority: 0 }, ctx, &ms, 0);
            assert_eq!(wires(&out, "a"), vec![want]);
        }
        assert!(f.rx("a", Rx::Ack, ON, &ms, 0).is_empty());
        assert!(f.rx("a", Rx::Release, Ctx { in_room: false, ..ON }, &ms, 0).is_empty());
    }

    #[test]
    fn release_by_speaker_sends_no_idle_to_speaker() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        let out = f.rx("a", Rx::Release, ON, &ms, 10);
        assert!(wires(&out, "a").is_empty());
        assert_eq!(wires(&out, "b"), vec![Wire::Idle { prev: Some("a".into()), seq: 2 }]);
        let again = f.rx("a", Rx::Release, ON, &ms, 600);
        assert_eq!(wires(&again, "a"), vec![Wire::Idle { prev: Some("a".into()), seq: 3 }]);
    }

    #[test]
    fn queue_then_succession_without_idle() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        let q = req(&mut f, "b", 0, &ms, 10);
        assert_eq!(wires(&q, "b"), vec![Wire::QueueInfo { position: 1, priority: 0 }]);
        let out = f.rx("a", Rx::Release, ON, &ms, 20);
        assert_eq!(wires(&out, "b"), vec![Wire::Granted { priority: 0, duration_s: 30 }]);
        assert_eq!(wires(&out, "a"), vec![Wire::Taken { speaker: "b".into(), seq: 2 }]);
        assert!(out.iter().all(|o| !matches!(o.wire, Wire::Idle { .. })));
    }

    #[test]
    fn preempt_revokes_with_grace_then_succeeds() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        let out = req(&mut f, "b", 1, &ms, 10);
        assert_eq!(wires(&out, "a"), vec![Wire::Revoke { cause: revoke::PREEMPTED }]);
        assert_eq!(wires(&out, "b"), vec![Wire::QueueInfo { position: 1, priority: 1 }]);
        assert_eq!(f.general(), General::PendingRevoke);
        assert_eq!(f.holder(), Some("a"));
        let done = f.rx("a", Rx::Release, ON, &ms, 50);
        assert_eq!(wires(&done, "b"), vec![Wire::Granted { priority: 1, duration_s: 30 }]);
        assert_eq!(f.speaker(), Some("b"));
    }

    #[test]
    fn preempt_during_grace_is_discarded() {
        let ms = m(&["a", "b", "c"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        req(&mut f, "b", 1, &ms, 10);
        let out = req(&mut f, "c", 2, &ms, 20);
        assert!(out.is_empty());
        assert_eq!(f.queue_view(), vec![("b".to_string(), 1)]);
    }

    #[test]
    fn t3_closes_grace_when_release_never_comes() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        req(&mut f, "b", 1, &ms, 10);
        f.rtp("a", 1_000);
        let resend = f.tick(&ms, 1_010);
        assert_eq!(wires(&resend, "a"), vec![Wire::Revoke { cause: revoke::PREEMPTED }]);
        assert_eq!(wires(&f.tick(&ms, 2_010), "a"), vec![Wire::Revoke { cause: revoke::PREEMPTED }]);
        let out = f.tick(&ms, 10 + timers::T3_MS);
        assert_eq!(f.speaker(), Some("b"));
        assert_eq!(wires(&out, "a"), vec![Wire::Taken { speaker: "b".into(), seq: 2 }]);
    }

    #[test]
    fn unpermitted_rtp_in_taken_is_revoked_unconditionally() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        let out = f.rtp("b", 100);
        assert_eq!(wires(&out, "b"), vec![Wire::Revoke { cause: revoke::NO_PERMISSION }]);
        assert_eq!(f.participant("b"), Participant::NotPermittedSends);
        let rel = f.rx("b", Rx::Release, ON, &ms, 200);
        assert_eq!(wires(&rel, "b"), vec![Wire::Taken { speaker: "a".into(), seq: 2 }]);
    }

    #[test]
    fn rtp_in_idle_is_revoked_only_after_release() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        f.tick(&ms, timers::T1_MS + 1);
        assert_eq!(f.general(), General::Idle);
        assert!(f.rtp("a", timers::T1_MS + 2).is_empty());
        req(&mut f, "b", 0, &ms, 10_000);
        f.rx("b", Rx::Release, ON, &ms, 10_100);
        let out = f.rtp("b", 10_200);
        assert_eq!(wires(&out, "b"), vec![Wire::Revoke { cause: revoke::NO_PERMISSION }]);
    }

    #[test]
    fn t2_revokes_from_first_rtp_and_arms_retry_after() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        for t in (5_000..=35_000).step_by(1_000) {
            f.rtp("a", t);
        }
        let out = f.tick(&ms, 35_001);
        assert_eq!(wires(&out, "a"), vec![Wire::Revoke { cause: revoke::BURST_TOO_LONG }]);
        f.rx("a", Rx::Release, ON, &ms, 35_050);
        let deny = req(&mut f, "a", 0, &ms, 35_100);
        assert_eq!(wires(&deny, "a"), vec![Wire::Deny { cause: reject::RETRY_AFTER_NOT_EXPIRED, text: None }]);
        f.tick(&ms, 35_001 + timers::T9_MS + 1);
        let ok = req(&mut f, "a", 0, &ms, 40_000);
        assert_eq!(wires(&ok, "a"), vec![Wire::Granted { priority: 0, duration_s: 30 }]);
    }

    #[test]
    fn regrant_carries_remaining_t2() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        f.rtp("a", 1_000);
        let out = req(&mut f, "a", 0, &ms, 11_000);
        assert_eq!(wires(&out, "a"), vec![Wire::Granted { priority: 0, duration_s: 20 }]);
    }

    #[test]
    fn queue_position_answers_regardless_of_membership_while_taken() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        assert!(f.rx("b", Rx::QueuePos, ON, &ms, 0).is_empty());
        req(&mut f, "a", 0, &ms, 0);
        let out = f.rx("b", Rx::QueuePos, ON, &ms, 10);
        assert_eq!(wires(&out, "b"), vec![Wire::QueueInfo { position: NOT_IN_QUEUE, priority: 0 }]);
    }

    #[test]
    fn t20_resends_succession_grant_until_rtp() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        req(&mut f, "b", 0, &ms, 10);
        f.rx("a", Rx::Release, ON, &ms, 20);
        let r1 = f.tick(&ms, 20 + timers::T20_MS);
        assert_eq!(wires(&r1, "b"), vec![Wire::Granted { priority: 0, duration_s: 30 }]);
        f.rtp("b", 1_500);
        assert!(wires(&f.tick(&ms, 20 + 2 * timers::T20_MS), "b").is_empty());
    }

    #[test]
    fn permission_lost_revokes_speaker_and_denies_waiter() {
        let ms = m(&["a", "b", "c"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        req(&mut f, "b", 0, &ms, 10);
        let w = f.permission_lost("b", 20);
        assert_eq!(wires(&w, "b"), vec![Wire::Deny { cause: reject::NO_PERMISSION, text: None }]);
        let s = f.permission_lost("a", 30);
        assert_eq!(wires(&s, "a"), vec![Wire::Revoke { cause: revoke::NO_PERMISSION }]);
        assert_eq!(f.general(), General::PendingRevoke);
    }

    #[test]
    fn away_releases_speaker_but_not_the_revoked() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        let out = f.away("a", &ms, 10);
        assert_eq!(f.general(), General::Idle);
        assert_eq!(wires(&out, "b"), vec![Wire::Idle { prev: Some("a".into()), seq: 2 }]);
        req(&mut f, "a", 0, &ms, 100);
        req(&mut f, "b", 1, &ms, 110);
        assert!(f.away("a", &ms, 120).is_empty());
        assert_eq!(f.general(), General::PendingRevoke);
    }

    #[test]
    fn ready_tells_current_holder_once() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        assert_eq!(wires(&f.ready("b"), "b"), vec![Wire::Idle { prev: None, seq: 1 }]);
        req(&mut f, "a", 0, &ms, 0);
        assert_eq!(wires(&f.ready("b"), "b"), vec![Wire::Taken { speaker: "a".into(), seq: 3 }]);
        assert!(f.ready("a").is_empty());
    }

    #[test]
    fn idle_is_resent_by_t7_up_to_c7() {
        let ms = m(&["a", "b"]);
        let mut f = Floor::new(timers::T2_MS);
        req(&mut f, "a", 0, &ms, 0);
        f.rx("a", Rx::Release, ON, &ms, 10);
        let mut sent = 0;
        for k in 1..20u64 {
            sent += wires(&f.tick(&ms, 10 + k * timers::T7_MS), "b").len();
        }
        assert_eq!(sent, (timers::C7 - 1) as usize);
    }
}
