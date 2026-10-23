// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:FLR.S · 연:RULE-FLR-SERVER-EVENTS · 연:RULE-FLR-PREEMPT · 연:RULE-FLR-PERM-REVOKE-ENFORCE · 연:RULE-FLR-JOIN-NOTIFY · 연:RULE-FLR-TIMERS · 연:RULE-FLR-SEQ · 연:DEV-FLR-T21-SAME-SEQ · 연:DEV-FLR-GRANTED-SEQ · 연:DEV-FLR-ACK-SEQ · 정:IMPL-FLR-LOCK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 발언권 판정(순수) — 연동규격서 원천이 지은 서버 상태기(판정 로직 6.3.4 · 참가자 6.3.5)를 그대로 옮긴 것. 한 걸음 = 새 상태 · 보낼 것 · 지난 라벨 · 효과 · 건 타이머.

use std::collections::{BTreeMap, BTreeSet};

use oxsig::flr::{deny, qpos, revoke, timers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum G {
    Idle,
    Taken,
    PRevoke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P {
    NpIdle,
    NpTaken,
    Permitted,
    PRevoke,
    NpSends,
    Releasing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub to: String,
    pub m: Msg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fx {
    Bind(String),
    Release(Option<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Timer {
    T1,
    T2,
    T3,
    T7,
    T20,
    T8,
    T9,
    T21,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub pub_: BTreeMap<String, bool>,
    pub half: BTreeMap<String, bool>,
    pub perm: BTreeMap<String, bool>,
    pub retry: BTreeMap<String, bool>,
    pub part: BTreeMap<String, P>,
    pub released: BTreeMap<String, bool>,
    pub g_taken: BTreeMap<String, bool>,
    pub c8: BTreeMap<String, u32>,
    pub c21: BTreeMap<String, u32>,
    pub tq: BTreeMap<String, u16>,
    pub rcause: BTreeMap<String, u32>,
    pub seq: u16,
}

impl Default for Srv {
    fn default() -> Self {
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
            pub_: BTreeMap::new(),
            half: BTreeMap::new(),
            perm: BTreeMap::new(),
            retry: BTreeMap::new(),
            part: BTreeMap::new(),
            released: BTreeMap::new(),
            g_taken: BTreeMap::new(),
            c8: BTreeMap::new(),
            c21: BTreeMap::new(),
            tq: BTreeMap::new(),
            rcause: BTreeMap::new(),
            seq: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct R {
    pub o: Vec<Out>,
    pub l: Vec<&'static str>,
    pub fx: Vec<Fx>,
    pub starts: Vec<(Timer, Option<String>)>,
}

pub struct Cfg {
    pub q_max: usize,
    pub t2_secs: u32,
    pub t2_remain_secs: u32,
}

fn b(m: &BTreeMap<String, bool>, u: &str) -> bool {
    m.get(u).copied().unwrap_or(false)
}

fn n(m: &BTreeMap<String, u32>, u: &str) -> u32 {
    m.get(u).copied().unwrap_or(0)
}

impl Srv {
    pub fn part_of(&self, u: &str) -> P {
        self.part.get(u).copied().unwrap_or(P::NpIdle)
    }

    pub fn holder(&self) -> Option<String> {
        match self.g {
            G::Taken => self.speaker.clone(),
            G::PRevoke => self.revoking.clone(),
            G::Idle => None,
        }
    }

    fn holder_prio(&self) -> u32 {
        if self.g == G::Taken {
            self.sprio
        } else {
            self.rprio
        }
    }

    fn in_q(&self, u: &str) -> bool {
        self.queue.iter().any(|w| w.u == u)
    }

    fn pos(&self, u: &str) -> u32 {
        self.queue.iter().position(|w| w.u == u).map(|i| i as u32 + 1).unwrap_or(qpos::NOT_IN_QUEUE as u32)
    }

    fn q_prio(&self, u: &str) -> u32 {
        self.queue.iter().find(|w| w.u == u).map(|w| w.p).unwrap_or(0)
    }

    fn dequeue(&mut self, u: &str) {
        self.queue.retain(|w| w.u != u);
    }

    fn pre_q(&self) -> bool {
        self.queue.iter().any(|w| w.pre)
    }

    fn insert_q(&mut self, w: Waiter) {
        let before = |w: &Waiter, x: &Waiter| (w.pre && !x.pre) || (w.pre == x.pre && w.p > x.p);
        let i = self.queue.iter().position(|x| before(&w, x));
        match i {
            Some(i) => self.queue.insert(i, w),
            None => self.queue.push(w),
        }
    }
}

pub struct Step<'a> {
    pub s: &'a mut Srv,
    pub r: R,
    pub cfg: &'a Cfg,
}

impl<'a> Step<'a> {
    pub fn new(s: &'a mut Srv, cfg: &'a Cfg) -> Step<'a> {
        Step { s, r: R::default(), cfg }
    }

    fn l(&mut self, label: &'static str) {
        self.r.l.push(label);
    }

    fn start(&mut self, t: Timer, u: Option<&str>) {
        self.r.starts.push((t, u.map(str::to_string)));
    }

    fn send(&mut self, u: &str, t: T, a: Option<String>, nn: u32, d: u32) {
        let seqd = matches!(t, T::Taken | T::Idle);
        let bump = seqd && d == 0;
        if bump {
            self.s.seq = self.s.seq.wrapping_add(1);
        }
        let q = if bump { self.s.seq } else if seqd { d as u16 } else { 0 };
        let d2 = if seqd { 0 } else { d };
        self.r.o.push(Out { to: u.to_string(), m: Msg { t, a, n: nn, d: d2, q } });
    }

    fn send_q(&mut self, u: &str, t: T, a: Option<String>, nn: u32, d: u32, q: u16) {
        self.r.o.push(Out { to: u.to_string(), m: Msg { t, a, n: nn, d, q } });
    }

    fn send_e(&mut self, u: &str, p: P, t: T, a: Option<String>, nn: u32, d: u32) {
        self.enter(u, p);
        self.send(u, t, a, nn, d);
    }

    fn send_eq(&mut self, u: &str, p: P, m: Msg) {
        self.enter(u, p);
        self.send_q(u, m.t, m.a, m.n, m.d, m.q);
    }

    fn fan(&mut self, mut ev: Msg, skip: Option<&str>) {
        let rest: Vec<String> = self.s.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
        if matches!(ev.t, T::Taken | T::Idle) {
            self.s.seq = self.s.seq.wrapping_add(1);
            ev.q = self.s.seq;
        }
        for u in rest {
            self.part_a(&u, &ev);
        }
    }

    pub fn t2_left(&self) -> u32 {
        if self.s.first_rtp {
            self.cfg.t2_remain_secs
        } else {
            self.cfg.t2_secs
        }
    }

    fn enter_taken(&mut self) {
        self.l("FLR.S.G.TAKEN.enter");
        self.s.idle_bcast = false;
        self.start(Timer::T1, None);
        let sp = self.s.speaker.clone().unwrap_or_default();
        let ev = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.cfg.t2_secs, q: self.s.seq };
        self.part_a(&sp, &ev);
        self.fan(Msg::new(T::Taken, Some(sp.clone()), 0, 0), Some(&sp));
        self.r.fx.push(Fx::Bind(sp));
    }

    fn idle_enter(&mut self) {
        if self.s.queue.is_empty() {
            self.l("FLR.S.G.IDLE.enter_empty");
            self.s.g = G::Idle;
            self.s.speaker = None;
            self.s.revoking = None;
            self.s.first_rtp = false;
            self.s.idle_bcast = true;
            self.s.c7 = 1;
            self.start(Timer::T7, None);
            let prev = self.s.prev.clone();
            self.fan(Msg::new(T::Idle, prev.clone(), 0, 0), None);
            self.r.fx.push(Fx::Release(prev));
            return;
        }
        self.l("FLR.S.G.IDLE.enter_succession");
        let w = self.s.queue.remove(0);
        self.s.g = G::Taken;
        self.s.speaker = Some(w.u);
        self.s.sprio = w.p;
        self.s.revoking = None;
        self.s.succ = true;
        self.s.rtp_seen = false;
        self.s.first_rtp = false;
        self.s.c20 = 1;
        self.start(Timer::T20, None);
        self.enter_taken();
    }

    fn prevoke_enter(&mut self, u: &str, cause: u32) {
        self.l("FLR.S.G.PREVOKE.enter");
        self.s.g = G::PRevoke;
        self.s.revoking = Some(u.to_string());
        self.s.rprio = self.s.sprio;
        self.s.rcause_g = cause;
        self.s.speaker = None;
        self.start(Timer::T3, None);
        self.part_a(u, &Msg::new(T::Revoke, None, cause, 0));
    }

    fn arb_req_idle(&mut self, u: &str, p: u32) {
        if self.s.g != G::Idle {
            self.l("FLR.S.G.any.req_discard");
            return;
        }
        if self.s.members.len() <= 1 {
            self.l("FLR.S.G.IDLE.req_alone");
            self.part_a(u, &Msg::new(T::Deny, None, deny::ONLY_ONE as u32, 0));
            return;
        }
        if b(&self.s.retry, u) {
            self.l("FLR.S.G.IDLE.req_retry_after");
            self.part_a(u, &Msg::new(T::Deny, None, deny::RETRY_AFTER as u32, 0));
            return;
        }
        self.l("FLR.S.G.IDLE.req_grant");
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
            self.l("FLR.S.G.TAKEN.req_regrant");
            let ev = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.t2_left(), q: self.s.seq };
            self.part_a(u, &ev);
            return;
        }
        self.l("FLR.S.G.any.regrant_discard");
    }

    fn arb_preempt(&mut self, u: &str, p: u32) {
        if self.s.g == G::Taken {
            self.l("FLR.S.G.TAKEN.preempt");
            self.s.queue.insert(0, Waiter { u: u.to_string(), p, pre: true });
            let sp = self.s.speaker.clone().unwrap_or_default();
            self.prevoke_enter(&sp, revoke::PREEMPTED as u32);
            self.part_a(u, &Msg::new(T::Qinfo, None, 1, p));
            return;
        }
        self.l("FLR.S.G.PREVOKE.preempt_discard");
    }

    fn arb_rel(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.G.TAKEN.rel_speaker");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        if self.s.g == G::PRevoke && self.s.revoking.as_deref() == Some(u) {
            self.l("FLR.S.G.PREVOKE.rel_revoked");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        self.l("FLR.S.G.any.rel_discard");
    }

    fn arb_rtp(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.G.TAKEN.rtp");
            if !self.s.first_rtp {
                self.start(Timer::T2, None);
            }
            self.s.rtp_seen = true;
            self.s.first_rtp = true;
            return;
        }
        if self.s.g == G::PRevoke && self.s.revoking.as_deref() == Some(u) {
            self.l("FLR.S.G.PREVOKE.rtp");
            return;
        }
        self.l("FLR.S.G.any.rtp_discard");
    }

    pub fn server_rx(&mut self, u: &str, m: &Msg) {
        if m.t == T::Ack && self.s.part_of(u) != P::NpTaken {
            self.l("FLR.S.gate.ack_drop");
            return;
        }
        let member = self.s.members.contains(u);
        if !member && m.t == T::Request {
            self.l("FLR.S.gate.not_in_room");
            self.send(u, T::Deny, None, deny::OTHER_REASON as u32, 0);
            return;
        }
        if !member {
            self.l("FLR.S.gate.not_in_room_drop");
            return;
        }
        if m.t == T::Request && !b(&self.s.half, u) {
            self.l("FLR.S.gate.recv_only");
            self.send(u, T::Deny, None, deny::RECV_ONLY as u32, 0);
            return;
        }
        if m.t == T::Request && !b(&self.s.pub_, u) {
            self.l("FLR.S.gate.not_pubroom");
            self.send(u, T::Deny, None, deny::NOT_PUBROOM as u32, 0);
            return;
        }
        if m.t == T::Request && !self.s.perm.get(u).copied().unwrap_or(true) {
            self.l("FLR.S.gate.no_permission");
            self.send(u, T::Deny, None, deny::NO_PERMISSION as u32, 0);
            return;
        }
        self.part_c(u, m);
    }

    pub fn ready(&mut self, u: &str) {
        let h = self.s.holder();
        if h.as_deref() == Some(u) {
            self.l("FLR.S.P.join.self");
            return;
        }
        if h.is_some() {
            self.l("FLR.S.P.join.taken");
            self.send(u, T::Taken, h, 0, 0);
            return;
        }
        self.l("FLR.S.P.join.idle");
        let prev = self.s.prev.clone();
        self.send(u, T::Idle, prev, 0, 0);
    }

    pub fn perm_down(&mut self, u: &str) {
        self.s.perm.insert(u.to_string(), false);
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.perm.speaker");
            self.prevoke_enter(u, revoke::NO_PERMISSION as u32);
            return;
        }
        if self.s.in_q(u) {
            self.l("FLR.S.perm.queued");
            self.s.dequeue(u);
            self.part_a(u, &Msg::new(T::Deny, None, deny::NO_PERMISSION as u32, 0));
            return;
        }
        self.l("FLR.S.perm.other");
    }

    pub fn perm_up(&mut self, u: &str) {
        self.l("FLR.S.perm.up");
        self.s.perm.insert(u.to_string(), true);
    }

    fn away(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.G.TAKEN.client_release");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        if self.s.g == G::PRevoke && self.s.revoking.as_deref() == Some(u) {
            self.l("FLR.S.G.PREVOKE.client_release");
            return;
        }
        if self.s.in_q(u) {
            self.l("FLR.S.away.queued");
            self.s.dequeue(u);
            return;
        }
        self.l("FLR.S.away.other");
    }

    pub fn ws_pub_select(&mut self, u: &str) {
        self.l("FLR.S.ws.pub_select");
        self.s.pub_.insert(u.to_string(), true);
    }

    pub fn ws_reg_half(&mut self, u: &str, on: bool) {
        self.l("FLR.S.ws.reg_half");
        self.s.half.insert(u.to_string(), on);
    }

    pub fn ws_pub_away(&mut self, u: &str) {
        self.l("FLR.S.ws.pub_away");
        self.s.pub_.insert(u.to_string(), false);
        self.away(u);
    }

    pub fn ws_join(&mut self, u: &str, perm_floor: bool) {
        self.s.members.insert(u.to_string());
        self.s.perm.insert(u.to_string(), perm_floor);
        if self.s.part_of(u) == P::Releasing {
            self.s.part.insert(u.to_string(), P::NpIdle);
        }
    }

    pub fn ws_leave(&mut self, u: &str) {
        self.l("FLR.S.ws.leave");
        self.s.pub_.insert(u.to_string(), false);
        self.away(u);
        self.s.members.remove(u);
        self.s.retry.insert(u.to_string(), false);
        self.s.part.insert(u.to_string(), P::Releasing);
    }

    fn part_a(&mut self, u: &str, ev: &Msg) {
        let p = self.s.part_of(u);
        match (p, ev.t) {
            (_, T::Qinfo) => {
                self.l("FLR.S.P.any.a_QINFO");
                self.send(u, T::Qinfo, None, ev.n, ev.d);
            }
            (_, T::Deny) => {
                self.l("FLR.S.P.any.a_DENY");
                self.send(u, T::Deny, None, ev.n, 0);
            }
            (P::NpIdle, T::Granted) => {
                self.l("FLR.S.P.NP_IDLE.a_GRANTED");
                self.send_eq(u, P::Permitted, Msg { t: T::Granted, a: None, n: ev.n, d: ev.d, q: ev.q });
            }
            (P::NpIdle, T::Taken) => {
                self.l("FLR.S.P.NP_IDLE.a_TAKEN");
                self.s.tq.insert(u.to_string(), ev.q);
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpIdle, T::Idle) => {
                self.l("FLR.S.P.NP_IDLE.a_IDLE");
                self.send(u, T::Idle, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpIdle, _) => self.l("FLR.S.P.NP_IDLE.a_other"),
            (P::NpTaken, T::Granted) => {
                self.l("FLR.S.P.NP_TAKEN.a_GRANTED");
                self.send_eq(u, P::Permitted, Msg { t: T::Granted, a: None, n: ev.n, d: ev.d, q: ev.q });
            }
            (P::NpTaken, T::Taken) => {
                self.l("FLR.S.P.NP_TAKEN.a_TAKEN");
                self.s.tq.insert(u.to_string(), ev.q);
                self.s.c21.insert(u.to_string(), 1);
                self.start(Timer::T21, Some(u));
                self.send(u, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpTaken, T::Idle) => {
                self.l("FLR.S.P.NP_TAKEN.a_IDLE");
                self.send_e(u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpTaken, _) => self.l("FLR.S.P.NP_TAKEN.a_other"),
            (P::Permitted, T::Granted) => {
                self.l("FLR.S.P.PERMITTED.a_GRANTED");
                self.send_q(u, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (P::Permitted, T::Idle) => {
                self.l("FLR.S.P.PERMITTED.a_IDLE");
                self.enter(u, P::NpIdle);
            }
            (P::Permitted, T::Taken) => {
                self.l("FLR.S.P.PERMITTED.a_TAKEN");
                self.s.tq.insert(u.to_string(), ev.q);
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::Permitted, T::Revoke) => {
                self.l("FLR.S.P.PERMITTED.a_REVOKE");
                self.s.rcause.insert(u.to_string(), ev.n);
                self.send_e(u, P::PRevoke, T::Revoke, None, ev.n, 0);
            }
            (P::Permitted, _) => self.l("FLR.S.P.PERMITTED.a_other"),
            (P::PRevoke, T::Idle) => {
                self.l("FLR.S.P.PREVOKE.a_IDLE");
                self.send_e(u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q as u32);
            }
            (P::PRevoke, T::Taken) => {
                self.l("FLR.S.P.PREVOKE.a_TAKEN");
                self.s.tq.insert(u.to_string(), ev.q);
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::PRevoke, _) => self.l("FLR.S.P.PREVOKE.a_other"),
            (P::Releasing, _) => self.l("FLR.S.P.RELEASING.a_any"),
            (P::NpSends, T::Idle) => {
                self.l("FLR.S.P.NPSENDS.a_IDLE");
                self.s.g_taken.insert(u.to_string(), false);
            }
            (P::NpSends, T::Taken) => {
                self.l("FLR.S.P.NPSENDS.a_TAKEN");
                self.s.g_taken.insert(u.to_string(), true);
            }
            (P::NpSends, _) => self.l("FLR.S.P.NPSENDS.a_other"),
        }
    }

    fn part_c(&mut self, u: &str, ev: &Msg) {
        let p = self.s.part_of(u);
        match (p, ev.t) {
            (P::NpIdle, T::Request) => {
                self.l("FLR.S.P.NP_IDLE.rx_REQUEST");
                self.arb_req_idle(u, ev.n);
            }
            (P::NpIdle, T::Release) => {
                self.l("FLR.S.P.NP_IDLE.rx_RELEASE");
                self.s.dequeue(u);
                let prev = self.s.prev.clone();
                self.send(u, T::Idle, prev, 0, 0);
            }
            (P::NpIdle, T::Rtp) if b(&self.s.released, u) => {
                self.l("FLR.S.P.NP_IDLE.rx_RTP_after_release");
                self.revoke3(u);
            }
            (P::NpIdle, T::Rtp) => self.l("FLR.S.P.NP_IDLE.rx_RTP_drop"),
            (P::NpIdle, _) => self.l("FLR.S.P.NP_IDLE.rx_other"),
            (P::NpTaken, T::Request) => {
                self.l("FLR.S.P.NP_TAKEN.rx_REQUEST");
                self.taken_req(u, ev.n);
            }
            (P::NpTaken, T::Release) => {
                self.l("FLR.S.P.NP_TAKEN.rx_RELEASE");
                let next = self.s.seq.wrapping_add(1);
                self.s.tq.insert(u.to_string(), next);
                self.s.c21.insert(u.to_string(), 1);
                self.start(Timer::T21, Some(u));
                self.s.dequeue(u);
                let h = self.s.holder();
                self.send(u, T::Taken, h, 0, 0);
            }
            (P::NpTaken, T::Ack) if ev.n == 2 && Some(ev.q) == self.s.tq.get(u).copied() => {
                self.l("FLR.S.P.NP_TAKEN.rx_ACK_taken");
                self.s.c21.insert(u.to_string(), 0);
            }
            (P::NpTaken, T::Ack) if ev.n == 2 => self.l("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale"),
            (P::NpTaken, T::Ack) => self.l("FLR.S.P.NP_TAKEN.rx_ACK_other"),
            (P::NpTaken, T::QposReq) => {
                self.l("FLR.S.P.NP_TAKEN.rx_QPOS_REQ");
                let (pos, pr) = (self.s.pos(u), self.s.q_prio(u));
                self.send(u, T::Qinfo, None, pos, pr);
            }
            (P::NpTaken, T::Rtp) => {
                self.l("FLR.S.P.NP_TAKEN.rx_RTP");
                self.revoke3(u);
            }
            (P::NpTaken, _) => self.l("FLR.S.P.NP_TAKEN.rx_other"),
            (P::Permitted, T::Release) => {
                self.l("FLR.S.P.PERMITTED.rx_RELEASE");
                self.s.released.insert(u.to_string(), true);
                self.arb_rel(u);
            }
            (P::Permitted, T::Request) => {
                self.l("FLR.S.P.PERMITTED.rx_REQUEST");
                self.arb_req_permitted(u);
            }
            (P::Permitted, T::Rtp) => {
                self.l("FLR.S.P.PERMITTED.rx_RTP");
                self.arb_rtp(u);
            }
            (P::Permitted, _) => self.l("FLR.S.P.PERMITTED.rx_other"),
            (P::PRevoke, T::Release) => {
                self.l("FLR.S.P.PREVOKE.rx_RELEASE");
                self.arb_rel(u);
            }
            (P::PRevoke, T::Rtp) => {
                self.l("FLR.S.P.PREVOKE.rx_RTP");
                self.arb_rtp(u);
            }
            (P::PRevoke, _) => self.l("FLR.S.P.PREVOKE.rx_other"),
            (_, T::Release) if b(&self.s.g_taken, u) => {
                self.l("FLR.S.P.NPSENDS.rx_RELEASE_taken");
                let next = self.s.seq.wrapping_add(1);
                self.s.tq.insert(u.to_string(), next);
                let h = self.s.holder();
                self.send_e(u, P::NpTaken, T::Taken, h, 0, 0);
            }
            (_, T::Release) => {
                self.l("FLR.S.P.NPSENDS.rx_RELEASE_idle");
                let prev = self.s.prev.clone();
                self.send_e(u, P::NpIdle, T::Idle, prev, 0, 0);
            }
            (_, T::Rtp) => self.l("FLR.S.P.NPSENDS.rx_RTP"),
            _ => self.l("FLR.S.P.NPSENDS.rx_other"),
        }
    }

    fn enter(&mut self, u: &str, p: P) {
        let key = u.to_string();
        match p {
            P::NpIdle => {
                self.l("FLR.S.P.NP_IDLE.enter");
                self.s.g_taken.insert(key.clone(), false);
                self.s.c21.insert(key.clone(), 0);
            }
            P::NpTaken => {
                self.l("FLR.S.P.NP_TAKEN.enter");
                self.s.g_taken.insert(key.clone(), true);
                self.s.c21.insert(key.clone(), 1);
                self.start(Timer::T21, Some(u));
            }
            P::Permitted => {
                self.l("FLR.S.P.PERMITTED.enter");
                self.s.released.insert(key.clone(), false);
                self.s.c21.insert(key.clone(), 0);
            }
            P::PRevoke => {
                self.l("FLR.S.P.PREVOKE.enter");
                self.s.c8.insert(key.clone(), 1);
                self.s.c21.insert(key.clone(), 0);
                self.start(Timer::T8, Some(u));
            }
            P::NpSends | P::Releasing => {
                self.l("FLR.S.P.NPSENDS.enter");
                self.s.c8.insert(key.clone(), 1);
                self.s.c21.insert(key.clone(), 0);
                self.start(Timer::T8, Some(u));
            }
        }
        self.s.part.insert(key, p);
    }

    fn revoke3(&mut self, u: &str) {
        self.s.rcause.insert(u.to_string(), revoke::NO_PERMISSION as u32);
        self.send_e(u, P::NpSends, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
    }

    fn taken_req(&mut self, u: &str, p: u32) {
        if b(&self.s.retry, u) {
            self.l("FLR.S.P.NP_TAKEN.req_retry_after");
            self.send(u, T::Deny, None, deny::RETRY_AFTER as u32, 0);
            return;
        }
        if self.s.in_q(u) {
            self.l("FLR.S.P.NP_TAKEN.req_renotify");
            let (pos, pr) = (self.s.pos(u), self.s.q_prio(u));
            self.send(u, T::Qinfo, None, pos, pr);
            return;
        }
        if p > self.s.holder_prio() && !self.s.pre_q() {
            self.l("FLR.S.P.NP_TAKEN.req_preempt");
            self.arb_preempt(u, p);
            return;
        }
        if p > self.s.holder_prio() {
            self.l("FLR.S.P.NP_TAKEN.req_preempt_blocked");
            return;
        }
        if self.s.queue.len() >= self.cfg.q_max {
            self.l("FLR.S.P.NP_TAKEN.req_full");
            self.send(u, T::Deny, None, deny::QUEUE_FULL as u32, 0);
            return;
        }
        self.l("FLR.S.P.NP_TAKEN.req_enqueue");
        self.s.insert_q(Waiter { u: u.to_string(), p, pre: false });
        let pos = self.s.pos(u);
        self.send(u, T::Qinfo, None, pos, p);
    }

    pub fn on_rtp(&mut self, u: &str) {
        let m = Msg::new(T::Rtp, None, 0, 0);
        if self.s.members.contains(u) {
            self.part_c(u, &m);
        }
    }

    pub fn srv_t1(&mut self) {
        if self.s.g == G::Taken {
            self.l("FLR.S.G.TAKEN.T1");
            self.s.prev = self.s.speaker.clone();
        } else {
            self.l("FLR.S.G.PREVOKE.T1");
            self.s.prev = self.s.revoking.clone();
        }
        self.idle_enter();
    }

    pub fn srv_t2(&mut self) {
        self.l("FLR.S.G.TAKEN.T2");
        let u = self.s.speaker.clone().unwrap_or_default();
        if !b(&self.s.retry, &u) {
            self.start(Timer::T9, Some(&u));
        }
        self.s.retry.insert(u.clone(), true);
        self.prevoke_enter(&u, revoke::TOO_LONG as u32);
    }

    pub fn srv_t3(&mut self) {
        self.l("FLR.S.G.PREVOKE.T3");
        self.s.prev = self.s.revoking.clone();
        self.idle_enter();
    }

    pub fn srv_t7(&mut self) {
        self.l("FLR.S.G.IDLE.T7");
        self.s.c7 += 1;
        let prev = self.s.prev.clone();
        self.fan(Msg::new(T::Idle, prev, 0, 0), None);
    }

    pub fn srv_t20(&mut self) {
        if self.s.c20 < timers::C20 {
            self.l("FLR.S.G.TAKEN.T20");
            self.s.c20 += 1;
            let sp = self.s.speaker.clone().unwrap_or_default();
            let ev = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.cfg.t2_secs, q: self.s.seq };
            self.part_a(&sp, &ev);
            return;
        }
        self.l("FLR.S.G.TAKEN.T20_exhaust");
        self.s.c20 += 1;
    }

    pub fn srv_t8(&mut self, u: &str) {
        let c = n(&self.s.c8, u) + 1;
        self.s.c8.insert(u.to_string(), c);
        if self.s.part_of(u) == P::PRevoke {
            self.l("FLR.S.P.PREVOKE.T8");
            let cause = n(&self.s.rcause, u);
            self.send(u, T::Revoke, None, cause, 0);
        } else {
            self.l("FLR.S.P.NPSENDS.T8");
            self.send(u, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
        }
    }

    pub fn srv_t21(&mut self, u: &str) {
        let c = n(&self.s.c21, u);
        self.s.c21.insert(u.to_string(), c + 1);
        if c < timers::C21 {
            self.l("FLR.S.P.NP_TAKEN.T21");
            let h = self.s.holder();
            let q = self.s.tq.get(u).copied().unwrap_or(0);
            self.send(u, T::Taken, h, 0, q as u32);
            return;
        }
        self.l("FLR.S.P.NP_TAKEN.T21_exhaust");
    }

    pub fn srv_t9(&mut self, u: &str) {
        self.l("FLR.S.T9.expire");
        self.s.retry.insert(u.to_string(), false);
    }
}

pub fn armed_server(s: &Srv) -> Vec<Timer> {
    let mut on = Vec::new();
    if s.g == G::Taken || s.g == G::PRevoke {
        on.push(Timer::T1);
    }
    if s.g == G::Taken && s.first_rtp {
        on.push(Timer::T2);
    }
    if s.g == G::PRevoke {
        on.push(Timer::T3);
    }
    if s.g == G::Idle && s.idle_bcast && s.c7 < timers::C7 {
        on.push(Timer::T7);
    }
    if s.g == G::Taken && s.succ && !s.rtp_seen && s.c20 <= timers::C20 {
        on.push(Timer::T20);
    }
    on
}

pub fn armed_participant(s: &Srv, u: &str) -> Vec<Timer> {
    let mut on = Vec::new();
    let p = s.part_of(u);
    if (p == P::PRevoke || p == P::NpSends) && n(&s.c8, u) < timers::C8 {
        on.push(Timer::T8);
    }
    if b(&s.retry, u) {
        on.push(Timer::T9);
    }
    if p == P::NpTaken && n(&s.c21, u) >= 1 && n(&s.c21, u) <= timers::C21 {
        on.push(Timer::T21);
    }
    on
}

pub fn ms_of(t: Timer, t2_ms: u64) -> u64 {
    match t {
        Timer::T1 => timers::T1_MS,
        Timer::T2 => t2_ms,
        Timer::T3 => timers::T3_MS,
        Timer::T7 => timers::T7_MS,
        Timer::T8 => timers::T8_MS,
        Timer::T9 => timers::T9_MS,
        Timer::T20 => timers::T20_MS,
        Timer::T21 => timers::T21_MS,
    }
}

pub fn retx(t: Timer) -> bool {
    matches!(t, Timer::T7 | Timer::T8 | Timer::T20 | Timer::T21)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Cfg {
        Cfg { q_max: 10, t2_secs: 30, t2_remain_secs: 30 }
    }

    fn room(users: &[&str]) -> Srv {
        let mut s = Srv::default();
        for u in users {
            s.members.insert(u.to_string());
            s.half.insert(u.to_string(), true);
            s.pub_.insert(u.to_string(), true);
            s.perm.insert(u.to_string(), true);
        }
        s
    }

    fn req(p: u32) -> Msg {
        Msg::new(T::Request, None, p, 0)
    }

    #[test]
    fn grant_taken_idle_round_carries_seq() {
        let c = cfg();
        let mut s = room(&["a", "b"]);
        let r = {
            let mut st = Step::new(&mut s, &c);
            st.server_rx("a", &req(0));
            st.r
        };
        assert!(r.l.contains(&"FLR.S.G.IDLE.req_grant"));
        let granted = r.o.iter().find(|o| o.to == "a").map(|o| (o.m.t, o.m.d));
        assert_eq!(granted, Some((T::Granted, 30)));
        let taken = r.o.iter().find(|o| o.to == "b").map(|o| (o.m.t, o.m.a.clone(), o.m.q));
        assert_eq!(taken, Some((T::Taken, Some("a".into()), 1)));
        assert_eq!(s.part_of("b"), P::NpTaken);
        let r2 = {
            let mut st = Step::new(&mut s, &c);
            st.server_rx("a", &Msg::new(T::Release, None, 0, 0));
            st.r
        };
        let idle = r2.o.iter().find(|o| o.to == "b").map(|o| (o.m.t, o.m.a.clone(), o.m.q));
        assert_eq!(idle, Some((T::Idle, Some("a".into()), 2)));
        assert_eq!(s.g, G::Idle);
    }

    #[test]
    fn gate_reasons_in_order() {
        let c = cfg();
        let mut s = room(&["a", "b"]);
        s.half.insert("a".into(), false);
        let mut st = Step::new(&mut s, &c);
        st.server_rx("a", &req(0));
        st.server_rx("zz", &req(0));
        let causes: Vec<u32> = st.r.o.iter().map(|o| o.m.n).collect();
        assert_eq!(causes, vec![deny::RECV_ONLY as u32, deny::OTHER_REASON as u32]);
    }

    #[test]
    fn preempt_revokes_then_successor_after_release() {
        let c = cfg();
        let mut s = room(&["a", "b", "c"]);
        Step::new(&mut s, &c).server_rx("a", &req(0));
        let r = {
            let mut st = Step::new(&mut s, &c);
            st.server_rx("b", &req(5));
            st.r
        };
        assert!(r.l.contains(&"FLR.S.G.TAKEN.preempt"));
        assert_eq!(s.g, G::PRevoke);
        assert!(r.o.iter().any(|o| o.to == "a" && o.m.t == T::Revoke && o.m.n == revoke::PREEMPTED as u32));
        assert!(r.o.iter().any(|o| o.to == "b" && o.m.t == T::Qinfo && o.m.n == 1));
        let r2 = {
            let mut st = Step::new(&mut s, &c);
            st.server_rx("a", &Msg::new(T::Release, None, 0, 0));
            st.r
        };
        assert!(r2.l.contains(&"FLR.S.G.IDLE.enter_succession"));
        assert_eq!(s.speaker.as_deref(), Some("b"));
    }

    #[test]
    fn ready_sends_current_holder_and_bumps_seq() {
        let c = cfg();
        let mut s = room(&["a", "b"]);
        Step::new(&mut s, &c).server_rx("a", &req(0));
        let before = s.seq;
        let r = {
            let mut st = Step::new(&mut s, &c);
            st.ready("b");
            st.r
        };
        assert_eq!(r.o[0].m.t, T::Taken);
        assert_eq!(r.o[0].m.q, before.wrapping_add(1));
    }

    #[test]
    fn armed_sets_follow_state() {
        let c = cfg();
        let mut s = room(&["a", "b"]);
        Step::new(&mut s, &c).server_rx("a", &req(0));
        assert_eq!(armed_server(&s), vec![Timer::T1]);
        Step::new(&mut s, &c).on_rtp("a");
        assert_eq!(armed_server(&s), vec![Timer::T1, Timer::T2]);
        assert_eq!(armed_participant(&s, "b"), vec![Timer::T21]);
    }
}
