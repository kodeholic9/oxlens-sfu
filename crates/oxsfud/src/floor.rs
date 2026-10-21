// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 생성본 spec/protocol/out/floor/server/floor_server.ts(IR 1812dea47fd2095f) 이식 · 정:IMPL-FLR-LOCK 정:IMPL-FLR-RTP-OBSERVE 정:IMPL-FLR-POLICY 정:IMPL-FLR-FIRST-NOTICE 정:IMPL-FLR-MEDIA-BIND 정:RULE-FLR-SNAPSHOT 연:FLR.wire 연:RULE-FLR-TIMERS · model: claude-opus-5-5
//! 발언권 — 판정 로직(6.3.4) · 참가자 상태기(6.3.5)는 생성본을 걸음 그대로 옮긴다. 판정은 락 안, 송신 · 결합은 락 밖.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::mbcp::{self, deny as cause, revoke, tlv, timers as timer};
use oxsig::wire::Kind;

use crate::media::PubStream;
use crate::room::Room;
use crate::sfu::{Sfu, lock, mono_ms};

const SEQ_WRAP: u32 = 65_536;
const QPOS_NOT_IN_QUEUE: u32 = 254;
const PRIMING_FRAMES: u32 = 250;
const RELEASE_SILENCE: u8 = 3;
const PLI_BURST: [u64; 3] = [0, 500, 1500];

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
    Releasing,
    Npsends,
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

#[derive(Debug, Clone)]
pub struct Msg {
    pub t: T,
    pub a: Option<String>,
    pub n: u32,
    pub d: u32,
    pub q: u32,
}

impl Msg {
    fn new(t: T, a: Option<String>, n: u32, d: u32) -> Msg {
        Msg { t, a, n, d, q: 0 }
    }
}

#[derive(Debug, Clone)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Debug, Clone)]
pub enum Fx {
    Bind(String),
    Release(Option<String>),
}

#[derive(Debug, Clone)]
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
    pub pub_: HashMap<String, bool>,
    pub half: HashMap<String, bool>,
    pub perm: HashMap<String, bool>,
    pub retry: HashMap<String, bool>,
    pub part: HashMap<String, P>,
    pub released: HashMap<String, bool>,
    pub g_taken: HashMap<String, bool>,
    pub c8: HashMap<String, u32>,
    pub c21: HashMap<String, u32>,
    pub tq: HashMap<String, u32>,
    pub rcause: HashMap<String, u32>,
    pub seq: u32,
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
            pub_: HashMap::new(),
            half: HashMap::new(),
            perm: HashMap::new(),
            retry: HashMap::new(),
            part: HashMap::new(),
            released: HashMap::new(),
            g_taken: HashMap::new(),
            c8: HashMap::new(),
            c21: HashMap::new(),
            tq: HashMap::new(),
            rcause: HashMap::new(),
            seq: 0,
        }
    }
}

