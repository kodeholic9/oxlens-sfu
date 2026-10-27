// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:FLR.S · 연:RULE-FLR-SERVER-STATE-SEPARATE · 연:RULE-FLR-TIMERS · 연:RULE-FLR-OWN-RULES · 연:RULE-FLR-PREEMPT · 연:RULE-FLR-SEQ · 연:RULE-FLR-JOIN-NOTIFY · 연:RULE-FLR-PERM-REVOKE-ENFORCE · 연:RULE-FLR-SERVER-EVENTS · 연:RULE-FLR-MSG-REQUEST · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-POLICY · 정:IMPL-FLR-MEMBER-JOIN · 정:CAND-FLR-MEMBER-JOIN-INIT · 정:RULE-CONC-SANS-IO · 연:RULE-FLR-T9-VALUE · 연:DEV-FLR-T21-SAME-SEQ · 연:DEV-FLR-GRANTED-SEQ · 연:DEV-FLR-TAKEN-ACK · 연:DEV-FLR-PUBROOM · 연:DEV-FLR-RECVONLY-TRIGGER · 연:DEV-FLR-PERM-REVOKE · 연:DEV-FLR-PREEMPT-RELATIVE · 연:DEV-FLR-T9 · 연:DEV-FLR-QUEUE-MAX · 연:DEV-FLR-FIRST-NOTIFY · 연:DEV-FLR-PUB-AWAY · 연:DEV-FLR-T7-VALUE · 연:DEV-FLR-DENY-101 · 연:DEV-FLR-DENY-102 · 연:DEV-FLR-DENY-255 · model: claude-opus-5-5
//! 발언권 판정 로직(순수) — 3GPP TS 24.380 §6.3.4 판정 로직 · §6.3.5 참가자 상태기. 연동규격서 원천 서버 판정(`protocol/out/floor/server/floor_server.ts`)과 같은 걸음 · 같은 라벨. 보낼 것 · 효과 · 지난 라벨을 돌려주고 시계는 부르는 쪽이 쥔다.

use std::collections::{BTreeMap, BTreeSet};

pub const SEQ_WRAP: u32 = 65_536;

pub mod deny {
    pub const ONLY_ONE: u32 = 3;
    pub const RETRY_AFTER: u32 = 4;
    pub const RECV_ONLY: u32 = 5;
    pub const QUEUE_FULL: u32 = 7;
    pub const NOT_PUBROOM: u32 = 101;
    pub const NO_PERMISSION: u32 = 102;
    pub const OTHER_REASON: u32 = 255;
}

pub mod revoke {
    pub const TOO_LONG: u32 = 2;
    pub const NO_PERMISSION: u32 = 3;
    pub const PREEMPTED: u32 = 4;
}

pub const QPOS_NOT_IN_QUEUE: u32 = 254;

pub mod ms {
    pub const T1: u64 = 4_000;
    pub const T3: u64 = 3_000;
    pub const T7: u64 = 1_000;
    pub const T8: u64 = 1_000;
    pub const T9: u64 = 3_000;
    pub const T20: u64 = 1_000;
    pub const T21: u64 = 1_000;
}

pub const C7: u32 = 10;
pub const C8: u32 = 3;
pub const C20: u32 = 3;
pub const C21: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G {
    Idle,
    Taken,
    Prevoke,
}

