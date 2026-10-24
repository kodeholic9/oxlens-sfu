// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-FLR-SERVER-SCOPE · 정:IMPL-FLR-LOCK · 정:IMPL-FLR-POLICY · 연:FLR.S · 연:RULE-FLR-SERVER-EVENTS · 연:RULE-FLR-OWN-RULES · 연:RULE-FLR-TIMERS · 연:RULE-FLR-JOIN-NOTIFY · 연:RULE-FLR-MSG · 연:RULE-FLR-TLV · 연:DEV-FLR-PUBROOM · 연:DEV-FLR-RECVONLY-TRIGGER · 연:DEV-FLR-PERM-REVOKE · 연:DEV-FLR-T9 · 연:DEV-FLR-TAKEN-ACK · 연:DEV-FLR-T21-SAME-SEQ · 연:DEV-FLR-FIRST-NOTIFY · 연:DEV-FLR-PREEMPT-RELATIVE · 연:DEV-FLR-DENY-255 · 연:R-FLR-T2-FIRST-RTP · model: claude-opus-5-5
//! 발언권 판정(순수) — 생성 서버 상태기(out/floor/server/floor_server.ts · 3GPP TS 24.380 §6.3.4 판정 로직 · §6.3.5 참가자 상태기)를 그대로 옮겼다. 판정은 보낼 것 · 효과 · 라벨의 목록을 돌려준다.

use std::collections::{BTreeMap, BTreeSet};

pub mod deny {
    pub const ONLY_ONE: u8 = 3;
    pub const RETRY_AFTER: u8 = 4;
    pub const RECV_ONLY: u8 = 5;
    pub const QUEUE_FULL: u8 = 7;
    pub const NOT_PUBROOM: u8 = 101;
    pub const NO_PERMISSION: u8 = 102;
    pub const OTHER_REASON: u8 = 255;
}

pub mod revoke {
    pub const TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
}