#[derive(Default)]
pub struct Step {
    pub o: Vec<(String, Msg)>,
    pub l: Vec<(&'static str, Option<String>)>,
    pub fx: Vec<Fx>,
}

pub struct Env {
    pub qmax: usize,
    pub t2_secs: u32,
    pub t2_remain: u32,
}

fn flag(m: &HashMap<String, bool>, u: &str) -> bool {
    m.get(u).copied().unwrap_or(false)
}

fn num(m: &HashMap<String, u32>, u: &str) -> u32 {
    m.get(u).copied().unwrap_or(0)
}

impl Srv {
    fn part(&self, u: &str) -> P {
        self.part.get(u).copied().unwrap_or(P::NpIdle)
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

    fn t2_left(&self, e: &Env) -> u32 {
        if self.first_rtp { e.t2_remain } else { e.t2_secs }
    }

    fn in_q(&self, u: &str) -> bool {
        self.queue.iter().any(|w| w.u == u)
    }

    fn pos(&self, u: &str) -> u32 {
        match self.queue.iter().position(|w| w.u == u) {
            Some(i) => i as u32 + 1,
            None => QPOS_NOT_IN_QUEUE,
        }
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

    fn send(&mut self, r: &mut Step, u: &str, t: T, a: Option<String>, n: u32, d: u32) {
        let seqd = matches!(t, T::Taken | T::Idle);
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
        r.o.push((u.to_string(), Msg { t, a, n, d: if seqd { 0 } else { d }, q }));
    }

    #[allow(clippy::too_many_arguments)]
    fn send_q(&mut self, r: &mut Step, u: &str, t: T, a: Option<String>, n: u32, d: u32, q: u32) {
        r.o.push((u.to_string(), Msg { t, a, n, d, q }));
    }

    #[allow(clippy::too_many_arguments)]
    fn send_e(&mut self, r: &mut Step, u: &str, p: P, t: T, a: Option<String>, n: u32, d: u32) {
        self.enter_r(r, u, p);
        self.send(r, u, t, a, n, d);
    }

    #[allow(clippy::too_many_arguments)]
    fn send_eq(&mut self, r: &mut Step, u: &str, p: P, t: T, a: Option<String>, n: u32, d: u32, q: u32) {
        self.enter_r(r, u, p);
        self.send_q(r, u, t, a, n, d, q);
    }

    fn fan(&mut self, r: &mut Step, e: &Env, ev: Msg, skip: Option<&str>) {
        let rest: Vec<String> = self.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
        let mut ev = ev;
        if matches!(ev.t, T::Taken | T::Idle) {
            self.seq = (self.seq + 1) % SEQ_WRAP;
            ev.q = self.seq;
        }
        for u in rest {
            self.part_a(r, e, &u, &ev);
        }
    }

    fn enter_taken(&mut self, r: &mut Step, e: &Env) {
        r.l.push(("FLR.S.G.TAKEN.enter", None));
        self.idle_bcast = false;
        let sp = self.speaker.clone().unwrap_or_default();
        let mut g = Msg::new(T::Granted, None, self.sprio, e.t2_secs);
        g.q = self.seq;
        self.part_a(r, e, &sp, &g);
        let taken = Msg::new(T::Taken, self.speaker.clone(), 0, 0);
        self.fan(r, e, taken, Some(&sp));
        r.fx.push(Fx::Bind(sp));
    }

    fn idle_enter(&mut self, r: &mut Step, e: &Env) {
        if self.queue.is_empty() {
            r.l.push(("FLR.S.G.IDLE.enter_empty", None));
            self.g = G::Idle;
            self.speaker = None;
            self.revoking = None;
            self.first_rtp = false;
            self.idle_bcast = true;
            self.c7 = 1;
            let ev = Msg::new(T::Idle, self.prev.clone(), 0, 0);
            self.fan(r, e, ev, None);
            r.fx.push(Fx::Release(self.prev.clone()));
            return;
        }
        r.l.push(("FLR.S.G.IDLE.enter_succession", None));
        let w = self.queue.remove(0);
        self.g = G::Taken;
        self.speaker = Some(w.u);
        self.sprio = w.p;
        self.revoking = None;
        self.succ = true;
        self.rtp_seen = false;
        self.first_rtp = false;
        self.c20 = 1;
        self.enter_taken(r, e);
    }

    fn prevoke_enter(&mut self, r: &mut Step, e: &Env, u: &str, c: u32) {
        r.l.push(("FLR.S.G.PREVOKE.enter", None));
        self.g = G::Prevoke;
        self.revoking = Some(u.to_string());
        self.rprio = self.sprio;
        self.rcause_g = c;
        self.speaker = None;
        self.part_a(r, e, u, &Msg::new(T::Revoke, None, c, 0));
    }

    fn arb_req_idle(&mut self, r: &mut Step, e: &Env, u: &str, p: u32) {
        if self.g != G::Idle {
            r.l.push(("FLR.S.G.any.req_discard", None));
            return;
        }
        if self.alone() {
            r.l.push(("FLR.S.G.IDLE.req_alone", None));
            self.part_a(r, e, u, &Msg::new(T::Deny, None, cause::ONLY_ONE as u32, 0));
            return;
        }
        if flag(&self.retry, u) {
            r.l.push(("FLR.S.G.IDLE.req_retry_after", None));
            self.part_a(r, e, u, &Msg::new(T::Deny, None, cause::RETRY_AFTER as u32, 0));
            return;
        }
        r.l.push(("FLR.S.G.IDLE.req_grant", None));
        self.g = G::Taken;
        self.speaker = Some(u.to_string());
        self.sprio = p;
        self.succ = false;
        self.rtp_seen = false;
        self.first_rtp = false;
        self.c20 = 0;
        self.enter_taken(r, e);
    }

    fn arb_req_permitted(&mut self, r: &mut Step, e: &Env, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.TAKEN.req_regrant", None));
            let mut g = Msg::new(T::Granted, None, self.sprio, self.t2_left(e));
            g.q = self.seq;
            self.part_a(r, e, u, &g);
            return;
        }
        r.l.push(("FLR.S.G.any.regrant_discard", None));
    }

    fn arb_preempt(&mut self, r: &mut Step, e: &Env, u: &str, p: u32) {
        if self.g == G::Taken {
            r.l.push(("FLR.S.G.TAKEN.preempt", None));
            self.queue.insert(0, Waiter { u: u.to_string(), p, pre: true });
            let sp = self.speaker.clone().unwrap_or_default();
            self.prevoke_enter(r, e, &sp, revoke::PREEMPTED as u32);
            self.part_a(r, e, u, &Msg::new(T::Qinfo, None, 1, p));
            return;
        }
        r.l.push(("FLR.S.G.PREVOKE.preempt_discard", None));
    }

    fn arb_rel(&mut self, r: &mut Step, e: &Env, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.TAKEN.rel_speaker", None));
            self.prev = Some(u.to_string());
            self.idle_enter(r, e);
            return;
        }
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.PREVOKE.rel_revoked", None));
            self.prev = Some(u.to_string());
            self.idle_enter(r, e);
            return;
        }
        r.l.push(("FLR.S.G.any.rel_discard", None));
    }

