// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:FLR.S.G · 연:FLR.S.P · 연:FLR.S.gate · 연:FLR.S.perm · 연:FLR.S.ws · 연:RULE-FLR-TIMERS · 정:IMPL-FLR-LOCK · 정:RULE-CONC-SIX · 정:CAND-FLR-MEMBER-JOIN-INIT · 연:RULE-FLR-SEQ · 연:RULE-FLR-SERVER-STATE-SEPARATE · 연:RULE-FLR-MSG-IDLE · 연:RULE-FLR-MSG-REVOKE · 연:RULE-FLR-MSG-RELEASE · 연:RULE-FLR-SERVER-POLICY · 연:DEV-FLR-PUBROOM · 연:DEV-FLR-QUEUE-MAX · 연:DEV-FLR-NO-MCPRIO · 연:DEV-FLR-PREEMPT-RELATIVE · 연:DEV-FLR-PUB-AWAY · 연:DEV-FLR-RECVONLY-TRIGGER · 연:DEV-FLR-DENY-2-6 · 연:DEV-FLR-REGRANT-FWD · 연:DEV-FLR-TAKEN-ACK · 연:DEV-FLR-ACK-DROP · 연:DEV-FLR-T21-SAME-SEQ · 연:DEV-FLR-GRANTED-SEQ · 연:DEV-FLR-ACK-SEQ · 연:DEV-FLR-ROOM-GONE · model: claude-opus-5-5
//! 발언권 판정 로직 — 생성 서버 상태기(`protocol/out/floor/server/floor_server.ts`)의 이식 · 걸음마다 보낼 것 · 지난 경우 라벨 · 효과를 돌려준다(판정은 락 안 · 집행은 락 밖).

use std::collections::{BTreeMap, BTreeSet};

use oxsig::floor::{deny, qpos, revoke, timers};

pub const SEQ_WRAP: u32 = 65_536;

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
pub enum P {
    NpIdle,
    NpTaken,
    Permitted,
    Prevoke,
    NpSends,
    Releasing,
}