impl G {
    pub fn as_str(self) -> &'static str {
        match self {
            G::Idle => "IDLE",
            G::Taken => "TAKEN",
            G::Prevoke => "PREVOKE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    NpIdle,
    NpTaken,
    Permitted,
    Prevoke,
    NpSends,
    Releasing,
}

impl Part {
    pub fn as_str(self) -> &'static str {
        match self {
            Part::NpIdle => "NP_IDLE",
            Part::NpTaken => "NP_TAKEN",
            Part::Permitted => "PERMITTED",
            Part::Prevoke => "PREVOKE",
            Part::NpSends => "NPSENDS",
            Part::Releasing => "RELEASING",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T {
    Request,
    Granted,
    Taken,
    Deny,
    Release,
    Idle,
    Revoke,
    QposReq,
    Qinfo,
    Ack,
    Rtp,
}

impl T {
    pub fn wire(self) -> Option<u8> {
        Some(match self {
            T::Request => 0,
            T::Granted => 1,
            T::Taken => 2,
            T::Deny => 3,
            T::Release => 4,
            T::Idle => 5,
            T::Revoke => 6,
            T::QposReq => 8,
            T::Qinfo => 9,
            T::Ack => 10,
            T::Rtp => return None,
        })
    }

    pub fn from_wire(t: u8) -> Option<T> {
        Some(match t {
            0 => T::Request,
            1 => T::Granted,
            2 => T::Taken,
            3 => T::Deny,
            4 => T::Release,
            5 => T::Idle,
            6 => T::Revoke,
            8 => T::QposReq,
            9 => T::Qinfo,
            10 => T::Ack,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            T::Request => "REQUEST",
            T::Granted => "GRANTED",
            T::Taken => "TAKEN",
            T::Deny => "DENY",
            T::Release => "RELEASE",
            T::Idle => "IDLE",
            T::Revoke => "REVOKE",
            T::QposReq => "QPOS_REQ",
            T::Qinfo => "QINFO",
            T::Ack => "ACK",
            T::Rtp => "RTP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub t: T,
    pub a: Option<String>,
    pub n: u32,
    pub d: u32,
    pub q: u16,
}

impl Msg {
    pub fn new(t: T, a: Option<String>, n: u32, d: u32) -> Msg {
        Msg { t, a, n, d, q: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub to: String,
    pub m: Msg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fx {
    Bind(String),
    Release(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Timer {
    T1,
    T2,
    T3,
    T7,
    T20,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PTimer {
    T8,
    T9,
    T21,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub pub_room: bool,
    pub half: bool,
    pub perm: bool,
    pub retry: bool,
    pub part: Part,
    pub released: bool,
    pub g_taken: bool,
    pub c8: u32,
    pub c21: u32,
    pub tq: u16,
    pub rcause: u32,
}

impl Person {
    fn fresh(half: bool, perm: bool, taken: bool) -> Person {
        Person { pub_room: false, half, perm, retry: false, part: if taken { Part::NpTaken } else { Part::NpIdle }, released: false, g_taken: taken, c8: 0, c21: 0, tq: 0, rcause: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Srv {
    pub g: G,
    pub speaker: Option<String>,
    pub sprio: u32,
    pub revoking: Option<String>,
    pub rprio: u32,
    pub rcause_g: u32,
    pub queue: Vec<Waiter>,
    pub succ: bool,
    pub rtp_seen: bool,
    pub first_rtp: bool,
    pub c20: u32,
    pub c7: u32,
    pub idle_bcast: bool,
    pub prev: Option<String>,
    pub members: BTreeSet<String>,
    pub people: BTreeMap<String, Person>,
    pub seq: u16,
    pub restarted: Vec<Timer>,
    pub restarted_p: Vec<(String, PTimer)>,
}

impl Default for Srv {
    fn default() -> Srv {
        Srv {
            g: G::Idle,
            speaker: None,
            sprio: 0,
            revoking: None,
            rprio: 0,
            rcause_g: 0,
            queue: Vec::new(),
            succ: false,
            rtp_seen: false,
            first_rtp: false,
            c20: 0,
            c7: 0,
            idle_bcast: false,
            prev: None,
            members: BTreeSet::new(),
            people: BTreeMap::new(),
            seq: 0,
            restarted: Vec::new(),
            restarted_p: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Env {
    pub qmax: usize,
    pub t2_secs: u32,
    pub t2_remain_secs: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct R {
    pub out: Vec<Out>,
    pub labels: Vec<&'static str>,
    pub fx: Vec<Fx>,
}

struct Ctx<'a> {
    s: &'a mut Srv,
    r: R,
    env: Env,
}

impl Srv {
    pub fn new() -> Srv {
        Srv::default()
    }

    pub fn holder(&self) -> Option<&String> {
        match self.g {
            G::Taken => self.speaker.as_ref(),
            G::Prevoke => self.revoking.as_ref(),
            G::Idle => None,
        }
    }

    fn holder_prio(&self) -> u32 {
        if self.g == G::Taken { self.sprio } else { self.rprio }
    }

    pub fn part(&self, u: &str) -> Option<Part> {
        self.people.get(u).map(|p| p.part)
    }

    fn in_q(&self, u: &str) -> bool {
        self.queue.iter().any(|w| w.u == u)
    }

    fn pos(&self, u: &str) -> u32 {
        self.queue.iter().position(|w| w.u == u).map(|i| i as u32 + 1).unwrap_or(QPOS_NOT_IN_QUEUE)
    }

    fn qprio(&self, u: &str) -> u32 {
        self.queue.iter().find(|w| w.u == u).map(|w| w.p).unwrap_or(0)
    }

    fn pre_q(&self) -> bool {
        self.queue.iter().any(|w| w.pre)
    }

    pub fn armed(&self) -> Vec<Timer> {
        let mut on = Vec::new();
        if self.g == G::Taken || self.g == G::Prevoke {
            on.push(Timer::T1);
        }
        if self.g == G::Taken && self.first_rtp {
            on.push(Timer::T2);
        }
        if self.g == G::Prevoke {
            on.push(Timer::T3);
        }
        if self.g == G::Idle && self.idle_bcast && self.c7 < C7 {
            on.push(Timer::T7);
        }
        if self.g == G::Taken && self.succ && !self.rtp_seen && self.c20 <= C20 {
            on.push(Timer::T20);
        }
        on
    }

    pub fn armed_p(&self, u: &str) -> Vec<PTimer> {
        let mut on = Vec::new();
        let Some(p) = self.people.get(u) else { return on };
        if (p.part == Part::Prevoke || p.part == Part::NpSends) && p.c8 < C8 {
            on.push(PTimer::T8);
        }
        if p.retry {
            on.push(PTimer::T9);
        }
        if p.part == Part::NpTaken && p.c21 >= 1 && p.c21 <= C21 {
            on.push(PTimer::T21);
        }
        on
    }
}

impl Ctx<'_> {
    fn label(&mut self, l: &'static str) {
        self.r.labels.push(l);
    }

    fn person(&mut self, u: &str) -> &mut Person {
        self.s.people.entry(u.to_string()).or_insert_with(|| Person::fresh(false, false, false))
    }

    fn part(&self, u: &str) -> Option<Part> {
        self.s.part(u)
    }

    fn bump(&mut self) -> u16 {
        self.s.seq = ((u32::from(self.s.seq) + 1) % SEQ_WRAP) as u16;
        self.s.seq
    }

    fn send(&mut self, u: &str, t: T, a: Option<String>, n: u32, d: u32) {
        let seqd = t == T::Taken || t == T::Idle;
        let bump = seqd && d == 0;
        let q = if bump {
            self.bump()
        } else if seqd {
            d as u16
        } else {
            0
        };
        self.r.out.push(Out { to: u.to_string(), m: Msg { t, a, n, d: if seqd { 0 } else { d }, q } });
    }

    fn send_q(&mut self, u: &str, t: T, a: Option<String>, n: u32, d: u32, q: u16) {
        self.r.out.push(Out { to: u.to_string(), m: Msg { t, a, n, d, q } });
    }

    fn send_e(&mut self, u: &str, p: Part, t: T, a: Option<String>, n: u32, d: u32) {
        self.enter(u, p);
        self.send(u, t, a, n, d);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_eq(&mut self, u: &str, p: Part, t: T, a: Option<String>, n: u32, d: u32, q: u16) {
        self.enter(u, p);
        self.send_q(u, t, a, n, d, q);
    }

    fn fan(&mut self, ev: Msg, skip: Option<&str>) {
        let rest: Vec<String> = self.s.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
        let mut ev = ev;
        if ev.t == T::Taken || ev.t == T::Idle {
            ev.q = self.bump();
        }
        for u in rest {
            self.part_a(&u, &ev);
        }
    }

    fn dequeue(&mut self, u: &str) {
        self.s.queue.retain(|w| w.u != u);
    }

    fn insert_q(&mut self, w: Waiter) {
        let before = |w: &Waiter, x: &Waiter| (w.pre && !x.pre) || (w.pre == x.pre && w.p > x.p);
        match self.s.queue.iter().position(|x| before(&w, x)) {
            Some(i) => self.s.queue.insert(i, w),
            None => self.s.queue.push(w),
        }
    }

    fn restart(&mut self, t: Timer) {
        self.s.restarted.push(t);
    }

    fn restart_p(&mut self, u: &str, t: PTimer) {
        self.s.restarted_p.push((u.to_string(), t));
    }

    fn t2_left(&self) -> u32 {
        if self.s.first_rtp { self.env.t2_remain_secs } else { self.env.t2_secs }
    }

    fn enter_taken(&mut self) {
        self.label("FLR.S.G.TAKEN.enter");
        self.s.idle_bcast = false;
        self.restart(Timer::T1);
        let sp = self.s.speaker.clone().unwrap_or_default();
        let ev = Msg { q: self.s.seq, ..Msg::new(T::Granted, None, self.s.sprio, self.env.t2_secs) };
        self.part_a(&sp, &ev);
        self.fan(Msg::new(T::Taken, Some(sp.clone()), 0, 0), Some(&sp));
        self.r.fx.push(Fx::Bind(sp));
    }

    fn idle_enter(&mut self) {
        if self.s.queue.is_empty() {
            self.label("FLR.S.G.IDLE.enter_empty");
            self.s.g = G::Idle;
            self.s.speaker = None;
            self.s.revoking = None;
            self.s.first_rtp = false;
            self.s.idle_bcast = true;
            self.s.c7 = 1;
            self.restart(Timer::T7);
            let prev = self.s.prev.clone();
            self.fan(Msg::new(T::Idle, prev.clone(), 0, 0), None);
            self.r.fx.push(Fx::Release(prev));
            return;
        }
        self.label("FLR.S.G.IDLE.enter_succession");
        let w = self.s.queue.remove(0);
        self.s.g = G::Taken;
        self.s.speaker = Some(w.u);
        self.s.sprio = w.p;
        self.s.revoking = None;
        self.s.succ = true;
        self.s.rtp_seen = false;
        self.s.first_rtp = false;
        self.s.c20 = 1;
        self.restart(Timer::T20);
        self.enter_taken();
    }

    fn prevoke_enter(&mut self, u: &str, cause: u32) {
        self.label("FLR.S.G.PREVOKE.enter");
        self.s.g = G::Prevoke;
        self.s.revoking = Some(u.to_string());
        self.s.rprio = self.s.sprio;
        self.s.rcause_g = cause;
        self.s.speaker = None;
        self.restart(Timer::T3);
        self.part_a(u, &Msg::new(T::Revoke, None, cause, 0));
    }

    fn arb_req_idle(&mut self, u: &str, p: u32) {
        if self.s.g != G::Idle {
            self.label("FLR.S.G.any.req_discard");
            return;
        }
        if self.s.members.len() <= 1 {
            self.label("FLR.S.G.IDLE.req_alone");
            self.part_a(u, &Msg::new(T::Deny, None, deny::ONLY_ONE, 0));
            return;
        }
        if self.s.people.get(u).is_some_and(|x| x.retry) {
            self.label("FLR.S.G.IDLE.req_retry_after");
            self.part_a(u, &Msg::new(T::Deny, None, deny::RETRY_AFTER, 0));
            return;
        }
        self.label("FLR.S.G.IDLE.req_grant");
        self.s.g = G::Taken;
        self.s.speaker = Some(u.to_string());
        self.s.sprio = p;
        self.s.succ = false;
        self.s.rtp_seen = false;
        self.s.first_rtp = false;
        self.s.c20 = 0;
        self.enter_taken();
    }

    fn arb_req_permitted(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.label("FLR.S.G.TAKEN.req_regrant");
            let ev = Msg { q: self.s.seq, ..Msg::new(T::Granted, None, self.s.sprio, self.t2_left()) };
            self.part_a(u, &ev);
            return;
        }
        self.label("FLR.S.G.any.regrant_discard");
    }

    fn arb_preempt(&mut self, u: &str, p: u32) {
        if self.s.g == G::Taken {
            self.label("FLR.S.G.TAKEN.preempt");
            self.s.queue.insert(0, Waiter { u: u.to_string(), p, pre: true });
            let sp = self.s.speaker.clone().unwrap_or_default();
            self.prevoke_enter(&sp, revoke::PREEMPTED);
            self.part_a(u, &Msg::new(T::Qinfo, None, 1, p));
            return;
        }
        self.label("FLR.S.G.PREVOKE.preempt_discard");
    }

    fn arb_rel(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.label("FLR.S.G.TAKEN.rel_speaker");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
            self.label("FLR.S.G.PREVOKE.rel_revoked");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        self.label("FLR.S.G.any.rel_discard");
    }

    fn arb_rtp(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.label("FLR.S.G.TAKEN.rtp");
            if !self.s.first_rtp {
                self.restart(Timer::T2);
            }
            self.s.rtp_seen = true;
            self.s.first_rtp = true;
            return;
        }
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
            self.label("FLR.S.G.PREVOKE.rtp");
            return;
        }
        self.label("FLR.S.G.any.rtp_discard");
    }

    fn part_a(&mut self, u: &str, ev: &Msg) {
        let part = self.part(u);
        match (part, ev.t) {
            (_, T::Qinfo) => {
                self.label("FLR.S.P.any.a_QINFO");
                self.send(u, T::Qinfo, None, ev.n, ev.d);
            }
            (_, T::Deny) => {
                self.label("FLR.S.P.any.a_DENY");
                self.send(u, T::Deny, None, ev.n, 0);
            }
            (Some(Part::NpIdle), T::Granted) => {
                self.label("FLR.S.P.NP_IDLE.a_GRANTED");
                self.send_eq(u, Part::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (Some(Part::NpIdle), T::Taken) => {
                self.label("FLR.S.P.NP_IDLE.a_TAKEN");
                self.person(u).tq = ev.q;
                self.send_e(u, Part::NpTaken, T::Taken, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::NpIdle), T::Idle) => {
                self.label("FLR.S.P.NP_IDLE.a_IDLE");
                self.send(u, T::Idle, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::NpIdle), _) => self.label("FLR.S.P.NP_IDLE.a_other"),
            (Some(Part::NpTaken), T::Granted) => {
                self.label("FLR.S.P.NP_TAKEN.a_GRANTED");
                self.send_eq(u, Part::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (Some(Part::NpTaken), T::Taken) => {
                self.label("FLR.S.P.NP_TAKEN.a_TAKEN");
                let p = self.person(u);
                p.tq = ev.q;
                p.c21 = 1;
                self.restart_p(u, PTimer::T21);
                self.send(u, T::Taken, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::NpTaken), T::Idle) => {
                self.label("FLR.S.P.NP_TAKEN.a_IDLE");
                self.send_e(u, Part::NpIdle, T::Idle, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::NpTaken), _) => self.label("FLR.S.P.NP_TAKEN.a_other"),
            (Some(Part::Permitted), T::Granted) => {
                self.label("FLR.S.P.PERMITTED.a_GRANTED");
                self.send_q(u, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (Some(Part::Permitted), T::Idle) => {
                self.label("FLR.S.P.PERMITTED.a_IDLE");
                self.enter(u, Part::NpIdle);
            }
            (Some(Part::Permitted), T::Taken) => {
                self.label("FLR.S.P.PERMITTED.a_TAKEN");
                self.person(u).tq = ev.q;
                self.send_e(u, Part::NpTaken, T::Taken, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::Permitted), T::Revoke) => {
                self.label("FLR.S.P.PERMITTED.a_REVOKE");
                self.person(u).rcause = ev.n;
                self.send_e(u, Part::Prevoke, T::Revoke, None, ev.n, 0);
            }
            (Some(Part::Permitted), _) => self.label("FLR.S.P.PERMITTED.a_other"),
            (Some(Part::Prevoke), T::Idle) => {
                self.label("FLR.S.P.PREVOKE.a_IDLE");
                self.send_e(u, Part::NpIdle, T::Idle, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::Prevoke), T::Taken) => {
                self.label("FLR.S.P.PREVOKE.a_TAKEN");
                self.person(u).tq = ev.q;
                self.send_e(u, Part::NpTaken, T::Taken, ev.a.clone(), 0, u32::from(ev.q));
            }
            (Some(Part::Prevoke), _) => self.label("FLR.S.P.PREVOKE.a_other"),
            (Some(Part::Releasing), _) => self.label("FLR.S.P.RELEASING.a_any"),
            (_, T::Idle) => {
                self.label("FLR.S.P.NPSENDS.a_IDLE");
                self.person(u).g_taken = false;
            }
            (_, T::Taken) => {
                self.label("FLR.S.P.NPSENDS.a_TAKEN");
                self.person(u).g_taken = true;
            }
            _ => self.label("FLR.S.P.NPSENDS.a_other"),
        }
    }

    fn part_c(&mut self, u: &str, ev: &Msg) {
        let part = self.part(u);
        let released = self.s.people.get(u).is_some_and(|p| p.released);
        match (part, ev.t) {
            (Some(Part::NpIdle), T::Request) => {
                self.label("FLR.S.P.NP_IDLE.rx_REQUEST");
                self.arb_req_idle(u, ev.n);
            }
            (Some(Part::NpIdle), T::Release) => {
                self.label("FLR.S.P.NP_IDLE.rx_RELEASE");
                self.dequeue(u);
                let prev = self.s.prev.clone();
                self.send(u, T::Idle, prev, 0, 0);
            }
            (Some(Part::NpIdle), T::Rtp) if released => {
                self.label("FLR.S.P.NP_IDLE.rx_RTP_after_release");
                self.revoke3(u);
            }
            (Some(Part::NpIdle), T::Rtp) => self.label("FLR.S.P.NP_IDLE.rx_RTP_drop"),
            (Some(Part::NpIdle), _) => self.label("FLR.S.P.NP_IDLE.rx_other"),
            (Some(Part::NpTaken), T::Request) => {
                self.label("FLR.S.P.NP_TAKEN.rx_REQUEST");
                self.taken_req(u, ev.n);
            }
            (Some(Part::NpTaken), T::Release) => {
                self.label("FLR.S.P.NP_TAKEN.rx_RELEASE");
                let tq = ((u32::from(self.s.seq) + 1) % SEQ_WRAP) as u16;
                let p = self.person(u);
                p.tq = tq;
                p.c21 = 1;
                self.restart_p(u, PTimer::T21);
                self.dequeue(u);
                let h = self.s.holder().cloned();
                self.send(u, T::Taken, h, 0, 0);
            }
            (Some(Part::NpTaken), T::Ack) if ev.n == 2 && self.s.people.get(u).is_some_and(|p| p.tq == ev.q) => {
                self.label("FLR.S.P.NP_TAKEN.rx_ACK_taken");
                self.person(u).c21 = 0;
            }
            (Some(Part::NpTaken), T::Ack) if ev.n == 2 => self.label("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale"),
            (Some(Part::NpTaken), T::Ack) => self.label("FLR.S.P.NP_TAKEN.rx_ACK_other"),
            (Some(Part::NpTaken), T::QposReq) => {
                self.label("FLR.S.P.NP_TAKEN.rx_QPOS_REQ");
                let (pos, pr) = (self.s.pos(u), self.s.qprio(u));
                self.send(u, T::Qinfo, None, pos, pr);
            }
            (Some(Part::NpTaken), T::Rtp) => {
                self.label("FLR.S.P.NP_TAKEN.rx_RTP");
                self.revoke3(u);
            }
            (Some(Part::NpTaken), _) => self.label("FLR.S.P.NP_TAKEN.rx_other"),
            (Some(Part::Permitted), T::Release) => {
                self.label("FLR.S.P.PERMITTED.rx_RELEASE");
                self.person(u).released = true;
                self.arb_rel(u);
            }
            (Some(Part::Permitted), T::Request) => {
                self.label("FLR.S.P.PERMITTED.rx_REQUEST");
                self.arb_req_permitted(u);
            }
            (Some(Part::Permitted), T::Rtp) => {
                self.label("FLR.S.P.PERMITTED.rx_RTP");
                self.arb_rtp(u);
            }
            (Some(Part::Permitted), _) => self.label("FLR.S.P.PERMITTED.rx_other"),
            (Some(Part::Prevoke), T::Release) => {
                self.label("FLR.S.P.PREVOKE.rx_RELEASE");
                self.arb_rel(u);
            }
            (Some(Part::Prevoke), T::Rtp) => {
                self.label("FLR.S.P.PREVOKE.rx_RTP");
                self.arb_rtp(u);
            }
            (Some(Part::Prevoke), _) => self.label("FLR.S.P.PREVOKE.rx_other"),
            (_, T::Release) if self.s.people.get(u).is_some_and(|p| p.g_taken) => {
                self.label("FLR.S.P.NPSENDS.rx_RELEASE_taken");
                let tq = ((u32::from(self.s.seq) + 1) % SEQ_WRAP) as u16;
                self.person(u).tq = tq;
                let h = self.s.holder().cloned();
                self.send_e(u, Part::NpTaken, T::Taken, h, 0, 0);
            }
            (_, T::Release) => {
                self.label("FLR.S.P.NPSENDS.rx_RELEASE_idle");
                let prev = self.s.prev.clone();
                self.send_e(u, Part::NpIdle, T::Idle, prev, 0, 0);
            }
            (_, T::Rtp) => self.label("FLR.S.P.NPSENDS.rx_RTP"),
            _ => self.label("FLR.S.P.NPSENDS.rx_other"),
        }
    }

    fn enter(&mut self, u: &str, p: Part) {
        match p {
            Part::NpIdle => {
                self.label("FLR.S.P.NP_IDLE.enter");
                let x = self.person(u);
                x.part = p;
                x.g_taken = false;
                x.c21 = 0;
            }
            Part::NpTaken => {
                self.label("FLR.S.P.NP_TAKEN.enter");
                let x = self.person(u);
                x.part = p;
                x.g_taken = true;
                x.c21 = 1;
                self.restart_p(u, PTimer::T21);
            }
            Part::Permitted => {
                self.label("FLR.S.P.PERMITTED.enter");
                let x = self.person(u);
                x.part = p;
                x.released = false;
                x.c21 = 0;
            }
            Part::Prevoke => {
                self.label("FLR.S.P.PREVOKE.enter");
                let x = self.person(u);
                x.part = p;
                x.c8 = 1;
                x.c21 = 0;
                self.restart_p(u, PTimer::T8);
            }
            _ => {
                self.label("FLR.S.P.NPSENDS.enter");
                let x = self.person(u);
                x.part = p;
                x.c8 = 1;
                x.c21 = 0;
                self.restart_p(u, PTimer::T8);
            }
        }
    }

    fn revoke3(&mut self, u: &str) {
        self.person(u).rcause = revoke::NO_PERMISSION;
        self.send_e(u, Part::NpSends, T::Revoke, None, revoke::NO_PERMISSION, 0);
    }

    fn taken_req(&mut self, u: &str, p: u32) {
        if self.s.people.get(u).is_some_and(|x| x.retry) {
            self.label("FLR.S.P.NP_TAKEN.req_retry_after");
            self.send(u, T::Deny, None, deny::RETRY_AFTER, 0);
            return;
        }
        if self.s.in_q(u) {
            self.label("FLR.S.P.NP_TAKEN.req_renotify");
            let (pos, pr) = (self.s.pos(u), self.s.qprio(u));
            self.send(u, T::Qinfo, None, pos, pr);
            return;
        }
        if p > self.s.holder_prio() && !self.s.pre_q() {
            self.label("FLR.S.P.NP_TAKEN.req_preempt");
            self.arb_preempt(u, p);
            return;
        }
        if p > self.s.holder_prio() {
            self.label("FLR.S.P.NP_TAKEN.req_preempt_blocked");
            return;
        }
        if self.s.queue.len() >= self.env.qmax {
            self.label("FLR.S.P.NP_TAKEN.req_full");
            self.send(u, T::Deny, None, deny::QUEUE_FULL, 0);
            return;
        }
        self.label("FLR.S.P.NP_TAKEN.req_enqueue");
        self.insert_q(Waiter { u: u.to_string(), p, pre: false });
        let pos = self.s.pos(u);
        self.send(u, T::Qinfo, None, pos, p);
    }

    fn away(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.label("FLR.S.G.TAKEN.client_release");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
            self.label("FLR.S.G.PREVOKE.client_release");
            return;
        }
        if self.s.in_q(u) {
            self.label("FLR.S.away.queued");
            self.dequeue(u);
            return;
        }
        self.label("FLR.S.away.other");
    }
}

fn run(s: &mut Srv, env: Env, f: impl FnOnce(&mut Ctx<'_>)) -> R {
    s.restarted.clear();
    s.restarted_p.clear();
    let mut c = Ctx { s, r: R::default(), env };
    f(&mut c);
    c.r
}

pub fn server_rx(s: &mut Srv, env: Env, u: &str, m: &Msg) -> R {
    run(s, env, |c| {
        let member = c.s.members.contains(u);
        if m.t == T::Ack && c.part(u) != Some(Part::NpTaken) {
            c.label("FLR.S.gate.ack_drop");
            return;
        }
        if !member && m.t == T::Request {
            c.label("FLR.S.gate.not_in_room");
            c.send(u, T::Deny, None, deny::OTHER_REASON, 0);
            return;
        }
        if !member {
            c.label("FLR.S.gate.not_in_room_drop");
            return;
        }
        let p = c.s.people.get(u).cloned().unwrap_or_else(|| Person::fresh(false, false, false));
        if m.t == T::Request && !p.half {
            c.label("FLR.S.gate.recv_only");
            c.send(u, T::Deny, None, deny::RECV_ONLY, 0);
            return;
        }
        if m.t == T::Request && !p.pub_room {
            c.label("FLR.S.gate.not_pubroom");
            c.send(u, T::Deny, None, deny::NOT_PUBROOM, 0);
            return;
        }
        if m.t == T::Request && !p.perm {
            c.label("FLR.S.gate.no_permission");
            c.send(u, T::Deny, None, deny::NO_PERMISSION, 0);
            return;
        }
        c.part_c(u, m);
    })
}

pub fn rtp(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| c.part_c(u, &Msg::new(T::Rtp, None, 0, 0)))
}

pub fn ready(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        let h = c.s.holder().cloned();
        if h.as_deref() == Some(u) {
            c.label("FLR.S.P.join.self");
            return;
        }
        if h.is_some() {
            c.label("FLR.S.P.join.taken");
            c.send(u, T::Taken, h, 0, 0);
            return;
        }
        c.label("FLR.S.P.join.idle");
        let prev = c.s.prev.clone();
        c.send(u, T::Idle, prev, 0, 0);
    })
}

pub fn perm_down(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        if c.s.g == G::Taken && c.s.speaker.as_deref() == Some(u) {
            c.label("FLR.S.perm.speaker");
            c.person(u).perm = false;
            c.prevoke_enter(u, revoke::NO_PERMISSION);
            return;
        }
        if c.s.in_q(u) {
            c.label("FLR.S.perm.queued");
            c.person(u).perm = false;
            c.dequeue(u);
            c.part_a(u, &Msg::new(T::Deny, None, deny::NO_PERMISSION, 0));
            return;
        }
        c.label("FLR.S.perm.other");
        c.person(u).perm = false;
    })
}

pub fn perm_up(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.perm.up");
        c.person(u).perm = true;
    })
}

pub fn ws_pub_select(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.ws.pub_select");
        c.person(u).pub_room = true;
    })
}

pub fn ws_reg_half(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.ws.reg_half");
        c.person(u).half = true;
    })
}

pub fn ws_pub_away(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.ws.pub_away");
        c.person(u).pub_room = false;
        c.away(u);
    })
}

pub fn ws_leave(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.ws.leave");
        c.person(u).pub_room = false;
        c.away(u);
        c.s.members.remove(u);
        let p = c.person(u);
        p.retry = false;
        p.part = Part::Releasing;
    })
}

pub fn ws_join(s: &mut Srv, env: Env, u: &str, half: bool, perm: bool) -> R {
    run(s, env, |c| {
        if c.s.g == G::Prevoke && c.s.revoking.as_deref() == Some(u) {
            c.label("FLR.S.ws.join_revoking");
            c.s.prev = c.s.revoking.clone();
            c.idle_enter();
        } else {
            c.label("FLR.S.ws.join");
        }
        let taken = c.s.holder().is_some();
        c.s.members.insert(u.to_string());
        c.s.people.insert(u.to_string(), Person::fresh(half, perm, taken));
    })
}

pub fn t1(s: &mut Srv, env: Env) -> R {
    run(s, env, |c| {
        if c.s.g == G::Taken {
            c.label("FLR.S.G.TAKEN.T1");
            c.s.prev = c.s.speaker.clone();
        } else {
            c.label("FLR.S.G.PREVOKE.T1");
            c.s.prev = c.s.revoking.clone();
        }
        c.idle_enter();
    })
}

pub fn t2(s: &mut Srv, env: Env) -> R {
    run(s, env, |c| {
        c.label("FLR.S.G.TAKEN.T2");
        let u = c.s.speaker.clone().unwrap_or_default();
        c.person(&u).retry = true;
        c.restart_p(&u, PTimer::T9);
        c.prevoke_enter(&u, revoke::TOO_LONG);
    })
}

pub fn t3(s: &mut Srv, env: Env) -> R {
    run(s, env, |c| {
        c.label("FLR.S.G.PREVOKE.T3");
        c.s.prev = c.s.revoking.clone();
        c.idle_enter();
    })
}

pub fn t7(s: &mut Srv, env: Env) -> R {
    run(s, env, |c| {
        c.label("FLR.S.G.IDLE.T7");
        c.s.c7 += 1;
        let prev = c.s.prev.clone();
        c.fan(Msg::new(T::Idle, prev, 0, 0), None);
    })
}

pub fn t20(s: &mut Srv, env: Env) -> R {
    run(s, env, |c| {
        if c.s.c20 < C20 {
            c.label("FLR.S.G.TAKEN.T20");
            c.s.c20 += 1;
            let sp = c.s.speaker.clone().unwrap_or_default();
            let ev = Msg { q: c.s.seq, ..Msg::new(T::Granted, None, c.s.sprio, c.env.t2_secs) };
            c.part_a(&sp, &ev);
            return;
        }
        c.label("FLR.S.G.TAKEN.T20_exhaust");
        c.s.c20 += 1;
    })
}

pub fn t8(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        let part = c.part(u);
        let p = c.person(u);
        p.c8 += 1;
        let cause = p.rcause;
        if part == Some(Part::Prevoke) {
            c.label("FLR.S.P.PREVOKE.T8");
            c.send(u, T::Revoke, None, cause, 0);
            return;
        }
        c.label("FLR.S.P.NPSENDS.T8");
        c.send(u, T::Revoke, None, revoke::NO_PERMISSION, 0);
    })
}

pub fn t21(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        let c21 = c.s.people.get(u).map(|p| p.c21).unwrap_or(0);
        c.person(u).c21 += 1;
        if c21 < C21 {
            c.label("FLR.S.P.NP_TAKEN.T21");
            let h = c.s.holder().cloned();
            let tq = c.s.people.get(u).map(|p| p.tq).unwrap_or(0);
            c.send(u, T::Taken, h, 0, u32::from(tq));
            return;
        }
        c.label("FLR.S.P.NP_TAKEN.T21_exhaust");
    })
}

pub fn t9(s: &mut Srv, env: Env, u: &str) -> R {
    run(s, env, |c| {
        c.label("FLR.S.T9.expire");
        c.person(u).retry = false;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV: Env = Env { qmax: 10, t2_secs: 30, t2_remain_secs: 30 };

    fn room(users: &[&str]) -> Srv {
        let mut s = Srv::new();
        for u in users {
            ws_join(&mut s, ENV, u, true, true);
            ws_pub_select(&mut s, ENV, u);
        }
        s
    }

    fn req(p: u32) -> Msg {
        Msg::new(T::Request, None, p, 0)
    }

    #[test]
    fn grant_then_taken_fan_out() {
        let mut s = room(&["a", "b", "c"]);
        let r = server_rx(&mut s, ENV, "a", &req(0));
        assert_eq!(r.labels[0], "FLR.S.P.NP_IDLE.rx_REQUEST");
        assert!(r.labels.contains(&"FLR.S.G.IDLE.req_grant"));
        assert_eq!(s.g, G::Taken);
        assert_eq!(r.out[0].to, "a");
        assert_eq!(r.out[0].m.t, T::Granted);
        let taken: Vec<&str> = r.out.iter().filter(|o| o.m.t == T::Taken).map(|o| o.to.as_str()).collect();
        assert_eq!(taken, vec!["b", "c"]);
        assert_eq!(r.fx, vec![Fx::Bind("a".into())]);
        assert!(s.armed().contains(&Timer::T1));
    }

    #[test]
    fn queue_and_succession_without_idle() {
        let mut s = room(&["a", "b"]);
        server_rx(&mut s, ENV, "a", &req(0));
        let r = server_rx(&mut s, ENV, "b", &req(0));
        assert!(r.labels.contains(&"FLR.S.P.NP_TAKEN.req_enqueue"));
        let r = server_rx(&mut s, ENV, "a", &Msg::new(T::Release, None, 0, 0));
        assert!(r.labels.contains(&"FLR.S.G.IDLE.enter_succession"));
        assert_eq!(s.speaker.as_deref(), Some("b"));
        assert!(r.out.iter().all(|o| o.m.t != T::Idle));
        assert!(s.armed().contains(&Timer::T20));
    }

    #[test]
    fn gates_and_alone() {
        let mut s = room(&["a"]);
        let r = server_rx(&mut s, ENV, "a", &req(0));
        assert_eq!(r.out[0].m.n, deny::ONLY_ONE);
        let r = server_rx(&mut s, ENV, "z", &req(0));
        assert_eq!(r.out[0].m.n, deny::OTHER_REASON);
        let mut s = Srv::new();
        ws_join(&mut s, ENV, "a", false, true);
        ws_join(&mut s, ENV, "b", true, true);
        let r = server_rx(&mut s, ENV, "a", &req(0));
        assert_eq!(r.out[0].m.n, deny::RECV_ONLY);
        let r = server_rx(&mut s, ENV, "b", &req(0));
        assert_eq!(r.out[0].m.n, deny::NOT_PUBROOM);
    }

    #[test]
    fn t2_revoke_and_t9() {
        let mut s = room(&["a", "b"]);
        server_rx(&mut s, ENV, "a", &req(0));
        rtp(&mut s, ENV, "a");
        assert!(s.armed().contains(&Timer::T2));
        let r = t2(&mut s, ENV);
        assert_eq!(s.g, G::Prevoke);
        assert_eq!(r.out[0].m.t, T::Revoke);
        assert_eq!(r.out[0].m.n, revoke::TOO_LONG);
        assert!(s.armed_p("a").contains(&PTimer::T9));
        let r = t3(&mut s, ENV);
        assert!(r.labels.contains(&"FLR.S.G.IDLE.enter_empty"));
        assert_eq!(r.fx, vec![Fx::Release(Some("a".into()))]);
    }
}