    fn arb_rtp(&mut self, r: &mut Step, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.TAKEN.rtp", None));
            self.rtp_seen = true;
            self.first_rtp = true;
            return;
        }
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.PREVOKE.rtp", None));
            return;
        }
        r.l.push(("FLR.S.G.any.rtp_discard", None));
    }

    pub fn server_rx(&mut self, r: &mut Step, e: &Env, u: &str, m: &Msg) {
        if m.t == T::Ack && self.part(u) != P::NpTaken {
            r.l.push(("FLR.S.gate.ack_drop", None));
            return;
        }
        let member = self.members.contains(u);
        if !member && m.t == T::Request {
            r.l.push(("FLR.S.gate.not_in_room", None));
            self.send(r, u, T::Deny, Some("not_in_room".to_string()), cause::OTHER_REASON as u32, 0);
            return;
        }
        if !member {
            r.l.push(("FLR.S.gate.not_in_room_drop", None));
            return;
        }
        if m.t == T::Request && !flag(&self.half, u) {
            r.l.push(("FLR.S.gate.recv_only", None));
            self.send(r, u, T::Deny, None, cause::RECV_ONLY as u32, 0);
            return;
        }
        if m.t == T::Request && !flag(&self.pub_, u) {
            r.l.push(("FLR.S.gate.not_pubroom", None));
            self.send(r, u, T::Deny, None, cause::NOT_PUBROOM as u32, 0);
            return;
        }
        if m.t == T::Request && !flag(&self.perm, u) {
            r.l.push(("FLR.S.gate.no_permission", None));
            self.send(r, u, T::Deny, None, cause::NO_PERMISSION as u32, 0);
            return;
        }
        self.part_c(r, e, u, m);
    }

    pub fn ready(&mut self, r: &mut Step, u: &str) {
        let h = self.holder();
        if h.as_deref() == Some(u) {
            r.l.push(("FLR.S.P.join.self", None));
            return;
        }
        if h.is_some() {
            r.l.push(("FLR.S.P.join.taken", None));
            self.send(r, u, T::Taken, h, 0, 0);
            return;
        }
        r.l.push(("FLR.S.P.join.idle", None));
        let prev = self.prev.clone();
        self.send(r, u, T::Idle, prev, 0, 0);
    }

    pub fn away(&mut self, r: &mut Step, e: &Env, u: &str) {
        if self.g == G::Taken && self.speaker.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.TAKEN.client_release", None));
            self.prev = Some(u.to_string());
            self.idle_enter(r, e);
            return;
        }
        if self.g == G::Prevoke && self.revoking.as_deref() == Some(u) {
            r.l.push(("FLR.S.G.PREVOKE.client_release", None));
            return;
        }
        if self.in_q(u) {
            r.l.push(("FLR.S.away.queued", None));
            self.dequeue(u);
            return;
        }
        r.l.push(("FLR.S.away.other", None));
    }

    pub fn ws_pub_select(&mut self, r: &mut Step, u: &str) {
        r.l.push(("FLR.S.ws.pub_select", None));
        self.pub_.insert(u.to_string(), true);
    }

    pub fn ws_reg_half(&mut self, r: &mut Step, u: &str) {
        r.l.push(("FLR.S.ws.reg_half", None));
        self.half.insert(u.to_string(), true);
    }

    pub fn ws_pub_away(&mut self, r: &mut Step, e: &Env, u: &str) {
        r.l.push(("FLR.S.ws.pub_away", None));
        self.pub_.insert(u.to_string(), false);
        self.away(r, e, u);
    }

    pub fn ws_leave(&mut self, r: &mut Step, e: &Env, u: &str) {
        r.l.push(("FLR.S.ws.leave", None));
        self.pub_.insert(u.to_string(), false);
        self.away(r, e, u);
        self.members.remove(u);
        self.retry.insert(u.to_string(), false);
        self.part.insert(u.to_string(), P::Releasing);
    }

    pub fn ws_join(&mut self, u: &str) {
        self.members.insert(u.to_string());
        self.part.insert(u.to_string(), P::NpIdle);
        self.released.insert(u.to_string(), false);
        self.g_taken.insert(u.to_string(), false);
        self.c8.insert(u.to_string(), 0);
        self.c21.insert(u.to_string(), 0);
    }

    fn part_a(&mut self, r: &mut Step, e: &Env, u: &str, ev: &Msg) {
        let p = self.part(u);
        match (ev.t, p) {
            (T::Qinfo, _) => {
                r.l.push(("FLR.S.P.any.a_QINFO", Some(u.to_string())));
                self.send(r, u, T::Qinfo, None, ev.n, ev.d);
            }
            (T::Deny, _) => {
                r.l.push(("FLR.S.P.any.a_DENY", Some(u.to_string())));
                self.send(r, u, T::Deny, None, ev.n, 0);
            }
            (T::Granted, P::NpIdle) => {
                r.l.push(("FLR.S.P.NP_IDLE.a_GRANTED", Some(u.to_string())));
                self.send_eq(r, u, P::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (T::Taken, P::NpIdle) => {
                r.l.push(("FLR.S.P.NP_IDLE.a_TAKEN", Some(u.to_string())));
                self.tq.insert(u.to_string(), ev.q);
                self.send_e(r, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q);
            }
            (T::Idle, P::NpIdle) => {
                r.l.push(("FLR.S.P.NP_IDLE.a_IDLE", Some(u.to_string())));
                self.send(r, u, T::Idle, ev.a.clone(), 0, ev.q);
            }
            (_, P::NpIdle) => r.l.push(("FLR.S.P.NP_IDLE.a_other", Some(u.to_string()))),
            (T::Granted, P::NpTaken) => {
                r.l.push(("FLR.S.P.NP_TAKEN.a_GRANTED", Some(u.to_string())));
                self.send_eq(r, u, P::Permitted, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (T::Taken, P::NpTaken) => {
                r.l.push(("FLR.S.P.NP_TAKEN.a_TAKEN", Some(u.to_string())));
                self.tq.insert(u.to_string(), ev.q);
                self.c21.insert(u.to_string(), 1);
                self.send(r, u, T::Taken, ev.a.clone(), 0, ev.q);
            }
            (T::Idle, P::NpTaken) => {
                r.l.push(("FLR.S.P.NP_TAKEN.a_IDLE", Some(u.to_string())));
                self.send_e(r, u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q);
            }
            (_, P::NpTaken) => r.l.push(("FLR.S.P.NP_TAKEN.a_other", Some(u.to_string()))),
            (T::Granted, P::Permitted) => {
                r.l.push(("FLR.S.P.PERMITTED.a_GRANTED", Some(u.to_string())));
                self.send_q(r, u, T::Granted, None, ev.n, ev.d, ev.q);
            }
            (T::Idle, P::Permitted) => {
                r.l.push(("FLR.S.P.PERMITTED.a_IDLE", Some(u.to_string())));
                self.enter_r(r, u, P::NpIdle);
            }
            (T::Taken, P::Permitted) => {
                r.l.push(("FLR.S.P.PERMITTED.a_TAKEN", Some(u.to_string())));
                self.tq.insert(u.to_string(), ev.q);
                self.send_e(r, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q);
            }
            (T::Revoke, P::Permitted) => {
                r.l.push(("FLR.S.P.PERMITTED.a_REVOKE", Some(u.to_string())));
                self.rcause.insert(u.to_string(), ev.n);
                self.send_e(r, u, P::Prevoke, T::Revoke, None, ev.n, 0);
            }
            (_, P::Permitted) => r.l.push(("FLR.S.P.PERMITTED.a_other", Some(u.to_string()))),
            (T::Idle, P::Prevoke) => {
                r.l.push(("FLR.S.P.PREVOKE.a_IDLE", Some(u.to_string())));
                self.send_e(r, u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q);
            }
            (T::Taken, P::Prevoke) => {
                r.l.push(("FLR.S.P.PREVOKE.a_TAKEN", Some(u.to_string())));
                self.tq.insert(u.to_string(), ev.q);
                self.send_e(r, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q);
            }
            (_, P::Prevoke) => r.l.push(("FLR.S.P.PREVOKE.a_other", Some(u.to_string()))),
            (_, P::Releasing) => r.l.push(("FLR.S.P.RELEASING.a_any", Some(u.to_string()))),
            (T::Idle, P::Npsends) => {
                r.l.push(("FLR.S.P.NPSENDS.a_IDLE", Some(u.to_string())));
                self.g_taken.insert(u.to_string(), false);
            }
            (T::Taken, P::Npsends) => {
                r.l.push(("FLR.S.P.NPSENDS.a_TAKEN", Some(u.to_string())));
                self.g_taken.insert(u.to_string(), true);
            }
            (_, P::Npsends) => r.l.push(("FLR.S.P.NPSENDS.a_other", Some(u.to_string()))),
        }
        let _ = e;
    }

    fn part_c(&mut self, r: &mut Step, e: &Env, u: &str, ev: &Msg) {
        let p = self.part(u);
        let us = Some(u.to_string());
        match (p, ev.t) {
            (P::NpIdle, T::Request) => {
                r.l.push(("FLR.S.P.NP_IDLE.rx_REQUEST", us));
                self.arb_req_idle(r, e, u, ev.n);
            }
            (P::NpIdle, T::Release) => {
                r.l.push(("FLR.S.P.NP_IDLE.rx_RELEASE", us));
                self.dequeue(u);
                let prev = self.prev.clone();
                self.send(r, u, T::Idle, prev, 0, 0);
            }
            (P::NpIdle, T::Rtp) if flag(&self.released, u) => {
                r.l.push(("FLR.S.P.NP_IDLE.rx_RTP_after_release", us));
                self.revoke3(r, u);
            }
            (P::NpIdle, T::Rtp) => r.l.push(("FLR.S.P.NP_IDLE.rx_RTP_drop", us)),
            (P::NpIdle, _) => r.l.push(("FLR.S.P.NP_IDLE.rx_other", us)),
            (P::NpTaken, T::Request) => {
                r.l.push(("FLR.S.P.NP_TAKEN.rx_REQUEST", us));
                self.taken_req(r, e, u, ev.n);
            }
            (P::NpTaken, T::Release) => {
                r.l.push(("FLR.S.P.NP_TAKEN.rx_RELEASE", us));
                let next = if self.seq + 1 == SEQ_WRAP { 0 } else { self.seq + 1 };
                self.tq.insert(u.to_string(), next);
                self.c21.insert(u.to_string(), 1);
                self.dequeue(u);
                let h = self.holder();
                self.send(r, u, T::Taken, h, 0, 0);
            }
            (P::NpTaken, T::Ack) if ev.n == mbcp::TAKEN as u32 && ev.q == num(&self.tq, u) => {
                r.l.push(("FLR.S.P.NP_TAKEN.rx_ACK_taken", us));
                self.c21.insert(u.to_string(), 0);
            }
            (P::NpTaken, T::Ack) if ev.n == mbcp::TAKEN as u32 => r.l.push(("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale", us)),
            (P::NpTaken, T::Ack) => r.l.push(("FLR.S.P.NP_TAKEN.rx_ACK_other", us)),
            (P::NpTaken, T::QposReq) => {
                r.l.push(("FLR.S.P.NP_TAKEN.rx_QPOS_REQ", us));
                let (pos, pr) = (self.pos(u), self.qprio(u));
                self.send(r, u, T::Qinfo, None, pos, pr);
            }
            (P::NpTaken, T::Rtp) => {
                r.l.push(("FLR.S.P.NP_TAKEN.rx_RTP", us));
                self.revoke3(r, u);
            }
            (P::NpTaken, _) => r.l.push(("FLR.S.P.NP_TAKEN.rx_other", us)),
            (P::Permitted, T::Release) => {
                r.l.push(("FLR.S.P.PERMITTED.rx_RELEASE", us));
                self.released.insert(u.to_string(), true);
                self.arb_rel(r, e, u);
            }
            (P::Permitted, T::Request) => {
                r.l.push(("FLR.S.P.PERMITTED.rx_REQUEST", us));
                self.arb_req_permitted(r, e, u);
            }
            (P::Permitted, T::Rtp) => {
                r.l.push(("FLR.S.P.PERMITTED.rx_RTP", us));
                self.arb_rtp(r, u);
            }
            (P::Permitted, _) => r.l.push(("FLR.S.P.PERMITTED.rx_other", us)),
            (P::Prevoke, T::Release) => {
                r.l.push(("FLR.S.P.PREVOKE.rx_RELEASE", us));
                self.arb_rel(r, e, u);
            }
            (P::Prevoke, T::Rtp) => {
                r.l.push(("FLR.S.P.PREVOKE.rx_RTP", us));
                self.arb_rtp(r, u);
            }
            (P::Prevoke, _) => r.l.push(("FLR.S.P.PREVOKE.rx_other", us)),
            (_, T::Release) if flag(&self.g_taken, u) => {
                r.l.push(("FLR.S.P.NPSENDS.rx_RELEASE_taken", us));
                let next = if self.seq + 1 == SEQ_WRAP { 0 } else { self.seq + 1 };
                self.tq.insert(u.to_string(), next);
                let h = self.holder();
                self.send_e(r, u, P::NpTaken, T::Taken, h, 0, 0);
            }
            (_, T::Release) => {
                r.l.push(("FLR.S.P.NPSENDS.rx_RELEASE_idle", us));
                let prev = self.prev.clone();
                self.send_e(r, u, P::NpIdle, T::Idle, prev, 0, 0);
            }
            (_, T::Rtp) => r.l.push(("FLR.S.P.NPSENDS.rx_RTP", us)),
            _ => r.l.push(("FLR.S.P.NPSENDS.rx_other", us)),
        }
    }

    fn enter_r(&mut self, r: &mut Step, u: &str, p: P) {
        let us = Some(u.to_string());
        let k = u.to_string();
        match p {
            P::NpIdle => {
                r.l.push(("FLR.S.P.NP_IDLE.enter", us));
                self.part.insert(k.clone(), p);
                self.g_taken.insert(k.clone(), false);
                self.c21.insert(k, 0);
            }
            P::NpTaken => {
                r.l.push(("FLR.S.P.NP_TAKEN.enter", us));
                self.part.insert(k.clone(), p);
                self.g_taken.insert(k.clone(), true);
                self.c21.insert(k, 1);
            }
            P::Permitted => {
                r.l.push(("FLR.S.P.PERMITTED.enter", us));
                self.part.insert(k.clone(), p);
                self.released.insert(k.clone(), false);
                self.c21.insert(k, 0);
            }
            P::Prevoke => {
                r.l.push(("FLR.S.P.PREVOKE.enter", us));
                self.part.insert(k.clone(), p);
                self.c8.insert(k.clone(), 1);
                self.c21.insert(k, 0);
            }
            _ => {
                r.l.push(("FLR.S.P.NPSENDS.enter", us));
                self.part.insert(k.clone(), p);
                self.c8.insert(k.clone(), 1);
                self.c21.insert(k, 0);
            }
        }
    }

    fn revoke3(&mut self, r: &mut Step, u: &str) {
        self.rcause.insert(u.to_string(), revoke::NO_PERMISSION as u32);
        self.send_e(r, u, P::Npsends, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
    }

    fn taken_req(&mut self, r: &mut Step, e: &Env, u: &str, p: u32) {
        if flag(&self.retry, u) {
            r.l.push(("FLR.S.P.NP_TAKEN.req_retry_after", None));
            self.send(r, u, T::Deny, None, cause::RETRY_AFTER as u32, 0);
            return;
        }
        if self.in_q(u) {
            r.l.push(("FLR.S.P.NP_TAKEN.req_renotify", None));
            let (pos, pr) = (self.pos(u), self.qprio(u));
            self.send(r, u, T::Qinfo, None, pos, pr);
            return;
        }
        if p > self.holder_prio() && !self.pre_q() {
            r.l.push(("FLR.S.P.NP_TAKEN.req_preempt", None));
            self.arb_preempt(r, e, u, p);
            return;
        }
        if p > self.holder_prio() {
            r.l.push(("FLR.S.P.NP_TAKEN.req_preempt_blocked", None));
            return;
        }
        if self.queue.len() >= e.qmax {
            r.l.push(("FLR.S.P.NP_TAKEN.req_full", None));
            self.send(r, u, T::Deny, None, cause::QUEUE_FULL as u32, 0);
            return;
        }
        r.l.push(("FLR.S.P.NP_TAKEN.req_enqueue", None));
        self.insert_q(Waiter { u: u.to_string(), p, pre: false });
        let pos = self.pos(u);
        self.send(r, u, T::Qinfo, None, pos, p);
    }

    fn srv_t1(&mut self, r: &mut Step, e: &Env) {
        if self.g == G::Taken {
            r.l.push(("FLR.S.G.TAKEN.T1", None));
            self.prev = self.speaker.clone();
        } else {
            r.l.push(("FLR.S.G.PREVOKE.T1", None));
            self.prev = self.revoking.clone();
        }
        self.idle_enter(r, e);
    }

    fn srv_t2(&mut self, r: &mut Step, e: &Env) {
        r.l.push(("FLR.S.G.TAKEN.T2", None));
        let u = self.speaker.clone().unwrap_or_default();
        self.retry.insert(u.clone(), true);
        self.prevoke_enter(r, e, &u, revoke::TOO_LONG as u32);
    }

    fn srv_t3(&mut self, r: &mut Step, e: &Env) {
        r.l.push(("FLR.S.G.PREVOKE.T3", None));
        self.prev = self.revoking.clone();
        self.idle_enter(r, e);
    }

    fn srv_t7(&mut self, r: &mut Step, e: &Env) {
        r.l.push(("FLR.S.G.IDLE.T7", None));
        self.c7 += 1;
        let ev = Msg::new(T::Idle, self.prev.clone(), 0, 0);
        self.fan(r, e, ev, None);
    }

    fn srv_t20(&mut self, r: &mut Step, e: &Env) {
        if self.c20 < timer::C20 as u32 {
            r.l.push(("FLR.S.G.TAKEN.T20", None));
            self.c20 += 1;
            let sp = self.speaker.clone().unwrap_or_default();
            let mut g = Msg::new(T::Granted, None, self.sprio, e.t2_secs);
            g.q = self.seq;
            self.part_a(r, e, &sp, &g);
            return;
        }
        r.l.push(("FLR.S.G.TAKEN.T20_exhaust", None));
        self.c20 += 1;
    }

    fn srv_t8(&mut self, r: &mut Step, u: &str) {
        let n = num(&self.c8, u) + 1;
        self.c8.insert(u.to_string(), n);
        if self.part(u) == P::Prevoke {
            r.l.push(("FLR.S.P.PREVOKE.T8", Some(u.to_string())));
            let c = num(&self.rcause, u);
            self.send(r, u, T::Revoke, None, c, 0);
            return;
        }
        r.l.push(("FLR.S.P.NPSENDS.T8", Some(u.to_string())));
        self.send(r, u, T::Revoke, None, revoke::NO_PERMISSION as u32, 0);
    }

    fn srv_t21(&mut self, r: &mut Step, u: &str) {
        let n = num(&self.c21, u);
        self.c21.insert(u.to_string(), n + 1);
        if n < timer::C21 as u32 {
            r.l.push(("FLR.S.P.NP_TAKEN.T21", Some(u.to_string())));
            let h = self.holder();
            let q = num(&self.tq, u);
            self.send(r, u, T::Taken, h, 0, q);
            return;
        }
        r.l.push(("FLR.S.P.NP_TAKEN.T21_exhaust", Some(u.to_string())));
    }

    fn srv_t9(&mut self, r: &mut Step, u: &str) {
        r.l.push(("FLR.S.T9.expire", Some(u.to_string())));
        self.retry.insert(u.to_string(), false);
    }

    fn armed_server(&self) -> Vec<Tm> {
        let mut on = Vec::new();
        if matches!(self.g, G::Taken | G::Prevoke) {
            on.push(Tm::T1);
        }
        if self.g == G::Taken && self.first_rtp {
            on.push(Tm::T2);
        }
        if self.g == G::Prevoke {
            on.push(Tm::T3);
        }
        if self.g == G::Idle && self.idle_bcast && self.c7 < timer::C7 as u32 {
            on.push(Tm::T7);
        }
        if self.g == G::Taken && self.succ && !self.rtp_seen && self.c20 <= timer::C20 as u32 {
            on.push(Tm::T20);
        }
        on
    }

    fn armed_participant(&self, u: &str) -> Vec<Tm> {
        let mut on = Vec::new();
        let p = self.part(u);
        if matches!(p, P::Prevoke | P::Npsends) && num(&self.c8, u) < timer::C8 as u32 {
            on.push(Tm::T8);
        }
        if flag(&self.retry, u) {
            on.push(Tm::T9);
        }
        let c = num(&self.c21, u);
        if p == P::NpTaken && c >= 1 && c <= timer::C21 as u32 {
            on.push(Tm::T21);
        }
        on
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tm {
    T1,
    T2,
    T3,
    T7,
    T8,
    T9,
    T20,
    T21,
}

fn starts(t: Tm) -> &'static [&'static str] {
    match t {
        Tm::T7 => &["FLR.S.G.IDLE.enter_empty"],
        Tm::T20 => &["FLR.S.G.IDLE.enter_succession"],
        Tm::T8 => &["FLR.S.P.PREVOKE.enter", "FLR.S.P.NPSENDS.enter"],
        Tm::T21 => &["FLR.S.P.NP_TAKEN.enter", "FLR.S.P.NP_TAKEN.a_TAKEN", "FLR.S.P.NP_TAKEN.rx_RELEASE"],
        Tm::T1 => &["FLR.S.G.TAKEN.enter"],
        Tm::T3 => &["FLR.S.G.PREVOKE.enter"],
        _ => &[],
    }
}

fn dur(t: Tm, t2_ms: u64) -> u64 {
    match t {
        Tm::T1 => timer::T1_MS,
        Tm::T2 => t2_ms,
        Tm::T3 => timer::T3_MS,
        Tm::T7 => timer::T7_MS,
        Tm::T8 => timer::T8_MS,
        Tm::T9 => timer::T9_MS,
        Tm::T20 => timer::T20_MS,
        Tm::T21 => timer::T21_MS,
    }
}

pub struct FloorRoom {
    pub srv: Srv,
    pub qmax: usize,
    pub first_rtp_at: u64,
    pub last_rtp_at: u64,
    pub taken_at: u64,
    pub deadlines: HashMap<(Tm, Option<String>), u64>,
}

impl FloorRoom {
    pub fn new(qmax: usize) -> FloorRoom {
        FloorRoom { srv: Srv::default(), qmax, first_rtp_at: 0, last_rtp_at: 0, taken_at: 0, deadlines: HashMap::new() }
    }

    fn env(&self, t2_secs: u32, now: u64) -> Env {
        let t2_ms = t2_secs as u64 * 1000;
        let remain = (self.first_rtp_at + t2_ms).saturating_sub(now) / 1000;
        Env { qmax: self.qmax, t2_secs, t2_remain: remain as u32 }
    }

    fn after(&mut self, step: &Step, was_first: bool, t2_ms: u64, now: u64) {
        if !was_first && self.srv.first_rtp {
            self.first_rtp_at = now;
        }
        for (l, _) in &step.l {
            if *l == "FLR.S.G.TAKEN.enter" {
                self.taken_at = now;
                self.last_rtp_at = 0;
            }
        }
        let mut armed: Vec<(Tm, Option<String>)> = self.srv.armed_server().into_iter().map(|t| (t, None)).collect();
        let users: Vec<String> = self.srv.part.keys().cloned().collect();
        for u in users {
            for t in self.srv.armed_participant(&u) {
                armed.push((t, Some(u.clone())));
            }
        }
        self.deadlines.retain(|k, _| armed.contains(k));
        for k in armed {
            let restarted = step.l.iter().any(|(l, lu)| starts(k.0).contains(l) && (k.1.is_none() || lu.as_ref() == k.1.as_ref()));
            if restarted || !self.deadlines.contains_key(&k) {
                let at = match k.0 {
                    Tm::T2 => self.first_rtp_at + t2_ms,
                    _ => now + dur(k.0, t2_ms),
                };
                self.deadlines.insert(k, at);
            }
        }
    }

    fn due(&self, now: u64) -> Option<(Tm, Option<String>)> {
        let mut best: Option<((Tm, Option<String>), u64)> = None;
        for (k, at) in &self.deadlines {
            let at = if k.0 == Tm::T1 { (*at).max(self.last_rtp_at + timer::T1_MS) } else { *at };
            if at <= now && best.as_ref().is_none_or(|(_, b)| at < *b) {
                best = Some((k.clone(), at));
            }
        }
        best.map(|(k, _)| k)
    }
}

fn run(sfu: &Arc<Sfu>, room: &Arc<Room>, f: impl FnOnce(&mut Srv, &mut Step, &Env)) {
    let now = mono_ms();
    let t2 = sfu.policy.floor.t2_stop_talking_secs as u32;
    let step = {
        let mut fr = lock(&room.floor);
        let env = fr.env(t2, now);
        let was = fr.srv.first_rtp;
        let mut step = Step::default();
        f(&mut fr.srv, &mut step, &env);
        fr.after(&step, was, t2 as u64 * 1000, now);
        room.holder.store(fr.srv.holder().map(Arc::new));
        step
    };
    execute(sfu, room, step);
}

fn execute(sfu: &Arc<Sfu>, room: &Arc<Room>, step: Step) {
    for (to, m) in &step.o {
        send_dc(sfu, room, to, m);
    }
    for fx in step.fx {
        match fx {
            Fx::Bind(u) => bind(sfu, room, &u),
            Fx::Release(prev) => release(sfu, room, prev),
        }
    }
    if room.holder.load().is_none() {
        for s in &room.slots {
            s.bound.store(None);
        }
    }
}

fn half_stream(sfu: &Sfu, user: &str, kind: Kind) -> Option<Arc<PubStream>> {
    sfu.peer_of_user(user)
        .into_iter()
        .flat_map(|p| lock(&p.streams).clone())
        .find(|s| s.kind == kind && s.is_half() && !s.gone.load(Ordering::Relaxed))
}

fn bind(sfu: &Arc<Sfu>, room: &Arc<Room>, u: &str) {
    let audio = half_stream(sfu, u, Kind::Audio);
    room.slot(Kind::Audio).bound.store(audio);
    let vcodec = room.codecs.video.clone().unwrap_or_else(|| "H264".into());
    let video = half_stream(sfu, u, Kind::Video).filter(|s| {
        let ok = crate::rtp::codec_matches(&vcodec, crate::rtp::sub_fmtp(&vcodec), &s.codec, s.fmtp.as_deref());
        if !ok {
            tracing::warn!(kind = "diag", room = room.id.as_str(), user = u, codec = s.codec.as_str(), "slot video codec mismatch — unbound");
        }
        ok
    });
    let has_video = video.is_some();
    room.slot(Kind::Video).bound.store(video);
    let a = room.slot(Kind::Audio);
    a.priming_left.store(PRIMING_FRAMES, Ordering::Relaxed);
    a.priming_stop.store(false, Ordering::Relaxed);
    a.silence_left.store(0, Ordering::Relaxed);
    let now = mono_ms();
    if has_video {
        let mut b = lock(&room.slot(Kind::Video).pli_burst);
        b.clear();
        b.extend(PLI_BURST.iter().map(|d| now + d));
    }
    tracing::info!(kind = "ev", spec = "IMPL-FLR-MEDIA-BIND", room = room.id.as_str(), user = u, "granted bind");
}

fn release(_sfu: &Arc<Sfu>, room: &Arc<Room>, prev: Option<String>) {
    for s in &room.slots {
        s.bound.store(None);
    }
    let a = room.slot(Kind::Audio);
    a.priming_stop.store(true, Ordering::Relaxed);
    a.priming_left.store(0, Ordering::Relaxed);
    a.silence_left.store(RELEASE_SILENCE, Ordering::Relaxed);
    room.prev_speaker.store(prev.map(Arc::new));
    lock(&room.slot(Kind::Video).pli_burst).clear();
    tracing::info!(kind = "ev", spec = "IMPL-FLR-MEDIA-BIND", room = room.id.as_str(), "released");
}

fn wire(room: &str, m: &Msg) -> mbcp::Msg {
    let t = match m.t {
        T::Request => mbcp::REQUEST,
        T::Granted => mbcp::GRANTED,
        T::Taken => mbcp::TAKEN,
        T::Deny => mbcp::DENY,
        T::Release => mbcp::RELEASE,
        T::Idle => mbcp::IDLE,
        T::Revoke => mbcp::REVOKE,
        T::QposReq => mbcp::QPOS_REQ,
        T::Qinfo => mbcp::QINFO,
        T::Ack | T::Rtp => mbcp::ACK,
    };
    let mut w = mbcp::Msg::new(t);
    w.ack = matches!(m.t, T::Granted | T::Taken | T::Deny);
    let u16be = |v: u32| (v as u16).to_be_bytes().to_vec();
    match m.t {
        T::Granted => {
            w.push(tlv::PRIORITY, vec![m.n as u8, 0]);
            w.push(tlv::DURATION, u16be(m.d));
            w.push(tlv::SEQ, u16be(m.q));
        }
        T::Taken => {
            if let Some(a) = &m.a {
                w.push(tlv::SPEAKER, a.as_bytes().to_vec());
            }
            w.push(tlv::SEQ, u16be(m.q));
        }
        T::Deny | T::Revoke => {
            w.push(tlv::CAUSE, vec![m.n as u8]);
            if let Some(a) = &m.a {
                w.push(tlv::REASON, a.as_bytes().to_vec());
            }
        }
        T::Idle => {
            if let Some(a) = &m.a {
                w.push(tlv::PREV, a.as_bytes().to_vec());
            }
            w.push(tlv::SEQ, u16be(m.q));
        }
        T::Qinfo => w.push(tlv::QINFO, vec![m.n as u8, m.d as u8]),
        _ => {}
    }
    w.push(tlv::ROOM, room.as_bytes().to_vec());
    w
}

fn send_dc(sfu: &Arc<Sfu>, room: &Room, to: &str, m: &Msg) {
    let Some(bytes) = mbcp::encode(&wire(&room.id, m)) else { return };
    let Some(frame) = oxsig::dc::encode(oxsig::dc::SVC_FLOOR, &bytes) else { return };
    let peer = sfu.peer_of_user(to).into_iter().find(|p| p.in_room(&room.id)).or_else(|| sfu.peer_of_user(to).into_iter().next());
    match peer {
        Some(p) => crate::secure::dc_send(&p.pub_conn, frame),
        None => sfu.counters.drop("dc_no_peer"),
    }
}

pub fn on_dc(sfu: &Arc<Sfu>, user: &str, payload: &[u8]) {
    let Some(f) = oxsig::dc::decode(payload) else {
        sfu.counters.drop("dc_frame");
        return;
    };
    if f.svc != oxsig::dc::SVC_FLOOR {
        sfu.counters.drop("dc_svc");
        return;
    }
    let Some(w) = mbcp::decode(f.payload) else {
        sfu.counters.drop("mbcp_decode");
        return;
    };
    let Some(room_id) = w.tlv(tlv::ROOM).map(|v| String::from_utf8_lossy(v).to_string()) else {
        sfu.counters.drop("mbcp_no_room");
        return;
    };
    let Some(room) = sfu.room(&room_id) else {
        sfu.counters.drop("mbcp_unknown_room");
        return;
    };
    let t = match w.msg_type {
        mbcp::REQUEST => T::Request,
        mbcp::RELEASE => T::Release,
        mbcp::QPOS_REQ => T::QposReq,
        mbcp::ACK => T::Ack,
        _ => {
            sfu.counters.drop("mbcp_s2c_type");
            return;
        }
    };
    let n = match t {
        T::Request => w.tlv(tlv::PRIORITY).and_then(|v| v.first().copied()).unwrap_or(0) as u32,
        T::Ack => w.tlv(tlv::ACKTYPE).and_then(|v| v.first().copied()).unwrap_or(0) as u32,
        _ => 0,
    };
    let q = w.tlv(tlv::SEQ).filter(|v| v.len() >= 2).map(|v| u16::from_be_bytes([v[0], v[1]]) as u32).unwrap_or(0);
    let m = Msg { t, a: None, n, d: 0, q };
    refresh(sfu, &room, user);
    run(sfu, &room, |s, r, e| s.server_rx(r, e, user, &m));
}

fn refresh(sfu: &Sfu, room: &Room, user: &str) {
    let peers = sfu.peer_of_user(user);
    let half = peers.iter().any(|p| lock(&p.streams).iter().any(|s| s.is_half() && !s.gone.load(Ordering::Relaxed)));
    let pubr = peers.iter().any(|p| p.pub_room_id().as_deref() == Some(room.id.as_str()));
    let perm = room.lock().members.get(user).map(|m| m.permission.floor_request).unwrap_or(false);
    let mut fr = lock(&room.floor);
    fr.srv.half.insert(user.to_string(), half);
    fr.srv.pub_.insert(user.to_string(), pubr);
    fr.srv.perm.insert(user.to_string(), perm);
}

pub fn observe_rtp(sfu: &Arc<Sfu>, room: &Arc<Room>, stream: &PubStream, now: u64) {
    let last = stream.notify_at.load(Ordering::Relaxed);
    {
        let mut fr = lock(&room.floor);
        fr.last_rtp_at = now;
    }
    if last != 0 && now.saturating_sub(last) < 500 {
        return;
    }
    stream.notify_at.store(now, Ordering::Relaxed);
    let m = Msg::new(T::Rtp, None, 0, 0);
    let u = stream.user_id.clone();
    run(sfu, room, |s, r, e| s.part_c(r, e, &u, &m));
}

pub fn join(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    {
        let mut fr = lock(&room.floor);
        fr.srv.ws_join(user);
    }
    refresh(sfu, room, user);
}

pub fn leave(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, e| s.ws_leave(r, e, user));
}

pub fn pub_select(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, _| s.ws_pub_select(r, user));
}

pub fn pub_away(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, e| s.ws_pub_away(r, e, user));
}

pub fn reg_half(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, _| s.ws_reg_half(r, user));
}

pub fn release_track(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, e| s.away(r, e, user));
}