impl P {
    pub fn as_str(self) -> &'static str {
        match self {
            P::NpIdle => "NP_IDLE",
            P::NpTaken => "NP_TAKEN",
            P::Permitted => "PERMITTED",
            P::Prevoke => "PREVOKE",
            P::NpSends => "NPSENDS",
            P::Releasing => "RELEASING",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mt {
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
    pub t: Mt,
    pub a: Option<String>,
    pub n: u32,
    pub d: u32,
    pub q: u32,
}

impl Msg {
    pub fn new(t: Mt, a: Option<String>, n: u32, d: u32) -> Msg {
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

#[derive(Debug, Clone, Default)]
pub struct Step {
    pub out: Vec<Out>,
    pub labels: Vec<&'static str>,
    pub fx: Vec<Fx>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Part {
    pub part: Option<P>,
    pub pub_room: bool,
    pub half: bool,
    pub perm: bool,
    pub retry: bool,
    pub released: bool,
    pub g_taken: bool,
    pub c8: u32,
    pub c21: u32,
    pub tq: u32,
    pub rcause: u32,
}

#[derive(Debug, Clone)]
pub struct Floor {
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
    pub parts: BTreeMap<String, Part>,
    pub seq: u32,
    pub qmax: usize,
    pub t2_secs: u32,
}

impl Floor {
    pub fn new(qmax: usize, t2_secs: u32) -> Floor {
        Floor {
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
            parts: BTreeMap::new(),
            seq: 0,
            qmax,
            t2_secs,
        }
    }

    fn p(&mut self, u: &str) -> &mut Part {
        self.parts.entry(u.to_string()).or_default()
    }

    fn part_of(&self, u: &str) -> Option<P> {
        self.parts.get(u).and_then(|p| p.part)
    }

    pub fn holder(&self) -> Option<String> {
        match self.g {
            G::Taken => self.speaker.clone(),
            G::Prevoke => self.revoking.clone(),
            G::Idle => None,
        }
    }

    fn holder_prio(&self) -> u32 {
        if self.g == G::Taken { self.sprio } else { self.rprio }
    }

    fn alone(&self) -> bool {
        self.members.len() <= 1
    }

    fn in_q(&self, u: &str) -> bool {
        self.queue.iter().any(|w| w.u == u)
    }

    fn pos(&self, u: &str) -> u32 {
        self.queue.iter().position(|w| w.u == u).map(|i| i as u32 + 1).unwrap_or(qpos::NOT_IN_QUEUE as u32)
    }

    fn qprio(&self, u: &str) -> u32 {
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
        match self.queue.iter().position(|x| before(&w, x)) {
            Some(i) => self.queue.insert(i, w),
            None => self.queue.push(w),
        }
    }

    fn t2_full(&self) -> u32 {
        self.t2_secs
    }

    fn send(&mut self, r: &mut Step, u: &str, t: Mt, a: Option<String>, n: u32, d: u32) {
        let seqd = matches!(t, Mt::Taken | Mt::Idle);
        let bump = seqd && d == 0;
        if bump {
            self.seq = (self.seq + 1) % SEQ_WRAP;
        }
        let q = if bump {
            self.seq
        } else if seqd {
            d
        } else {
            0
        };
        r.out.push(Out { to: u.to_string(), m: Msg { t, a, n, d: if seqd { 0 } else { d }, q } });
    }

    fn send_q(&mut self, r: &mut Step, u: &str, t: Mt, n: u32, d: u32, q: u32) {
        r.out.push(Out { to: u.to_string(), m: Msg { t, a: None, n, d, q } });
    }

    fn fan(&mut self, r: &mut Step, ev: Msg, skip: Option<&str>) {
        let rest: Vec<String> = self.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
        let mut ev = ev;
        if matches!(ev.t, Mt::Taken | Mt::Idle) {
            self.seq = (self.seq + 1) % SEQ_WRAP;
            ev.q = self.seq;
        }
        for u in rest {
            self.part_a(r, &u, &ev);
        }
    }

    fn enter_taken(&mut self, r: &mut Step) {
        r.labels.push("FLR.S.G.TAKEN.enter");
        self.idle_bcast = false;
        let sp = self.speaker.clone().unwrap_or_default();
        let m = Msg { t: Mt::Granted, a: None, n: self.sprio, d: self.t2_full(), q: self.seq };
        self.part_a(r, &sp, &m);
        self.fan(r, Msg::new(Mt::Taken, Some(sp.clone()), 0, 0), Some(&sp));
        r.fx.push(Fx::Bind(sp));
    }

    fn idle_enter(&mut self, r: &mut Step) {
        if self.queue.is_empty() {
            r.labels.push("FLR.S.G.IDLE.enter_empty");
            self.g = G::Idle;
            self.speaker = None;
            self.revoking = None;
            self.first_rtp = false;
            self.idle_bcast = true;
            self.c7 = 1;
            let prev = self.prev.clone();
            self.fan(r, Msg::new(Mt::Idle, prev.clone(), 0, 0), None);
            r.fx.push(Fx::Release(prev));
            return;
        }
        r.labels.push("FLR.S.G.IDLE.enter_succession");
        let w = self.queue.remove(0);
        self.g = G::Taken;
        self.speaker = Some(w.u);
        self.sprio = w.p;
        self.revoking = None;
        self.succ = true;
        self.rtp_seen = false;
        self.first_rtp = false;
        self.c20 = 1;
        self.enter_taken(r);
    }

    fn prevoke_enter(&mut self, r: &mut Step, u: &str, cause: u32) {
        r.labels.push("FLR.S.G.PREVOKE.enter");
        self.g = G::Prevoke;
        self.revoking = Some(u.to_string());
        self.rprio = self.sprio;
        self.rcause_g = cause;
        self.speaker = None;
        self.part_a(r, u, &Msg::new(Mt::Revoke, None, cause, 0));
    }

    fn arb_req_idle(&mut self, r: &mut Step, u: &str, p: u32) {
        if self.g != G::Idle {
            r.labels.push("FLR.S.G.any.req_discard");
            return;
        }
        if self.alone() {
            r.labels.push("FLR.S.G.IDLE.req_alone");
            self.part_a(r, u, &Msg::new(Mt::Deny, None, deny::ONLY_ONE as u32, 0));
            return;
        }
        if self.parts.get(u).map(|x| x.retry).unwrap_or(false) {
            r.labels.push("FLR.S.G.IDLE.req_retry_after");
            self.part_a(r, u, &Msg::new(Mt::Deny, None, deny::RETRY_AFTER as u32, 0));
            return;
        }
        r.labels.push("FLR.S.G.IDLE.req_grant");
        self.g = G::Taken;
        self.speaker = Some(u.to_string());
        self.sprio = p;
        self.succ = false;
        self.rtp_seen = false;
        self.first_rtp = false;
        self.c20 = 0;
        self.enter_taken(r);
    }

    fn arb_req_permitted(&mut self, r: &mut Step, u: &str, t2_left: u32) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.TAKEN.req_regrant");
            let d = if self.first_rtp { t2_left } else { self.t2_full() };
            let m = Msg { t: Mt::Granted, a: None, n: self.sprio, d, q: self.seq };
            self.part_a(r, u, &m);
            return;
        }
        r.labels.push("FLR.S.G.any.regrant_discard");
    }

    fn arb_preempt(&mut self, r: &mut Step, u: &str, p: u32) {
        if self.g == G::Taken {
            r.labels.push("FLR.S.G.TAKEN.preempt");
            self.queue.insert(0, Waiter { u: u.to_string(), p, pre: true });
            let sp = self.speaker.clone().unwrap_or_default();
            self.prevoke_enter(r, &sp, revoke::PREEMPTED as u32);
            self.part_a(r, u, &Msg::new(Mt::Qinfo, None, 1, p));
            return;
        }
        r.labels.push("FLR.S.G.PREVOKE.preempt_discard");
    }

    fn arb_rel(&mut self, r: &mut Step, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.TAKEN.rel_speaker");
            self.prev = Some(u.to_string());
            self.idle_enter(r);
            return;
        }
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.PREVOKE.rel_revoked");
            self.prev = Some(u.to_string());
            self.idle_enter(r);
            return;
        }
        r.labels.push("FLR.S.G.any.rel_discard");
    }

    fn arb_rtp(&mut self, r: &mut Step, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.TAKEN.rtp");
            self.rtp_seen = true;
            self.first_rtp = true;
            return;
        }
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.PREVOKE.rtp");
            return;
        }
        r.labels.push("FLR.S.G.any.rtp_discard");
    }

    pub fn server_rx(&mut self, u: &str, m: &Msg, t2_left: u32) -> Step {
        let mut r = Step::default();
        if m.t == Mt::Ack && self.part_of(u) != Some(P::NpTaken) {
            r.labels.push("FLR.S.gate.ack_drop");
            return r;
        }
        if !self.members.contains(u) && m.t == Mt::Request {
            r.labels.push("FLR.S.gate.not_in_room");
            self.send(&mut r, u, Mt::Deny, None, deny::OTHER_REASON as u32, 0);
            return r;
        }
        if !self.members.contains(u) {
            r.labels.push("FLR.S.gate.not_in_room_drop");
            return r;
        }
        let pp = self.parts.get(u).cloned().unwrap_or_default();
        if m.t == Mt::Request && !pp.half {
            r.labels.push("FLR.S.gate.recv_only");
            self.send(&mut r, u, Mt::Deny, None, deny::RECV_ONLY as u32, 0);
            return r;
        }
        if m.t == Mt::Request && !pp.pub_room {
            r.labels.push("FLR.S.gate.not_pubroom");
            self.send(&mut r, u, Mt::Deny, None, deny::NOT_PUBROOM as u32, 0);
            return r;
        }
        if m.t == Mt::Request && !pp.perm {
            r.labels.push("FLR.S.gate.no_permission");
            self.send(&mut r, u, Mt::Deny, None, deny::NO_PERMISSION as u32, 0);
            return r;
        }
        self.part_c(&mut r, u, m, t2_left);
        r
    }

    pub fn ready(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        let h = self.holder();
        if h.as_deref() == Some(u) {
            r.labels.push("FLR.S.P.join.self");
        } else if h.is_some() {
            r.labels.push("FLR.S.P.join.taken");
            self.send(&mut r, u, Mt::Taken, h, 0, 0);
        } else {
            r.labels.push("FLR.S.P.join.idle");
            let prev = self.prev.clone();
            self.send(&mut r, u, Mt::Idle, prev, 0, 0);
        }
        r
    }

    pub fn perm_down(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        self.p(u).perm = false;
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.labels.push("FLR.S.perm.speaker");
            self.prevoke_enter(&mut r, u, revoke::NO_PERMISSION as u32);
        } else if self.in_q(u) {
            r.labels.push("FLR.S.perm.queued");
            self.dequeue(u);
            self.part_a(&mut r, u, &Msg::new(Mt::Deny, None, deny::NO_PERMISSION as u32, 0));
        } else {
            r.labels.push("FLR.S.perm.other");
        }
        r
    }

    pub fn perm_up(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.perm.up");
        self.p(u).perm = true;
        r
    }

    fn away(&mut self, r: &mut Step, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.TAKEN.client_release");
            self.prev = Some(u.to_string());
            self.idle_enter(r);
        } else if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.labels.push("FLR.S.G.PREVOKE.client_release");
        } else if self.in_q(u) {
            r.labels.push("FLR.S.away.queued");
            self.dequeue(u);
        } else {
            r.labels.push("FLR.S.away.other");
        }
    }

    pub fn ws_pub_select(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.ws.pub_select");
        self.p(u).pub_room = true;
        r
    }

    pub fn ws_reg_half(&mut self, u: &str, half: bool) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.ws.reg_half");
        self.p(u).half = half;
        r
    }

    pub fn ws_pub_away(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.ws.pub_away");
        self.p(u).pub_room = false;
        self.away(&mut r, u);
        r
    }

    pub fn ws_leave(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.ws.leave");
        self.p(u).pub_room = false;
        self.away(&mut r, u);
        self.members.remove(u);
        let p = self.p(u);
        p.retry = false;
        p.part = Some(P::Releasing);
        r
    }

    pub fn ws_join(&mut self, u: &str, half: bool, perm: bool) -> Step {
        let mut r = Step::default();
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.labels.push("FLR.S.ws.join_revoking");
            self.prev = self.revoking.clone();
            self.idle_enter(&mut r);
        } else {
            r.labels.push("FLR.S.ws.join");
        }
        self.members.insert(u.to_string());
        let taken = self.holder().is_some();
        let p = self.p(u);
        p.part = Some(if taken { P::NpTaken } else { P::NpIdle });
        p.g_taken = taken;
        p.pub_room = false;
        p.half = half;
        p.perm = perm;
        p.retry = false;
        p.released = false;
        p.tq = 0;
        p.rcause = 0;
        r
    }

    fn enter_r(&mut self, r: &mut Step, u: &str, to: P) {
        let p = self.p(u);
        match to {
            P::NpIdle => {
                r.labels.push("FLR.S.P.NP_IDLE.enter");
                p.g_taken = false;
                p.c21 = 0;
            }
            P::NpTaken => {
                r.labels.push("FLR.S.P.NP_TAKEN.enter");
                p.g_taken = true;
                p.c21 = 1;
            }
            P::Permitted => {
                r.labels.push("FLR.S.P.PERMITTED.enter");
                p.released = false;
                p.c21 = 0;
            }
            P::Prevoke => {
                r.labels.push("FLR.S.P.PREVOKE.enter");
                p.c8 = 1;
                p.c21 = 0;
            }
            _ => {
                r.labels.push("FLR.S.P.NPSENDS.enter");
                p.c8 = 1;
                p.c21 = 0;
            }
        }
        p.part = Some(to);
    }

    fn send_e(&mut self, r: &mut Step, u: &str, to: P, m: Msg) {
        self.enter_r(r, u, to);
        self.send(r, u, m.t, m.a, m.n, m.d);
    }

    fn send_eq(&mut self, r: &mut Step, u: &str, to: P, m: Msg) {
        self.enter_r(r, u, to);
        self.send_q(r, u, m.t, m.n, m.d, m.q);
    }

    fn part_a(&mut self, r: &mut Step, u: &str, ev: &Msg) {
        let st = self.part_of(u);
        match (ev.t, st) {
            (Mt::Qinfo, _) => {
                r.labels.push("FLR.S.P.any.a_QINFO");
                self.send(r, u, Mt::Qinfo, None, ev.n, ev.d);
            }
            (Mt::Deny, _) => {
                r.labels.push("FLR.S.P.any.a_DENY");
                self.send(r, u, Mt::Deny, None, ev.n, 0);
            }
            (Mt::Granted, Some(P::NpIdle)) => {
                r.labels.push("FLR.S.P.NP_IDLE.a_GRANTED");
                self.send_eq(r, u, P::Permitted, Msg { t: Mt::Granted, a: None, n: ev.n, d: ev.d, q: ev.q });
            }
            (Mt::Taken, Some(P::NpIdle)) => {
                r.labels.push("FLR.S.P.NP_IDLE.a_TAKEN");
                self.p(u).tq = ev.q;
                self.send_e(r, u, P::NpTaken, Msg::new(Mt::Taken, ev.a.clone(), 0, ev.q));
            }
            (Mt::Idle, Some(P::NpIdle)) => {
                r.labels.push("FLR.S.P.NP_IDLE.a_IDLE");
                self.send(r, u, Mt::Idle, ev.a.clone(), 0, ev.q);
            }
            (_, Some(P::NpIdle)) => r.labels.push("FLR.S.P.NP_IDLE.a_other"),
            (Mt::Granted, Some(P::NpTaken)) => {
                r.labels.push("FLR.S.P.NP_TAKEN.a_GRANTED");
                self.send_eq(r, u, P::Permitted, Msg { t: Mt::Granted, a: None, n: ev.n, d: ev.d, q: ev.q });
            }
            (Mt::Taken, Some(P::NpTaken)) => {
                r.labels.push("FLR.S.P.NP_TAKEN.a_TAKEN");
                let p = self.p(u);
                p.tq = ev.q;
                p.c21 = 1;
                self.send(r, u, Mt::Taken, ev.a.clone(), 0, ev.q);
            }
            (Mt::Idle, Some(P::NpTaken)) => {
                r.labels.push("FLR.S.P.NP_TAKEN.a_IDLE");
                self.send_e(r, u, P::NpIdle, Msg::new(Mt::Idle, ev.a.clone(), 0, ev.q));
            }
            (_, Some(P::NpTaken)) => r.labels.push("FLR.S.P.NP_TAKEN.a_other"),
            (Mt::Granted, Some(P::Permitted)) => {
                r.labels.push("FLR.S.P.PERMITTED.a_GRANTED");
                self.send_q(r, u, Mt::Granted, ev.n, ev.d, ev.q);
            }
            (Mt::Idle, Some(P::Permitted)) => {
                r.labels.push("FLR.S.P.PERMITTED.a_IDLE");
                self.enter_r(r, u, P::NpIdle);
            }
            (Mt::Taken, Some(P::Permitted)) => {
                r.labels.push("FLR.S.P.PERMITTED.a_TAKEN");
                self.p(u).tq = ev.q;
                self.send_e(r, u, P::NpTaken, Msg::new(Mt::Taken, ev.a.clone(), 0, ev.q));
            }
            (Mt::Revoke, Some(P::Permitted)) => {
                r.labels.push("FLR.S.P.PERMITTED.a_REVOKE");
                self.p(u).rcause = ev.n;
                self.send_e(r, u, P::Prevoke, Msg::new(Mt::Revoke, None, ev.n, 0));
            }
            (_, Some(P::Permitted)) => r.labels.push("FLR.S.P.PERMITTED.a_other"),
            (Mt::Idle, Some(P::Prevoke)) => {
                r.labels.push("FLR.S.P.PREVOKE.a_IDLE");
                self.send_e(r, u, P::NpIdle, Msg::new(Mt::Idle, ev.a.clone(), 0, ev.q));
            }
            (Mt::Taken, Some(P::Prevoke)) => {
                r.labels.push("FLR.S.P.PREVOKE.a_TAKEN");
                self.p(u).tq = ev.q;
                self.send_e(r, u, P::NpTaken, Msg::new(Mt::Taken, ev.a.clone(), 0, ev.q));
            }
            (_, Some(P::Prevoke)) => r.labels.push("FLR.S.P.PREVOKE.a_other"),
            (_, Some(P::Releasing)) => r.labels.push("FLR.S.P.RELEASING.a_any"),
            (Mt::Idle, _) => {
                r.labels.push("FLR.S.P.NPSENDS.a_IDLE");
                self.p(u).g_taken = false;
            }
            (Mt::Taken, _) => {
                r.labels.push("FLR.S.P.NPSENDS.a_TAKEN");
                self.p(u).g_taken = true;
            }
            _ => r.labels.push("FLR.S.P.NPSENDS.a_other"),
        }
    }

    fn revoke3(&mut self, r: &mut Step, u: &str) {
        self.p(u).rcause = revoke::NO_PERMISSION as u32;
        self.send_e(r, u, P::NpSends, Msg::new(Mt::Revoke, None, revoke::NO_PERMISSION as u32, 0));
    }

    fn taken_req(&mut self, r: &mut Step, u: &str, p: u32) {
        if self.parts.get(u).map(|x| x.retry).unwrap_or(false) {
            r.labels.push("FLR.S.P.NP_TAKEN.req_retry_after");
            self.send(r, u, Mt::Deny, None, deny::RETRY_AFTER as u32, 0);
        } else if self.in_q(u) {
            r.labels.push("FLR.S.P.NP_TAKEN.req_renotify");
            let (pos, qp) = (self.pos(u), self.qprio(u));
            self.send(r, u, Mt::Qinfo, None, pos, qp);
        } else if p > self.holder_prio() && !self.pre_q() {
            r.labels.push("FLR.S.P.NP_TAKEN.req_preempt");
            self.arb_preempt(r, u, p);
        } else if p > self.holder_prio() {
            r.labels.push("FLR.S.P.NP_TAKEN.req_preempt_blocked");
        } else if self.queue.len() >= self.qmax {
            r.labels.push("FLR.S.P.NP_TAKEN.req_full");
            self.send(r, u, Mt::Deny, None, deny::QUEUE_FULL as u32, 0);
        } else {
            r.labels.push("FLR.S.P.NP_TAKEN.req_enqueue");
            self.insert_q(Waiter { u: u.to_string(), p, pre: false });
            let pos = self.pos(u);
            self.send(r, u, Mt::Qinfo, None, pos, p);
        }
    }

    fn part_c(&mut self, r: &mut Step, u: &str, ev: &Msg, t2_left: u32) {
        let st = self.part_of(u);
        let pp = self.parts.get(u).cloned().unwrap_or_default();
        match (st, ev.t) {
            (Some(P::NpIdle), Mt::Request) => {
                r.labels.push("FLR.S.P.NP_IDLE.rx_REQUEST");
                self.arb_req_idle(r, u, ev.n);
            }
            (Some(P::NpIdle), Mt::Release) => {
                r.labels.push("FLR.S.P.NP_IDLE.rx_RELEASE");
                self.dequeue(u);
                let prev = self.prev.clone();
                self.send(r, u, Mt::Idle, prev, 0, 0);
            }
            (Some(P::NpIdle), Mt::Rtp) if pp.released => {
                r.labels.push("FLR.S.P.NP_IDLE.rx_RTP_after_release");
                self.revoke3(r, u);
            }
            (Some(P::NpIdle), Mt::Rtp) => r.labels.push("FLR.S.P.NP_IDLE.rx_RTP_drop"),
            (Some(P::NpIdle), _) => r.labels.push("FLR.S.P.NP_IDLE.rx_other"),
            (Some(P::NpTaken), Mt::Request) => {
                r.labels.push("FLR.S.P.NP_TAKEN.rx_REQUEST");
                self.taken_req(r, u, ev.n);
            }
            (Some(P::NpTaken), Mt::Release) => {
                r.labels.push("FLR.S.P.NP_TAKEN.rx_RELEASE");
                let next = if self.seq + 1 == SEQ_WRAP { 0 } else { self.seq + 1 };
                let p = self.p(u);
                p.tq = next;
                p.c21 = 1;
                self.dequeue(u);
                let h = self.holder();
                self.send(r, u, Mt::Taken, h, 0, 0);
            }
            (Some(P::NpTaken), Mt::Ack) if ev.n == 2 && ev.q == pp.tq => {
                r.labels.push("FLR.S.P.NP_TAKEN.rx_ACK_taken");
                self.p(u).c21 = 0;
            }
            (Some(P::NpTaken), Mt::Ack) if ev.n == 2 => r.labels.push("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale"),
            (Some(P::NpTaken), Mt::Ack) => r.labels.push("FLR.S.P.NP_TAKEN.rx_ACK_other"),
            (Some(P::NpTaken), Mt::QposReq) => {
                r.labels.push("FLR.S.P.NP_TAKEN.rx_QPOS_REQ");
                let (pos, qp) = (self.pos(u), self.qprio(u));
                self.send(r, u, Mt::Qinfo, None, pos, qp);
            }
            (Some(P::NpTaken), Mt::Rtp) => {
                r.labels.push("FLR.S.P.NP_TAKEN.rx_RTP");
                self.revoke3(r, u);
            }
            (Some(P::NpTaken), _) => r.labels.push("FLR.S.P.NP_TAKEN.rx_other"),
            (Some(P::Permitted), Mt::Release) => {
                r.labels.push("FLR.S.P.PERMITTED.rx_RELEASE");
                self.p(u).released = true;
                self.arb_rel(r, u);
            }
            (Some(P::Permitted), Mt::Request) => {
                r.labels.push("FLR.S.P.PERMITTED.rx_REQUEST");
                self.arb_req_permitted(r, u, t2_left);
            }
            (Some(P::Permitted), Mt::Rtp) => {
                r.labels.push("FLR.S.P.PERMITTED.rx_RTP");
                self.arb_rtp(r, u);
            }
            (Some(P::Permitted), _) => r.labels.push("FLR.S.P.PERMITTED.rx_other"),
            (Some(P::Prevoke), Mt::Release) => {
                r.labels.push("FLR.S.P.PREVOKE.rx_RELEASE");
                self.arb_rel(r, u);
            }
            (Some(P::Prevoke), Mt::Rtp) => {
                r.labels.push("FLR.S.P.PREVOKE.rx_RTP");
                self.arb_rtp(r, u);
            }
            (Some(P::Prevoke), _) => r.labels.push("FLR.S.P.PREVOKE.rx_other"),
            (_, Mt::Release) if pp.g_taken => {
                r.labels.push("FLR.S.P.NPSENDS.rx_RELEASE_taken");
                let next = if self.seq + 1 == SEQ_WRAP { 0 } else { self.seq + 1 };
                self.p(u).tq = next;
                let h = self.holder();
                self.send_e(r, u, P::NpTaken, Msg::new(Mt::Taken, h, 0, 0));
            }
            (_, Mt::Release) => {
                r.labels.push("FLR.S.P.NPSENDS.rx_RELEASE_idle");
                let prev = self.prev.clone();
                self.send_e(r, u, P::NpIdle, Msg::new(Mt::Idle, prev, 0, 0));
            }
            (_, Mt::Rtp) => r.labels.push("FLR.S.P.NPSENDS.rx_RTP"),
            _ => r.labels.push("FLR.S.P.NPSENDS.rx_other"),
        }
    }

    pub fn rtp(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        if !self.members.contains(u) {
            r.labels.push("FLR.S.gate.not_in_room_drop");
            return r;
        }
        self.part_c(&mut r, u, &Msg::new(Mt::Rtp, None, 0, 0), 0);
        r
    }

    pub fn t1(&mut self) -> Step {
        let mut r = Step::default();
        if self.g == G::Taken {
            r.labels.push("FLR.S.G.TAKEN.T1");
            self.prev = self.speaker.clone();
        } else {
            r.labels.push("FLR.S.G.PREVOKE.T1");
            self.prev = self.revoking.clone();
        }
        self.idle_enter(&mut r);
        r
    }

    pub fn t2(&mut self) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.G.TAKEN.T2");
        let u = self.speaker.clone().unwrap_or_default();
        self.p(&u).retry = true;
        self.prevoke_enter(&mut r, &u, revoke::TOO_LONG as u32);
        r
    }

    pub fn t3(&mut self) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.G.PREVOKE.T3");
        self.prev = self.revoking.clone();
        self.idle_enter(&mut r);
        r
    }

    pub fn t7(&mut self) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.G.IDLE.T7");
        self.c7 += 1;
        let prev = self.prev.clone();
        self.fan(&mut r, Msg::new(Mt::Idle, prev, 0, 0), None);
        r
    }

    pub fn t20(&mut self) -> Step {
        let mut r = Step::default();
        if self.c20 < timers::C20 as u32 {
            r.labels.push("FLR.S.G.TAKEN.T20");
            self.c20 += 1;
            let sp = self.speaker.clone().unwrap_or_default();
            let m = Msg { t: Mt::Granted, a: None, n: self.sprio, d: self.t2_full(), q: self.seq };
            self.part_a(&mut r, &sp, &m);
        } else {
            r.labels.push("FLR.S.G.TAKEN.T20_exhaust");
            self.c20 += 1;
        }
        r
    }

    pub fn t8(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        let st = self.part_of(u);
        let p = self.p(u);
        p.c8 += 1;
        let rc = p.rcause;
        if st == Some(P::Prevoke) {
            r.labels.push("FLR.S.P.PREVOKE.T8");
            self.send(&mut r, u, Mt::Revoke, None, rc, 0);
        } else {
            r.labels.push("FLR.S.P.NPSENDS.T8");
            self.send(&mut r, u, Mt::Revoke, None, revoke::NO_PERMISSION as u32, 0);
        }
        r
    }

    pub fn t21(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        let p = self.p(u);
        let c = p.c21;
        p.c21 += 1;
        let tq = p.tq;
        if c < timers::C21 as u32 {
            r.labels.push("FLR.S.P.NP_TAKEN.T21");
            let h = self.holder();
            self.send(&mut r, u, Mt::Taken, h, 0, tq);
        } else {
            r.labels.push("FLR.S.P.NP_TAKEN.T21_exhaust");
        }
        r
    }

    pub fn t9(&mut self, u: &str) -> Step {
        let mut r = Step::default();
        r.labels.push("FLR.S.T9.expire");
        self.p(u).retry = false;
        r
    }

    pub fn armed_server(&self) -> Vec<&'static str> {
        let mut on = Vec::new();
        if matches!(self.g, G::Taken | G::Prevoke) {
            on.push("T1");
        }
        if self.g == G::Taken && self.first_rtp {
            on.push("T2");
        }
        if self.g == G::Prevoke {
            on.push("T3");
        }
        if self.g == G::Idle && self.idle_bcast && self.c7 < timers::C7 as u32 {
            on.push("T7");
        }
        if self.g == G::Taken && self.succ && !self.rtp_seen && self.c20 <= timers::C20 as u32 {
            on.push("T20");
        }
        on
    }

    pub fn armed_participant(&self, u: &str) -> Vec<&'static str> {
        let mut on = Vec::new();
        let Some(p) = self.parts.get(u) else { return on };
        if matches!(p.part, Some(P::Prevoke) | Some(P::NpSends)) && p.c8 < timers::C8 as u32 {
            on.push("T8");
        }
        if p.retry {
            on.push("T9");
        }
        if p.part == Some(P::NpTaken) && p.c21 >= 1 && p.c21 <= timers::C21 as u32 {
            on.push("T21");
        }
        on
    }

    pub fn part_name(&self, u: &str) -> &'static str {
        self.part_of(u).map(|p| p.as_str()).unwrap_or("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_then_release() {
        let mut f = Floor::new(10, 30);
        f.ws_join("a", true, true);
        f.ws_join("b", true, true);
        f.ws_pub_select("a");
        let r = f.server_rx("a", &Msg::new(Mt::Request, None, 5, 0), 0);
        assert!(r.out.iter().any(|o| o.to == "a" && o.m.t == Mt::Granted));
        assert!(r.out.iter().any(|o| o.to == "b" && o.m.t == Mt::Taken));
        assert_eq!(r.fx, vec![Fx::Bind("a".into())]);
        let r = f.server_rx("a", &Msg::new(Mt::Release, None, 0, 0), 0);
        assert_eq!(f.g, G::Idle);
        assert_eq!(r.fx, vec![Fx::Release(Some("a".into()))]);
    }
}