pub const QPOS_NOT_IN_QUEUE: u8 = 254;
pub const SEQ_WRAP: u32 = 65_536;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P {
    NpIdle,
    NpTaken,
    Permitted,
    Prevoke,
    Npsends,
    Releasing,
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
    MediaBind(String),
    MediaRelease(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pu {
    pub pub_: bool,
    pub half: bool,
    pub perm: bool,
    pub retry: bool,
    pub part: P,
    pub released: bool,
    pub g_taken: bool,
    pub c8: u32,
    pub c21: u32,
    pub tq: u16,
    pub rcause: u32,
}

impl Default for Pu {
    fn default() -> Self {
        Pu { pub_: false, half: false, perm: true, retry: false, part: P::NpIdle, released: false, g_taken: false, c8: 0, c21: 0, tq: 0, rcause: 0 }
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
    pub u: BTreeMap<String, Pu>,
    pub seq: u16,
}

impl Default for Srv {
    fn default() -> Self {
        Srv { g: G::Idle, speaker: None, sprio: 0, revoking: None, rprio: 0, rcause_g: 0, queue: Vec::new(), succ: false, rtp_seen: false, first_rtp: false, c20: 0, c7: 0, idle_bcast: false, prev: None, members: BTreeSet::new(), u: BTreeMap::new(), seq: 0 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cfg {
    pub qmax: usize,
    pub t2_secs: u32,
    pub t2_remain_secs: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct R {
    pub o: Vec<Out>,
    pub l: Vec<&'static str>,
    pub fx: Vec<Fx>,
}

pub struct Step<'a> {
    pub s: &'a mut Srv,
    pub cfg: Cfg,
    pub r: R,
}

fn bump(seq: u16) -> u16 {
    ((seq as u32 + 1) % SEQ_WRAP) as u16
}

impl<'a> Step<'a> {
    pub fn new(s: &'a mut Srv, cfg: Cfg) -> Step<'a> {
        Step { s, cfg, r: R::default() }
    }

    fn l(&mut self, name: &'static str) {
        self.r.l.push(name);
    }

    fn pu(&mut self, u: &str) -> &mut Pu {
        self.s.u.entry(u.to_string()).or_default()
    }

    fn part(&self, u: &str) -> P {
        self.s.u.get(u).map(|p| p.part).unwrap_or(P::NpIdle)
    }

    fn get(&self, u: &str) -> Pu {
        self.s.u.get(u).cloned().unwrap_or_default()
    }

    fn send(&mut self, u: &str, t: T, a: Option<String>, n: u32, d: u32) {
        let seqd = matches!(t, T::Taken | T::Idle);
        let bumped = seqd && d == 0;
        if bumped {
            self.s.seq = bump(self.s.seq);
        }
        let q = if bumped { self.s.seq } else if seqd { d as u16 } else { 0 };
        self.r.o.push(Out { to: u.to_string(), m: Msg { t, a, n, d: if seqd { 0 } else { d }, q } });
    }

    fn send_q(&mut self, u: &str, t: T, a: Option<String>, n: u32, d: u32, q: u16) {
        self.r.o.push(Out { to: u.to_string(), m: Msg { t, a, n, d, q } });
    }

    fn send_e(&mut self, u: &str, p: P, t: T, a: Option<String>, n: u32, d: u32) {
        self.enter(u, p);
        self.send(u, t, a, n, d);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_eq(&mut self, u: &str, p: P, t: T, a: Option<String>, n: u32, d: u32, q: u16) {
        self.enter(u, p);
        self.send_q(u, t, a, n, d, q);
    }

    fn fan(&mut self, ev: Msg, skip: Option<&str>) {
        let rest: Vec<String> = self.s.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
        let mut ev = ev;
        if matches!(ev.t, T::Taken | T::Idle) {
            self.s.seq = bump(self.s.seq);
            ev.q = self.s.seq;
        }
        for u in rest {
            self.part_a(&u, ev.clone());
        }
    }

    fn in_q(&self, u: &str) -> bool {
        self.s.queue.iter().any(|w| w.u == u)
    }

    fn pos(&self, u: &str) -> u32 {
        self.s.queue.iter().position(|w| w.u == u).map(|i| i as u32 + 1).unwrap_or(QPOS_NOT_IN_QUEUE as u32)
    }

    fn qprio(&self, u: &str) -> u32 {
        self.s.queue.iter().find(|w| w.u == u).map(|w| w.p).unwrap_or(0)
    }

    fn dequeue(&mut self, u: &str) {
        self.s.queue.retain(|w| w.u != u);
    }

    fn pre_q(&self) -> bool {
        self.s.queue.iter().any(|w| w.pre)
    }

    fn insert_q(&mut self, w: Waiter) {
        let before = |a: &Waiter, x: &Waiter| (a.pre && !x.pre) || (a.pre == x.pre && a.p > x.p);
        match self.s.queue.iter().position(|x| before(&w, x)) {
            Some(i) => self.s.queue.insert(i, w),
            None => self.s.queue.push(w),
        }
    }

    pub fn holder(s: &Srv) -> Option<String> {
        match s.g {
            G::Taken => s.speaker.clone(),
            G::Prevoke => s.revoking.clone(),
            G::Idle => None,
        }
    }

    fn holder_prio(&self) -> u32 {
        if self.s.g == G::Taken { self.s.sprio } else { self.s.rprio }
    }

    fn alone(&self) -> bool {
        self.s.members.len() <= 1
    }

    fn t2_left(&self) -> u32 {
        if self.s.first_rtp { self.cfg.t2_remain_secs } else { self.cfg.t2_secs }
    }

    fn enter_taken(&mut self) {
        self.l("FLR.S.G.TAKEN.enter");
        self.s.idle_bcast = false;
        let sp = self.s.speaker.clone().unwrap_or_default();
        let g = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.cfg.t2_secs, q: self.s.seq };
        self.part_a(&sp, g);
        self.fan(Msg::new(T::Taken, Some(sp.clone()), 0, 0), Some(&sp));
        self.r.fx.push(Fx::MediaBind(sp));
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
            let prev = self.s.prev.clone();
            self.fan(Msg::new(T::Idle, prev.clone(), 0, 0), None);
            self.r.fx.push(Fx::MediaRelease(prev));
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
        self.enter_taken();
    }

    fn prevoke_enter(&mut self, u: &str, cause: u32) {
        self.l("FLR.S.G.PREVOKE.enter");
        self.s.g = G::Prevoke;
        self.s.revoking = Some(u.to_string());
        self.s.rprio = self.s.sprio;
        self.s.rcause_g = cause;
        self.s.speaker = None;
        self.part_a(u, Msg::new(T::Revoke, None, cause, 0));
    }

    fn arb_req_idle(&mut self, u: &str, p: u32) {
        if self.s.g != G::Idle {
            self.l("FLR.S.G.any.req_discard");
            return;
        }
        if self.alone() {
            self.l("FLR.S.G.IDLE.req_alone");
            self.part_a(u, Msg::new(T::Deny, None, deny::ONLY_ONE as u32, 0));
            return;
        }
        if self.get(u).retry {
            self.l("FLR.S.G.IDLE.req_retry_after");
            self.part_a(u, Msg::new(T::Deny, None, deny::RETRY_AFTER as u32, 0));
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
            let m = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.t2_left(), q: self.s.seq };
            self.part_a(u, m);
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
            self.part_a(u, Msg::new(T::Qinfo, None, 1, p));
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
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
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
            self.s.rtp_seen = true;
            self.s.first_rtp = true;
            return;
        }
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
            self.l("FLR.S.G.PREVOKE.rtp");
            return;
        }
        self.l("FLR.S.G.any.rtp_discard");
    }

    pub fn server_rx(&mut self, u: &str, m: Msg) {
        let member = self.s.members.contains(u);
        if m.t == T::Ack && self.part(u) != P::NpTaken {
            self.l("FLR.S.gate.ack_drop");
            return;
        }
        if !member && m.t == T::Request {
            self.l("FLR.S.gate.not_in_room");
            self.send(u, T::Deny, None, deny::OTHER_REASON as u32, 0);
            return;
        }
        if !member {
            self.l("FLR.S.gate.not_in_room_drop");
            return;
        }
        let pu = self.get(u);
        if m.t == T::Request && !pu.half {
            self.l("FLR.S.gate.recv_only");
            self.send(u, T::Deny, None, deny::RECV_ONLY as u32, 0);
            return;
        }
        if m.t == T::Request && !pu.pub_ {
            self.l("FLR.S.gate.not_pubroom");
            self.send(u, T::Deny, None, deny::NOT_PUBROOM as u32, 0);
            return;
        }
        if m.t == T::Request && !pu.perm {
            self.l("FLR.S.gate.no_permission");
            self.send(u, T::Deny, None, deny::NO_PERMISSION as u32, 0);
            return;
        }
        self.part_c(u, m);
    }

    pub fn ready(&mut self, u: &str) {
        let h = Step::holder(self.s);
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
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.perm.speaker");
            self.pu(u).perm = false;
            self.prevoke_enter(u, revoke::NO_PERMISSION as u32);
            return;
        }
        if self.in_q(u) {
            self.l("FLR.S.perm.queued");
            self.pu(u).perm = false;
            self.dequeue(u);
            self.part_a(u, Msg::new(T::Deny, None, deny::NO_PERMISSION as u32, 0));
            return;
        }
        self.l("FLR.S.perm.other");
        self.pu(u).perm = false;
    }

    pub fn perm_up(&mut self, u: &str) {
        self.l("FLR.S.perm.up");
        self.pu(u).perm = true;
    }

    fn away(&mut self, u: &str) {
        if self.s.g == G::Taken && self.s.speaker.as_deref() == Some(u) {
            self.l("FLR.S.G.TAKEN.client_release");
            self.s.prev = Some(u.to_string());
            self.idle_enter();
            return;
        }
        if self.s.g == G::Prevoke && self.s.revoking.as_deref() == Some(u) {
            self.l("FLR.S.G.PREVOKE.client_release");
            return;
        }
        if self.in_q(u) {
            self.l("FLR.S.away.queued");
            self.dequeue(u);
            return;
        }
        self.l("FLR.S.away.other");
    }

    pub fn release(&mut self, u: &str) {
        self.away(u);
    }

    pub fn ws_pub_select(&mut self, u: &str) {
        self.l("FLR.S.ws.pub_select");
        self.pu(u).pub_ = true;
    }

    pub fn ws_reg_half(&mut self, u: &str) {
        self.l("FLR.S.ws.reg_half");
        self.pu(u).half = true;
    }

    pub fn ws_unreg_half(&mut self, u: &str) {
        self.pu(u).half = false;
    }

    pub fn ws_pub_away(&mut self, u: &str) {
        self.l("FLR.S.ws.pub_away");
        self.pu(u).pub_ = false;
        self.away(u);
    }

    pub fn ws_leave(&mut self, u: &str) {
        self.l("FLR.S.ws.leave");
        self.pu(u).pub_ = false;
        self.away(u);
        self.s.members.remove(u);
        let p = self.pu(u);
        p.retry = false;
        p.part = P::Releasing;
    }

    pub fn join(&mut self, u: &str, perm: bool, half: bool, is_pub: bool) {
        self.s.members.insert(u.to_string());
        let p = self.pu(u);
        let fresh = Pu { perm, half, pub_: is_pub, ..Pu::default() };
        let _ = std::mem::replace(p, fresh);
    }

    fn part_a(&mut self, u: &str, ev: Msg) {
        let st = self.part(u);
        match (st, ev.t) {
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
                self.send_eq(u, P::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (P::NpIdle, T::Taken) => {
                self.l("FLR.S.P.NP_IDLE.a_TAKEN");
                self.pu(u).tq = ev.q;
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpIdle, T::Idle) => {
                self.l("FLR.S.P.NP_IDLE.a_IDLE");
                self.send(u, T::Idle, ev.a.clone(), 0, ev.q as u32);
            }
            (P::NpIdle, _) => self.l("FLR.S.P.NP_IDLE.a_other"),
            (P::NpTaken, T::Granted) => {
                self.l("FLR.S.P.NP_TAKEN.a_GRANTED");
                self.send_eq(u, P::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (P::NpTaken, T::Taken) => {
                self.l("FLR.S.P.NP_TAKEN.a_TAKEN");
                let p = self.pu(u);
                p.tq = ev.q;
                p.c21 = 1;
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
                self.pu(u).tq = ev.q;
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::Permitted, T::Revoke) => {
                self.l("FLR.S.P.PERMITTED.a_REVOKE");
                self.pu(u).rcause = ev.n;
                self.send_e(u, P::Prevoke, T::Revoke, None, ev.n, 0);
            }
            (P::Permitted, _) => self.l("FLR.S.P.PERMITTED.a_other"),
            (P::Prevoke, T::Idle) => {
                self.l("FLR.S.P.PREVOKE.a_IDLE");
                self.send_e(u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q as u32);
            }
            (P::Prevoke, T::Taken) => {
                self.l("FLR.S.P.PREVOKE.a_TAKEN");
                self.pu(u).tq = ev.q;
                self.send_e(u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q as u32);
            }
            (P::Prevoke, _) => self.l("FLR.S.P.PREVOKE.a_other"),
            (P::Releasing, _) => self.l("FLR.S.P.RELEASING.a_any"),
            (P::Npsends, T::Idle) => {
                self.l("FLR.S.P.NPSENDS.a_IDLE");
                self.pu(u).g_taken = false;
            }
            (P::Npsends, T::Taken) => {
                self.l("FLR.S.P.NPSENDS.a_TAKEN");
                self.pu(u).g_taken = true;
            }
            (P::Npsends, _) => self.l("FLR.S.P.NPSENDS.a_other"),
        }
    }

    fn part_c(&mut self, u: &str, ev: Msg) {
        let pu = self.get(u);
        match (pu.part, ev.t) {
            (P::NpIdle, T::Request) => {
                self.l("FLR.S.P.NP_IDLE.rx_REQUEST");
                self.arb_req_idle(u, ev.n);
            }
            (P::NpIdle, T::Release) => {
                self.l("FLR.S.P.NP_IDLE.rx_RELEASE");
                self.dequeue(u);
                let prev = self.s.prev.clone();
                self.send(u, T::Idle, prev, 0, 0);
            }
            (P::NpIdle, T::Rtp) if pu.released => {
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
                let next = bump(self.s.seq);
                let p = self.pu(u);
                p.tq = next;
                p.c21 = 1;
                self.dequeue(u);
                let h = Step::holder(self.s);
                self.send(u, T::Taken, h, 0, 0);
            }
            (P::NpTaken, T::Ack) if ev.n == 2 && ev.q == pu.tq => {
                self.l("FLR.S.P.NP_TAKEN.rx_ACK_taken");
                self.pu(u).c21 = 0;
            }
            (P::NpTaken, T::Ack) if ev.n == 2 => self.l("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale"),
            (P::NpTaken, T::Ack) => self.l("FLR.S.P.NP_TAKEN.rx_ACK_other"),
            (P::NpTaken, T::QposReq) => {
                self.l("FLR.S.P.NP_TAKEN.rx_QPOS_REQ");
                let (pos, pr) = (self.pos(u), self.qprio(u));
                self.send(u, T::Qinfo, None, pos, pr);
            }
            (P::NpTaken, T::Rtp) => {
                self.l("FLR.S.P.NP_TAKEN.rx_RTP");
                self.revoke3(u);
            }
            (P::NpTaken, _) => self.l("FLR.S.P.NP_TAKEN.rx_other"),
            (P::Permitted, T::Release) => {
                self.l("FLR.S.P.PERMITTED.rx_RELEASE");
                self.pu(u).released = true;
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
            (P::Prevoke, T::Release) => {
                self.l("FLR.S.P.PREVOKE.rx_RELEASE");
                self.arb_rel(u);
            }
            (P::Prevoke, T::Rtp) => {
                self.l("FLR.S.P.PREVOKE.rx_RTP");
                self.arb_rtp(u);
            }
            (P::Prevoke, _) => self.l("FLR.S.P.PREVOKE.rx_other"),
            (_, T::Release) if pu.g_taken => {
                self.l("FLR.S.P.NPSENDS.rx_RELEASE_taken");
                self.pu(u).tq = bump(self.s.seq);
                let h = Step::holder(self.s);
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
        let label = match p {
            P::NpIdle => "FLR.S.P.NP_IDLE.enter",
            P::NpTaken => "FLR.S.P.NP_TAKEN.enter",
            P::Permitted => "FLR.S.P.PERMITTED.enter",
            P::Prevoke => "FLR.S.P.PREVOKE.enter",
            _ => "FLR.S.P.NPSENDS.enter",
        };
        self.l(label);
        let x = self.pu(u);
        x.part = p;
        match p {
            P::NpIdle => {
                x.g_taken = false;
                x.c21 = 0;
            }
            P::NpTaken => {
                x.g_taken = true;
                x.c21 = 1;
            }
            P::Permitted => {
                x.released = false;
                x.c21 = 0;
            }
            _ => {
                x.c8 = 1;
                x.c21 = 0;
            }
        }
    }

    fn revoke3(&mut self, u: &str) {
        self.pu(u).rcause = revoke::NO_PERMISSION as u32;
        self.send_e(u, P::Npsends, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
    }

    fn taken_req(&mut self, u: &str, p: u32) {
        if self.get(u).retry {
            self.l("FLR.S.P.NP_TAKEN.req_retry_after");
            self.send(u, T::Deny, None, deny::RETRY_AFTER as u32, 0);
            return;
        }
        if self.in_q(u) {
            self.l("FLR.S.P.NP_TAKEN.req_renotify");
            let (pos, pr) = (self.pos(u), self.qprio(u));
            self.send(u, T::Qinfo, None, pos, pr);
            return;
        }
        if p > self.holder_prio() && !self.pre_q() {
            self.l("FLR.S.P.NP_TAKEN.req_preempt");
            self.arb_preempt(u, p);
            return;
        }
        if p > self.holder_prio() {
            self.l("FLR.S.P.NP_TAKEN.req_preempt_blocked");
            return;
        }
        if self.s.queue.len() >= self.cfg.qmax {
            self.l("FLR.S.P.NP_TAKEN.req_full");
            self.send(u, T::Deny, None, deny::QUEUE_FULL as u32, 0);
            return;
        }
        self.l("FLR.S.P.NP_TAKEN.req_enqueue");
        self.insert_q(Waiter { u: u.to_string(), p, pre: false });
        let pos = self.pos(u);
        self.send(u, T::Qinfo, None, pos, p);
    }

    pub fn t1(&mut self) {
        if self.s.g == G::Taken {
            self.l("FLR.S.G.TAKEN.T1");
            self.s.prev = self.s.speaker.clone();
        } else {
            self.l("FLR.S.G.PREVOKE.T1");
            self.s.prev = self.s.revoking.clone();
        }
        self.idle_enter();
    }

    pub fn t2(&mut self) {
        self.l("FLR.S.G.TAKEN.T2");
        let u = self.s.speaker.clone().unwrap_or_default();
        self.pu(&u).retry = true;
        self.prevoke_enter(&u, revoke::TOO_LONG as u32);
    }

    pub fn t3(&mut self) {
        self.l("FLR.S.G.PREVOKE.T3");
        self.s.prev = self.s.revoking.clone();
        self.idle_enter();
    }

    pub fn t7(&mut self) {
        self.l("FLR.S.G.IDLE.T7");
        self.s.c7 += 1;
        let prev = self.s.prev.clone();
        self.fan(Msg::new(T::Idle, prev, 0, 0), None);
    }

    pub fn t20(&mut self) {
        if self.s.c20 < C20 {
            self.l("FLR.S.G.TAKEN.T20");
            self.s.c20 += 1;
            let sp = self.s.speaker.clone().unwrap_or_default();
            let m = Msg { t: T::Granted, a: None, n: self.s.sprio, d: self.cfg.t2_secs, q: self.s.seq };
            self.part_a(&sp, m);
            return;
        }
        self.l("FLR.S.G.TAKEN.T20_exhaust");
        self.s.c20 += 1;
    }

    pub fn t8(&mut self, u: &str) {
        let pu = self.get(u);
        if pu.part == P::Prevoke {
            self.l("FLR.S.P.PREVOKE.T8");
            self.pu(u).c8 += 1;
            self.send(u, T::Revoke, None, pu.rcause, 0);
            return;
        }
        self.l("FLR.S.P.NPSENDS.T8");
        self.pu(u).c8 += 1;
        self.send(u, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
    }

    pub fn t21(&mut self, u: &str) {
        let pu = self.get(u);
        if pu.c21 < C21 {
            self.l("FLR.S.P.NP_TAKEN.T21");
            self.pu(u).c21 += 1;
            let h = Step::holder(self.s);
            self.send(u, T::Taken, h, 0, pu.tq as u32);
            return;
        }
        self.l("FLR.S.P.NP_TAKEN.T21_exhaust");
        self.pu(u).c21 += 1;
    }

    pub fn t9(&mut self, u: &str) {
        self.l("FLR.S.T9.expire");
        self.pu(u).retry = false;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tm {
    T1,
    T2,
    T3,
    T7,
    T20,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tp {
    T8,
    T9,
    T21,
}

pub fn armed_server(s: &Srv) -> Vec<Tm> {
    let mut on = Vec::new();
    if matches!(s.g, G::Taken | G::Prevoke) {
        on.push(Tm::T1);
    }
    if s.g == G::Taken && s.first_rtp {
        on.push(Tm::T2);
    }
    if s.g == G::Prevoke {
        on.push(Tm::T3);
    }
    if s.g == G::Idle && s.idle_bcast && s.c7 < C7 {
        on.push(Tm::T7);
    }
    if s.g == G::Taken && s.succ && !s.rtp_seen && s.c20 <= C20 {
        on.push(Tm::T20);
    }
    on
}

pub fn armed_participant(s: &Srv, u: &str) -> Vec<Tp> {
    let p = s.u.get(u).cloned().unwrap_or_default();
    let mut on = Vec::new();
    if matches!(p.part, P::Prevoke | P::Npsends) && p.c8 < C8 {
        on.push(Tp::T8);
    }
    if p.retry {
        on.push(Tp::T9);
    }
    if p.part == P::NpTaken && p.c21 >= 1 && p.c21 <= C21 {
        on.push(Tp::T21);
    }
    on
}

pub fn newer(a: u16, b: u16) -> bool {
    a != b && ((a as u32 + SEQ_WRAP - b as u32) % SEQ_WRAP) < SEQ_WRAP / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Cfg {
        Cfg { qmax: 10, t2_secs: 30, t2_remain_secs: 30 }
    }

    fn room(users: &[&str]) -> Srv {
        let mut s = Srv::default();
        for u in users {
            let mut st = Step::new(&mut s, cfg());
            st.join(u, true, true, true);
        }
        s
    }

    #[test]
    fn grant_taken_release_idle() {
        let mut s = room(&["a", "b"]);
        let mut st = Step::new(&mut s, cfg());
        st.server_rx("a", Msg::new(T::Request, None, 0, 0));
        let r = st.r;
        assert!(r.l.contains(&"FLR.S.G.IDLE.req_grant"));
        assert_eq!(r.o[0].to, "a");
        assert_eq!(r.o[0].m.t, T::Granted);
        assert_eq!(r.o[1].to, "b");
        assert_eq!(r.o[1].m.t, T::Taken);
        assert_eq!(r.o[1].m.q, 1, "방송 한 번 = 일련번호 한 번");
        assert_eq!(r.fx, vec![Fx::MediaBind("a".into())]);
        assert_eq!(s.g, G::Taken);
        assert_eq!(armed_server(&s), vec![Tm::T1]);
        let mut st = Step::new(&mut s, cfg());
        st.server_rx("a", Msg::new(T::Release, None, 0, 0));
        let r = st.r;
        assert!(r.l.contains(&"FLR.S.G.IDLE.enter_empty"));
        assert_eq!(r.fx, vec![Fx::MediaRelease(Some("a".into()))]);
        assert_eq!(s.g, G::Idle);
        assert!(armed_server(&s).contains(&Tm::T7));
    }

    #[test]
    fn gate_and_queue() {
        let mut s = room(&["a", "b", "c"]);
        let mut st = Step::new(&mut s, cfg());
        st.server_rx("x", Msg::new(T::Request, None, 0, 0));
        assert_eq!(st.r.o[0].m.n, deny::OTHER_REASON as u32);
        st.server_rx("a", Msg::new(T::Request, None, 0, 0));
        st.server_rx("b", Msg::new(T::Request, None, 0, 0));
        assert!(st.r.l.contains(&"FLR.S.P.NP_TAKEN.req_enqueue"));
        st.server_rx("c", Msg::new(T::Request, None, 5, 0));
        assert!(st.r.l.contains(&"FLR.S.G.TAKEN.preempt"), "높은 우선순위는 선점");
        assert_eq!(s.g, G::Prevoke);
        let mut st = Step::new(&mut s, cfg());
        st.t3();
        assert_eq!(s.speaker.as_deref(), Some("c"), "회수 유예 끝에 선점자가 승계");
        assert!(s.succ);
    }

    #[test]
    fn ready_notifies_once() {
        let mut s = room(&["a", "b"]);
        let mut st = Step::new(&mut s, cfg());
        st.ready("b");
        assert_eq!(st.r.o[0].m.t, T::Idle);
        st.server_rx("a", Msg::new(T::Request, None, 0, 0));
        st.ready("b");
        assert!(st.r.l.contains(&"FLR.S.P.join.taken"));
        st.ready("a");
        assert!(st.r.l.contains(&"FLR.S.P.join.self"));
    }
}