pub fn first_notice(sfu: &Arc<Sfu>, room: &Arc<Room>, user: &str) {
    run(sfu, room, |s, r, _| s.ready(r, user));
}

pub fn is_holder(room: &Room, user: &str) -> bool {
    lock(&room.floor).srv.holder().as_deref() == Some(user)
}

pub fn tick(sfu: &Arc<Sfu>, room: &Arc<Room>) {
    for _ in 0..16 {
        let now = mono_ms();
        let due = lock(&room.floor).due(now);
        let Some((t, u)) = due else { return };
        let uu = u.clone().unwrap_or_default();
        run(sfu, room, |s, r, e| match t {
            Tm::T1 => s.srv_t1(r, e),
            Tm::T2 => s.srv_t2(r, e),
            Tm::T3 => s.srv_t3(r, e),
            Tm::T7 => s.srv_t7(r, e),
            Tm::T20 => s.srv_t20(r, e),
            Tm::T8 => s.srv_t8(r, &uu),
            Tm::T9 => s.srv_t9(r, &uu),
            Tm::T21 => s.srv_t21(r, &uu),
        });
        let mut fr = lock(&room.floor);
        let key = (t, u);
        if fr.deadlines.contains_key(&key) && matches!(t, Tm::T7 | Tm::T8 | Tm::T20 | Tm::T21) {
            let t2 = sfu.policy.floor.t2_stop_talking_secs as u64 * 1000;
            fr.deadlines.insert(key, now + dur(t, t2));
        } else if fr.deadlines.get(&key).is_some_and(|at| *at <= now) && matches!(t, Tm::T9) {
            fr.deadlines.remove(&key);
        }
    }
}

pub fn snapshot(room: &Room) -> serde_json::Value {
    let fr = lock(&room.floor);
    let s = &fr.srv;
    let started = if s.holder().is_some() { Some(crate::sfu::now_ms().saturating_sub(mono_ms().saturating_sub(fr.taken_at))) } else { None };
    serde_json::json!({
        "speaker": s.holder(),
        "queue": s.queue.iter().map(|w| serde_json::json!({ "user_id": w.u, "priority": w.p })).collect::<Vec<_>>(),
        "started_at": started,
    })
}
