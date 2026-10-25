// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-FLR-SERVER-SCOPE 정:IMPL-FLR-LOCK 정:RULE-CONC-SIX 연:FLR.S 연:RULE-FLR-SERVER-EVENTS 연:RULE-FLR-JOIN-NOTIFY 연:RULE-FLR-PREEMPT 연:RULE-FLR-PERM-REVOKE-ENFORCE 연:RULE-FLR-SEQ 연:DEV-FLR-TAKEN-ACK 연:DEV-FLR-T21-SAME-SEQ 연:DEV-FLR-GRANTED-SEQ 연:DEV-FLR-FIRST-NOTIFY 연:DEV-FLR-PREEMPT-RELATIVE · model: claude-opus-5-5
//! 발언권 서버 판정 — 연동규격서 원천이 지은 `out/floor/server/floor_server.ts`(3GPP TS 24.380 §6.3.4 판정 로직 · §6.3.5 참가자 상태기)를 옮겼다. 한 걸음 = (새 상태 · 보낼 것 · 지난 라벨 · 효과). 시계 · I/O 없음.

use std::collections::{BTreeSet, HashMap};

use oxsig::mbcp::{deny, qpos, revoke, timers};

pub const SEQ_WRAP: u32 = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum G {
    Idle,
    Taken,
    Prevoke,
}

impl G {
    pub fn name(self) -> &'static str {
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
    pub fn name(self) -> &'static str {
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
pub struct M {
    pub t: T,
    pub a: Option<String>,
    pub n: u32,
    pub d: u32,
    pub q: u32,
}

impl M {
    pub fn new(t: T, a: Option<String>, n: u32, d: u32) -> M {
        M { t, a, n, d, q: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub to: String,
    pub m: M,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiter {
    pub u: String,
    pub p: u32,
    pub pre: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn b(m: &HashMap<String, bool>, u: &str) -> bool {
    m.get(u).copied().unwrap_or(false)
}

fn n(m: &HashMap<String, u32>, u: &str) -> u32 {
    m.get(u).copied().unwrap_or(0)
}

impl Srv {
    pub fn part(&self, u: &str) -> P {
        self.part.get(u).copied().unwrap_or(P::NpIdle)
    }

    pub fn perm_of(&self, u: &str) -> bool {
        self.perm.get(u).copied().unwrap_or(true)
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

    pub fn pos(&self, u: &str) -> u32 {
        match self.queue.iter().position(|w| w.u == u) {
            Some(i) => i as u32 + 1,
            None => u32::from(qpos::NOT_IN_QUEUE),
        }
    }

    fn q_prio(&self, u: &str) -> u32 {
        self.queue.iter().find(|w| w.u == u).map(|w| w.p).unwrap_or(0)
    }

    fn dequeue(mut self, u: &str) -> Srv {
        self.queue.retain(|w| w.u != u);
        self
    }

    fn pre_q(&self) -> bool {
        self.queue.iter().any(|w| w.pre)
    }
}

fn queue_before(w: &Waiter, x: &Waiter) -> bool {
    (w.pre && !x.pre) || (w.pre == x.pre && w.p > x.p)
}

fn insert_q(q: &[Waiter], w: Waiter) -> Vec<Waiter> {
    let mut v = q.to_vec();
    match v.iter().position(|x| queue_before(&w, x)) {
        Some(i) => v.insert(i, w),
        None => v.push(w),
    }
    v
}

pub struct Cfg {
    pub qmax: u32,
    pub t2_remain_secs: u32,
}

pub const T2_SECS: u32 = (timers::T2_MS / 1000) as u32;

pub type Lab = (&'static str, Vec<(&'static str, String)>);

#[derive(Debug, Clone)]
pub struct R {
    pub s: Srv,
    pub o: Vec<Out>,
    pub l: Vec<Lab>,
    pub fx: Vec<Fx>,
}

pub fn r0(s: Srv) -> R {
    R { s, o: Vec::new(), l: Vec::new(), fx: Vec::new() }
}

fn tl(name: &'static str, args: Vec<(&'static str, String)>, mut r: R) -> R {
    r.l.insert(0, (name, args));
    r
}

fn fxr(e: Fx, mut r: R) -> R {
    r.fx.push(e);
    r
}

fn then(r: R, f: impl FnOnce(Srv) -> R) -> R {
    let r2 = f(r.s);
    let mut o = r.o;
    o.extend(r2.o);
    let mut l = r.l;
    l.extend(r2.l);
    let mut fx = r.fx;
    fx.extend(r2.fx);
    R { s: r2.s, o, l, fx }
}

fn send(mut s: Srv, u: &str, t: T, a: Option<String>, nn: u32, d: u32) -> R {
    let seqd = matches!(t, T::Taken | T::Idle);
    let bump = seqd && d == 0;
    if bump {
        s.seq = (s.seq + 1) % SEQ_WRAP;
    }
    let q = if bump {
        s.seq
    } else if seqd {
        d
    } else {
        0
    };
    let m = M { t, a, n: nn, d: if seqd { 0 } else { d }, q };
    R { o: vec![Out { to: u.to_string(), m }], ..r0(s) }
}

fn send_e(s: Srv, u: &str, p: P, t: T, a: Option<String>, nn: u32, d: u32) -> R {
    let u2 = u.to_string();
    then(enter_r(s, u, p), move |x| send(x, &u2, t, a, nn, d))
}

fn send_q(s: Srv, u: &str, t: T, a: Option<String>, nn: u32, d: u32, q: u32) -> R {
    R { o: vec![Out { to: u.to_string(), m: M { t, a, n: nn, d, q } }], ..r0(s) }
}

struct Ev {
    t: T,
    a: Option<String>,
    n: u32,
    d: u32,
    q: u32,
}

fn send_eq(s: Srv, u: &str, p: P, ev: Ev) -> R {
    let u2 = u.to_string();
    then(enter_r(s, u, p), move |x| send_q(x, &u2, ev.t, ev.a, ev.n, ev.d, ev.q))
}

fn fan_set(s: Srv, ev: &M, rest: Vec<String>, c: &Cfg) -> R {
    let mut r = r0(s);
    let mut rest = rest;
    rest.sort();
    for u in rest {
        let e = ev.clone();
        r = then(r, |x| part_a(x, &u, &e, c));
    }
    r
}

fn fan(mut s: Srv, ev: M, skip: Option<&str>, c: &Cfg) -> R {
    let rest: Vec<String> = s.members.iter().filter(|x| Some(x.as_str()) != skip).cloned().collect();
    if matches!(ev.t, T::Taken | T::Idle) {
        s.seq = (s.seq + 1) % SEQ_WRAP;
        let e = M { q: s.seq, ..ev };
        return fan_set(s, &e, rest, c);
    }
    fan_set(s, &ev, rest, c)
}

fn granted(s: &Srv, d: u32) -> M {
    M { t: T::Granted, a: None, n: s.sprio, d, q: s.seq }
}

pub fn enter_taken(s: Srv, c: &Cfg) -> R {
    let mut x1 = s;
    x1.idle_bcast = false;
    let speaker = x1.speaker.clone().unwrap_or_default();
    let g = granted(&x1, T2_SECS);
    let r = then(part_a(x1, &speaker, &g, c), |y2| {
        let sp = y2.speaker.clone();
        then(fan(y2, M::new(T::Taken, sp.clone(), 0, 0), sp.as_deref(), c), |y3| {
            let who = y3.speaker.clone().unwrap_or_default();
            fxr(Fx::Bind(who), r0(y3))
        })
    });
    tl("FLR.S.G.TAKEN.enter", Vec::new(), r)
}

pub fn idle_enter(s: Srv, c: &Cfg) -> R {
    if s.queue.is_empty() {
        let mut x4 = s;
        x4.g = G::Idle;
        x4.speaker = None;
        x4.revoking = None;
        x4.first_rtp = false;
        x4.idle_bcast = true;
        x4.c7 = 1;
        let prev = x4.prev.clone();
        let r = then(fan(x4, M::new(T::Idle, prev, 0, 0), None, c), |y5| {
            let p = y5.prev.clone();
            fxr(Fx::Release(p), r0(y5))
        });
        return tl("FLR.S.G.IDLE.enter_empty", Vec::new(), r);
    }
    let mut x6 = s;
    let w = x6.queue.remove(0);
    x6.g = G::Taken;
    x6.speaker = Some(w.u);
    x6.sprio = w.p;
    x6.revoking = None;
    x6.succ = true;
    x6.rtp_seen = false;
    x6.first_rtp = false;
    x6.c20 = 1;
    tl("FLR.S.G.IDLE.enter_succession", Vec::new(), enter_taken(x6, c))
}

pub fn prevoke_enter(s: Srv, u: &str, cause: u32, c: &Cfg) -> R {
    let mut x7 = s;
    x7.g = G::Prevoke;
    x7.revoking = Some(u.to_string());
    x7.rprio = x7.sprio;
    x7.rcause_g = cause;
    x7.speaker = None;
    tl("FLR.S.G.PREVOKE.enter", vec![("u", u.to_string()), ("cause", format!("{cause:?}"))], part_a(x7, u, &M::new(T::Revoke, None, cause, 0), c))
}

pub fn arb_req_idle(s: Srv, u: &str, p: u32, c: &Cfg) -> R {
    if s.g != G::Idle {
        return tl("FLR.S.G.any.req_discard", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(s));
    }
    if s.alone() {
        return tl(
            "FLR.S.G.IDLE.req_alone",
            vec![("u", u.to_string()), ("p", format!("{p:?}"))],
            part_a(s, u, &M::new(T::Deny, None, u32::from(deny::ONLY_ONE), 0), c),
        );
    }
    if b(&s.retry, u) {
        return tl(
            "FLR.S.G.IDLE.req_retry_after",
            vec![("u", u.to_string()), ("p", format!("{p:?}"))],
            part_a(s, u, &M::new(T::Deny, None, u32::from(deny::RETRY_AFTER), 0), c),
        );
    }
    let mut x8 = s;
    x8.g = G::Taken;
    x8.speaker = Some(u.to_string());
    x8.sprio = p;
    x8.succ = false;
    x8.rtp_seen = false;
    x8.first_rtp = false;
    x8.c20 = 0;
    tl("FLR.S.G.IDLE.req_grant", vec![("u", u.to_string()), ("p", format!("{p:?}"))], enter_taken(x8, c))
}

fn t2_left(s: &Srv, c: &Cfg) -> u32 {
    if s.first_rtp { c.t2_remain_secs } else { T2_SECS }
}

pub fn arb_req_permitted(s: Srv, u: &str, c: &Cfg) -> R {
    if s.g == G::Taken && s.speaker.as_deref() == Some(u) {
        let g = granted(&s, t2_left(&s, c));
        return tl("FLR.S.G.TAKEN.req_regrant", vec![("u", u.to_string())], part_a(s, u, &g, c));
    }
    tl("FLR.S.G.any.regrant_discard", vec![("u", u.to_string())], r0(s))
}

pub fn arb_preempt(s: Srv, u: &str, p: u32, c: &Cfg) -> R {
    if s.g == G::Taken {
        let mut x9 = s;
        x9.queue.insert(0, Waiter { u: u.to_string(), p, pre: true });
        let sp = x9.speaker.clone().unwrap_or_default();
        let u2 = u.to_string();
        let r = then(prevoke_enter(x9, &sp, u32::from(revoke::PREEMPTED), c), move |y10| part_a(y10, &u2, &M::new(T::Qinfo, None, 1, p), c));
        return tl("FLR.S.G.TAKEN.preempt", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r);
    }
    tl("FLR.S.G.PREVOKE.preempt_discard", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(s))
}

pub fn arb_rel(s: Srv, u: &str, c: &Cfg) -> R {
    if s.g == G::Taken && s.speaker.as_deref() == Some(u) {
        let mut x11 = s;
        x11.prev = Some(u.to_string());
        return tl("FLR.S.G.TAKEN.rel_speaker", vec![("u", u.to_string())], idle_enter(x11, c));
    }
    if s.g == G::Prevoke && s.revoking.as_deref() == Some(u) {
        let mut x12 = s;
        x12.prev = Some(u.to_string());
        return tl("FLR.S.G.PREVOKE.rel_revoked", vec![("u", u.to_string())], idle_enter(x12, c));
    }
    tl("FLR.S.G.any.rel_discard", vec![("u", u.to_string())], r0(s))
}

pub fn arb_rtp(s: Srv, u: &str) -> R {
    if s.g == G::Taken && s.speaker.as_deref() == Some(u) {
        let mut x13 = s;
        x13.rtp_seen = true;
        x13.first_rtp = true;
        return tl("FLR.S.G.TAKEN.rtp", vec![("u", u.to_string())], r0(x13));
    }
    if s.g == G::Prevoke && s.revoking.as_deref() == Some(u) {
        return tl("FLR.S.G.PREVOKE.rtp", vec![("u", u.to_string())], r0(s));
    }
    tl("FLR.S.G.any.rtp_discard", vec![("u", u.to_string())], r0(s))
}

pub fn server_rx(s: Srv, u: &str, m: &M, c: &Cfg) -> R {
    if m.t == T::Ack && s.part(u) != P::NpTaken {
        return tl("FLR.S.gate.ack_drop", vec![("u", u.to_string())], r0(s));
    }
    if !s.members.contains(u) && m.t == T::Request {
        return tl("FLR.S.gate.not_in_room", vec![("u", u.to_string())], send(s, u, T::Deny, None, u32::from(deny::OTHER_REASON), 0));
    }
    if !s.members.contains(u) {
        return tl("FLR.S.gate.not_in_room_drop", vec![("u", u.to_string())], r0(s));
    }
    if m.t == T::Request && !b(&s.half, u) {
        return tl("FLR.S.gate.recv_only", vec![("u", u.to_string())], send(s, u, T::Deny, None, u32::from(deny::RECV_ONLY), 0));
    }
    if m.t == T::Request && !b(&s.pub_, u) {
        return tl("FLR.S.gate.not_pubroom", vec![("u", u.to_string())], send(s, u, T::Deny, None, u32::from(deny::NOT_PUBROOM), 0));
    }
    if m.t == T::Request && !s.perm_of(u) {
        return tl("FLR.S.gate.no_permission", vec![("u", u.to_string())], send(s, u, T::Deny, None, u32::from(deny::NO_PERMISSION), 0));
    }
    part_c(s, u, m, c)
}

pub fn ready(s: Srv, u: &str) -> R {
    let h = s.holder();
    if h.as_deref() == Some(u) {
        return tl("FLR.S.P.join.self", vec![("u", u.to_string())], r0(s));
    }
    if h.is_some() {
        return tl("FLR.S.P.join.taken", vec![("u", u.to_string())], send(s, u, T::Taken, h, 0, 0));
    }
    let p = s.prev.clone();
    tl("FLR.S.P.join.idle", vec![("u", u.to_string())], send(s, u, T::Idle, p, 0, 0))
}

pub fn perm_down(s: Srv, u: &str, c: &Cfg) -> R {
    if s.g == G::Taken && s.speaker.as_deref() == Some(u) {
        let mut x14 = s;
        x14.perm.insert(u.to_string(), false);
        return tl("FLR.S.perm.speaker", vec![("u", u.to_string())], prevoke_enter(x14, u, u32::from(revoke::NO_PERMISSION), c));
    }
    if s.in_q(u) {
        let mut x15 = s;
        x15.perm.insert(u.to_string(), false);
        let x16 = x15.dequeue(u);
        return tl("FLR.S.perm.queued", vec![("u", u.to_string())], part_a(x16, u, &M::new(T::Deny, None, u32::from(deny::NO_PERMISSION), 0), c));
    }
    let mut x17 = s;
    x17.perm.insert(u.to_string(), false);
    tl("FLR.S.perm.other", vec![("u", u.to_string())], r0(x17))
}

pub fn perm_up(s: Srv, u: &str) -> R {
    let mut x18 = s;
    x18.perm.insert(u.to_string(), true);
    tl("FLR.S.perm.up", vec![("u", u.to_string())], r0(x18))
}

pub fn away(s: Srv, u: &str, c: &Cfg) -> R {
    if s.g == G::Taken && s.speaker.as_deref() == Some(u) {
        let mut x19 = s;
        x19.prev = Some(u.to_string());
        return tl("FLR.S.G.TAKEN.client_release", vec![("u", u.to_string())], idle_enter(x19, c));
    }
    if s.g == G::Prevoke && s.revoking.as_deref() == Some(u) {
        return tl("FLR.S.G.PREVOKE.client_release", vec![("u", u.to_string())], r0(s));
    }
    if s.in_q(u) {
        return tl("FLR.S.away.queued", vec![("u", u.to_string())], r0(s.dequeue(u)));
    }
    tl("FLR.S.away.other", vec![("u", u.to_string())], r0(s))
}

pub fn ws_pub_select(s: Srv, u: &str) -> R {
    let mut x21 = s;
    x21.pub_.insert(u.to_string(), true);
    tl("FLR.S.ws.pub_select", vec![("u", u.to_string())], r0(x21))
}

pub fn ws_reg_half(s: Srv, u: &str) -> R {
    let mut x22 = s;
    x22.half.insert(u.to_string(), true);
    tl("FLR.S.ws.reg_half", vec![("u", u.to_string())], r0(x22))
}

pub fn ws_pub_away(s: Srv, u: &str, c: &Cfg) -> R {
    let mut x23 = s;
    x23.pub_.insert(u.to_string(), false);
    tl("FLR.S.ws.pub_away", vec![("u", u.to_string())], away(x23, u, c))
}

pub fn ws_leave(s: Srv, u: &str, c: &Cfg) -> R {
    let mut x24 = s;
    x24.pub_.insert(u.to_string(), false);
    let u2 = u.to_string();
    let r = then(away(x24, u, c), move |mut y25| {
        y25.members.remove(&u2);
        y25.retry.insert(u2.clone(), false);
        y25.part.insert(u2, P::Releasing);
        r0(y25)
    });
    tl("FLR.S.ws.leave", vec![("u", u.to_string())], r)
}

pub fn part_a(s: Srv, u: &str, ev: &M, c: &Cfg) -> R {
    let _ = c;
    let pu = s.part(u);
    if ev.t == T::Qinfo {
        return tl("FLR.S.P.any.a_QINFO", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(s, u, T::Qinfo, None, ev.n, ev.d));
    }
    if ev.t == T::Deny {
        return tl("FLR.S.P.any.a_DENY", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(s, u, T::Deny, None, ev.n, 0));
    }
    let evq = || Ev { t: ev.t, a: None, n: ev.n, d: ev.d, q: ev.q };
    match (pu, ev.t) {
        (P::NpIdle, T::Granted) => {
            tl("FLR.S.P.NP_IDLE.a_GRANTED", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_eq(s, u, P::Permitted, evq()))
        }
        (P::NpIdle, T::Taken) => {
            let mut x27 = s;
            x27.tq.insert(u.to_string(), ev.q);
            tl(
                "FLR.S.P.NP_IDLE.a_TAKEN",
                vec![("u", u.to_string()), ("ev", ev.t.name().to_string())],
                send_e(x27, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q),
            )
        }
        (P::NpIdle, T::Idle) => {
            tl("FLR.S.P.NP_IDLE.a_IDLE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(s, u, T::Idle, ev.a.clone(), 0, ev.q))
        }
        (P::NpIdle, _) => tl("FLR.S.P.NP_IDLE.a_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpTaken, T::Granted) => {
            tl("FLR.S.P.NP_TAKEN.a_GRANTED", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_eq(s, u, P::Permitted, evq()))
        }
        (P::NpTaken, T::Taken) => {
            let mut x28 = s;
            x28.tq.insert(u.to_string(), ev.q);
            x28.c21.insert(u.to_string(), 1);
            tl("FLR.S.P.NP_TAKEN.a_TAKEN", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(x28, u, T::Taken, ev.a.clone(), 0, ev.q))
        }
        (P::NpTaken, T::Idle) => {
            tl("FLR.S.P.NP_TAKEN.a_IDLE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_e(s, u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q))
        }
        (P::NpTaken, _) => tl("FLR.S.P.NP_TAKEN.a_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::Permitted, T::Granted) => {
            tl("FLR.S.P.PERMITTED.a_GRANTED", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_q(s, u, T::Granted, None, ev.n, ev.d, ev.q))
        }
        (P::Permitted, T::Idle) => tl("FLR.S.P.PERMITTED.a_IDLE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], enter_r(s, u, P::NpIdle)),
        (P::Permitted, T::Taken) => {
            let mut x29 = s;
            x29.tq.insert(u.to_string(), ev.q);
            tl(
                "FLR.S.P.PERMITTED.a_TAKEN",
                vec![("u", u.to_string()), ("ev", ev.t.name().to_string())],
                send_e(x29, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q),
            )
        }
        (P::Permitted, T::Revoke) => {
            let mut x30 = s;
            x30.rcause.insert(u.to_string(), ev.n);
            tl("FLR.S.P.PERMITTED.a_REVOKE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_e(x30, u, P::Prevoke, T::Revoke, None, ev.n, 0))
        }
        (P::Permitted, _) => tl("FLR.S.P.PERMITTED.a_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::Prevoke, T::Idle) => {
            tl("FLR.S.P.PREVOKE.a_IDLE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_e(s, u, P::NpIdle, T::Idle, ev.a.clone(), 0, ev.q))
        }
        (P::Prevoke, T::Taken) => {
            let mut x31 = s;
            x31.tq.insert(u.to_string(), ev.q);
            tl(
                "FLR.S.P.PREVOKE.a_TAKEN",
                vec![("u", u.to_string()), ("ev", ev.t.name().to_string())],
                send_e(x31, u, P::NpTaken, T::Taken, ev.a.clone(), 0, ev.q),
            )
        }
        (P::Prevoke, _) => tl("FLR.S.P.PREVOKE.a_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::Releasing, _) => tl("FLR.S.P.RELEASING.a_any", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpSends, T::Idle) => {
            let mut x32 = s;
            x32.g_taken.insert(u.to_string(), false);
            tl("FLR.S.P.NPSENDS.a_IDLE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(x32))
        }
        (P::NpSends, T::Taken) => {
            let mut x33 = s;
            x33.g_taken.insert(u.to_string(), true);
            tl("FLR.S.P.NPSENDS.a_TAKEN", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(x33))
        }
        (P::NpSends, _) => tl("FLR.S.P.NPSENDS.a_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
    }
}

fn next_seq(s: &Srv) -> u32 {
    if s.seq + 1 == SEQ_WRAP { 0 } else { s.seq + 1 }
}

pub fn part_c(s: Srv, u: &str, ev: &M, c: &Cfg) -> R {
    let pu = s.part(u);
    match (pu, ev.t) {
        (P::NpIdle, T::Request) => tl("FLR.S.P.NP_IDLE.rx_REQUEST", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_req_idle(s, u, ev.n, c)),
        (P::NpIdle, T::Release) => {
            let x34 = s.dequeue(u);
            let prev = x34.prev.clone();
            tl("FLR.S.P.NP_IDLE.rx_RELEASE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(x34, u, T::Idle, prev, 0, 0))
        }
        (P::NpIdle, T::Rtp) if b(&s.released, u) => {
            tl("FLR.S.P.NP_IDLE.rx_RTP_after_release", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], revoke3(s, u))
        }
        (P::NpIdle, T::Rtp) => tl("FLR.S.P.NP_IDLE.rx_RTP_drop", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpIdle, _) => tl("FLR.S.P.NP_IDLE.rx_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpTaken, T::Request) => tl("FLR.S.P.NP_TAKEN.rx_REQUEST", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], taken_req(s, u, ev.n, c)),
        (P::NpTaken, T::Release) => {
            let mut x35 = s;
            let q = next_seq(&x35);
            x35.tq.insert(u.to_string(), q);
            x35.c21.insert(u.to_string(), 1);
            let x36 = x35.dequeue(u);
            let h = x36.holder();
            tl("FLR.S.P.NP_TAKEN.rx_RELEASE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(x36, u, T::Taken, h, 0, 0))
        }
        (P::NpTaken, T::Ack) if ev.n == 2 && ev.q == n(&s.tq, u) => {
            let mut x37 = s;
            x37.c21.insert(u.to_string(), 0);
            tl("FLR.S.P.NP_TAKEN.rx_ACK_taken", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(x37))
        }
        (P::NpTaken, T::Ack) if ev.n == 2 => tl("FLR.S.P.NP_TAKEN.rx_ACK_taken_stale", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpTaken, T::Ack) => tl("FLR.S.P.NP_TAKEN.rx_ACK_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::NpTaken, T::QposReq) => {
            let (p, q) = (s.pos(u), s.q_prio(u));
            tl("FLR.S.P.NP_TAKEN.rx_QPOS_REQ", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send(s, u, T::Qinfo, None, p, q))
        }
        (P::NpTaken, T::Rtp) => tl("FLR.S.P.NP_TAKEN.rx_RTP", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], revoke3(s, u)),
        (P::NpTaken, _) => tl("FLR.S.P.NP_TAKEN.rx_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::Permitted, T::Release) => {
            let mut x38 = s;
            x38.released.insert(u.to_string(), true);
            tl("FLR.S.P.PERMITTED.rx_RELEASE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_rel(x38, u, c))
        }
        (P::Permitted, T::Request) => {
            tl("FLR.S.P.PERMITTED.rx_REQUEST", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_req_permitted(s, u, c))
        }
        (P::Permitted, T::Rtp) => tl("FLR.S.P.PERMITTED.rx_RTP", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_rtp(s, u)),
        (P::Permitted, _) => tl("FLR.S.P.PERMITTED.rx_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (P::Prevoke, T::Release) => tl("FLR.S.P.PREVOKE.rx_RELEASE", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_rel(s, u, c)),
        (P::Prevoke, T::Rtp) => tl("FLR.S.P.PREVOKE.rx_RTP", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], arb_rtp(s, u)),
        (P::Prevoke, _) => tl("FLR.S.P.PREVOKE.rx_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        (_, T::Release) if b(&s.g_taken, u) => {
            let mut x39 = s;
            let q = next_seq(&x39);
            x39.tq.insert(u.to_string(), q);
            let h = x39.holder();
            tl("FLR.S.P.NPSENDS.rx_RELEASE_taken", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_e(x39, u, P::NpTaken, T::Taken, h, 0, 0))
        }
        (_, T::Release) => {
            let prev = s.prev.clone();
            tl("FLR.S.P.NPSENDS.rx_RELEASE_idle", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], send_e(s, u, P::NpIdle, T::Idle, prev, 0, 0))
        }
        (_, T::Rtp) => tl("FLR.S.P.NPSENDS.rx_RTP", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
        _ => tl("FLR.S.P.NPSENDS.rx_other", vec![("u", u.to_string()), ("ev", ev.t.name().to_string())], r0(s)),
    }
}

pub fn enter_r(s: Srv, u: &str, p: P) -> R {
    let mut x = s;
    let k = u.to_string();
    x.part.insert(k.clone(), p);
    match p {
        P::NpIdle => {
            x.g_taken.insert(k.clone(), false);
            x.c21.insert(k, 0);
            tl("FLR.S.P.NP_IDLE.enter", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(x))
        }
        P::NpTaken => {
            x.g_taken.insert(k.clone(), true);
            x.c21.insert(k, 1);
            tl("FLR.S.P.NP_TAKEN.enter", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(x))
        }
        P::Permitted => {
            x.released.insert(k.clone(), false);
            x.c21.insert(k, 0);
            tl("FLR.S.P.PERMITTED.enter", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(x))
        }
        P::Prevoke => {
            x.c8.insert(k.clone(), 1);
            x.c21.insert(k, 0);
            tl("FLR.S.P.PREVOKE.enter", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(x))
        }
        _ => {
            x.c8.insert(k.clone(), 1);
            x.c21.insert(k, 0);
            tl("FLR.S.P.NPSENDS.enter", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(x))
        }
    }
}

pub fn revoke3(s: Srv, u: &str) -> R {
    let mut x45 = s;
    let c = u32::from(revoke::NO_PERMISSION);
    x45.rcause.insert(u.to_string(), c);
    send_e(x45, u, P::NpSends, T::Revoke, None, c, 0)
}

pub fn taken_req(s: Srv, u: &str, p: u32, c: &Cfg) -> R {
    if b(&s.retry, u) {
        return tl(
            "FLR.S.P.NP_TAKEN.req_retry_after",
            vec![("u", u.to_string()), ("p", format!("{p:?}"))],
            send(s, u, T::Deny, None, u32::from(deny::RETRY_AFTER), 0),
        );
    }
    if s.in_q(u) {
        let (pp, qp) = (s.pos(u), s.q_prio(u));
        return tl("FLR.S.P.NP_TAKEN.req_renotify", vec![("u", u.to_string()), ("p", format!("{p:?}"))], send(s, u, T::Qinfo, None, pp, qp));
    }
    if p > s.holder_prio() && !s.pre_q() {
        return tl("FLR.S.P.NP_TAKEN.req_preempt", vec![("u", u.to_string()), ("p", format!("{p:?}"))], arb_preempt(s, u, p, c));
    }
    if p > s.holder_prio() {
        return tl("FLR.S.P.NP_TAKEN.req_preempt_blocked", vec![("u", u.to_string()), ("p", format!("{p:?}"))], r0(s));
    }
    if s.queue.len() as u32 >= c.qmax {
        return tl("FLR.S.P.NP_TAKEN.req_full", vec![("u", u.to_string()), ("p", format!("{p:?}"))], send(s, u, T::Deny, None, u32::from(deny::QUEUE_FULL), 0));
    }
    let mut x46 = s;
    x46.queue = insert_q(&x46.queue, Waiter { u: u.to_string(), p, pre: false });
    let pos = x46.pos(u);
    tl("FLR.S.P.NP_TAKEN.req_enqueue", vec![("u", u.to_string()), ("p", format!("{p:?}"))], send(x46, u, T::Qinfo, None, pos, p))
}

pub fn srv_t1(s: Srv, c: &Cfg) -> R {
    if s.g == G::Taken {
        let mut x47 = s;
        x47.prev = x47.speaker.clone();
        return tl("FLR.S.G.TAKEN.T1", Vec::new(), idle_enter(x47, c));
    }
    let mut x48 = s;
    x48.prev = x48.revoking.clone();
    tl("FLR.S.G.PREVOKE.T1", Vec::new(), idle_enter(x48, c))
}

pub fn srv_t2(s: Srv, c: &Cfg) -> R {
    let u = s.speaker.clone().unwrap_or_default();
    let mut x49 = s;
    x49.retry.insert(u.clone(), true);
    tl("FLR.S.G.TAKEN.T2", Vec::new(), prevoke_enter(x49, &u, u32::from(revoke::TOO_LONG), c))
}

pub fn srv_t3(s: Srv, c: &Cfg) -> R {
    let mut x50 = s;
    x50.prev = x50.revoking.clone();
    tl("FLR.S.G.PREVOKE.T3", Vec::new(), idle_enter(x50, c))
}

pub fn srv_t7(s: Srv, c: &Cfg) -> R {
    let mut x51 = s;
    x51.c7 += 1;
    let prev = x51.prev.clone();
    tl("FLR.S.G.IDLE.T7", Vec::new(), fan(x51, M::new(T::Idle, prev, 0, 0), None, c))
}

pub fn srv_t20(s: Srv, c: &Cfg) -> R {
    if s.c20 < u32::from(timers::C20) {
        let mut x52 = s;
        x52.c20 += 1;
        let sp = x52.speaker.clone().unwrap_or_default();
        let g = granted(&x52, T2_SECS);
        return tl("FLR.S.G.TAKEN.T20", Vec::new(), part_a(x52, &sp, &g, c));
    }
    let mut x53 = s;
    x53.c20 += 1;
    tl("FLR.S.G.TAKEN.T20_exhaust", Vec::new(), r0(x53))
}

pub fn srv_t8(s: Srv, u: &str) -> R {
    let mut x = s;
    let c8 = n(&x.c8, u) + 1;
    x.c8.insert(u.to_string(), c8);
    if x.part(u) == P::Prevoke {
        let cause = n(&x.rcause, u);
        return tl("FLR.S.P.PREVOKE.T8", vec![("u", u.to_string())], send(x, u, T::Revoke, None, cause, 0));
    }
    tl("FLR.S.P.NPSENDS.T8", vec![("u", u.to_string())], send(x, u, T::Revoke, None, u32::from(revoke::NO_PERMISSION), 0))
}

pub fn srv_t21(s: Srv, u: &str) -> R {
    let mut x = s;
    let c21 = n(&x.c21, u);
    x.c21.insert(u.to_string(), c21 + 1);
    if c21 < u32::from(timers::C21) {
        let h = x.holder();
        let q = n(&x.tq, u);
        return tl("FLR.S.P.NP_TAKEN.T21", vec![("u", u.to_string())], send(x, u, T::Taken, h, 0, q));
    }
    tl("FLR.S.P.NP_TAKEN.T21_exhaust", vec![("u", u.to_string())], r0(x))
}

pub fn srv_t9(s: Srv, u: &str) -> R {
    let mut x58 = s;
    x58.retry.insert(u.to_string(), false);
    tl("FLR.S.T9.expire", vec![("u", u.to_string())], r0(x58))
}

pub fn armed_server(s: &Srv) -> Vec<&'static str> {
    let mut on = Vec::new();
    if matches!(s.g, G::Taken | G::Prevoke) {
        on.push("T1");
    }
    if s.g == G::Taken && s.first_rtp {
        on.push("T2");
    }
    if s.g == G::Prevoke {
        on.push("T3");
    }
    if s.g == G::Idle && s.idle_bcast && s.c7 < u32::from(timers::C7) {
        on.push("T7");
    }
    if s.g == G::Taken && s.succ && !s.rtp_seen && s.c20 <= u32::from(timers::C20) {
        on.push("T20");
    }
    on
}

pub fn armed_participant(s: &Srv, u: &str) -> Vec<&'static str> {
    let mut on = Vec::new();
    let p = s.part(u);
    if matches!(p, P::Prevoke | P::NpSends) && n(&s.c8, u) < u32::from(timers::C8) {
        on.push("T8");
    }
    if b(&s.retry, u) {
        on.push("T9");
    }
    let c21 = n(&s.c21, u);
    if p == P::NpTaken && c21 >= 1 && c21 <= u32::from(timers::C21) {
        on.push("T21");
    }
    on
}

pub fn timer_ms(name: &str) -> u64 {
    match name {
        "T1" => timers::T1_MS,
        "T2" => timers::T2_MS,
        "T3" => timers::T3_MS,
        "T7" => timers::T7_MS,
        "T8" => timers::T8_MS,
        "T9" => timers::T9_MS,
        "T20" => timers::T20_MS,
        "T21" => timers::T21_MS,
        _ => u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Cfg {
        Cfg { qmax: 10, t2_remain_secs: 30 }
    }

    fn room(users: &[&str]) -> Srv {
        let mut s = Srv::default();
        for u in users {
            s.members.insert((*u).to_string());
            s.pub_.insert((*u).to_string(), true);
            s.half.insert((*u).to_string(), true);
        }
        s
    }

    fn req(p: u32) -> M {
        M::new(T::Request, None, p, 0)
    }

    #[test]
    fn grant_broadcasts_taken_and_binds() {
        let c = cfg();
        let r = server_rx(room(&["a", "b", "c"]), "a", &req(0), &c);
        assert_eq!(r.s.g, G::Taken);
        assert_eq!(r.o[0].to, "a");
        assert_eq!(r.o[0].m.t, T::Granted);
        assert_eq!(r.o[0].m.d, 30);
        let taken: Vec<&Out> = r.o.iter().filter(|o| o.m.t == T::Taken).collect();
        assert_eq!(taken.len(), 2);
        assert!(taken.iter().all(|o| o.m.q == 1 && o.m.a.as_deref() == Some("a")));
        assert_eq!(r.fx, vec![Fx::Bind("a".into())]);
        assert_eq!(r.s.part("b"), P::NpTaken);
        assert!(armed_participant(&r.s, "b").contains(&"T21"));
    }

    #[test]
    fn queue_then_succession() {
        let c = cfg();
        let r = server_rx(room(&["a", "b", "c"]), "a", &req(0), &c);
        let r = server_rx(r.s, "b", &req(0), &c);
        assert_eq!(r.o[0].m.t, T::Qinfo);
        assert_eq!(r.o[0].m.n, 1);
        let r = server_rx(r.s, "a", &M::new(T::Release, None, 0, 0), &c);
        assert_eq!(r.s.speaker.as_deref(), Some("b"));
        assert!(r.s.succ);
        assert!(r.l.iter().any(|(l, _)| *l == "FLR.S.G.IDLE.enter_succession"));
        assert!(!r.o.iter().any(|o| o.m.t == T::Idle), "큐가 있는데 IDLE 을 보내지 않는다");
    }

    #[test]
    fn gate_reasons() {
        let c = cfg();
        let mut s = room(&["a", "b"]);
        s.half.insert("b".into(), false);
        let r = server_rx(s, "b", &req(0), &c);
        assert_eq!((r.o[0].m.t, r.o[0].m.n), (T::Deny, u32::from(deny::RECV_ONLY)));
        let r = server_rx(r.s, "z", &req(0), &c);
        assert_eq!(r.o[0].m.n, u32::from(deny::OTHER_REASON));
        let mut s = r.s;
        s.half.insert("b".into(), true);
        s.pub_.insert("b".into(), false);
        assert_eq!(server_rx(s, "b", &req(0), &c).o[0].m.n, u32::from(deny::NOT_PUBROOM));
    }

    #[test]
    fn preempt_higher_priority() {
        let c = cfg();
        let r = server_rx(room(&["a", "b"]), "a", &req(0), &c);
        let r = server_rx(r.s, "b", &req(5), &c);
        assert_eq!(r.s.g, G::Prevoke);
        assert_eq!(r.o[0].to, "a");
        assert_eq!((r.o[0].m.t, r.o[0].m.n), (T::Revoke, u32::from(revoke::PREEMPTED)));
        let r = server_rx(r.s, "a", &M::new(T::Release, None, 0, 0), &c);
        assert_eq!(r.s.speaker.as_deref(), Some("b"));
    }

    #[test]
    fn ack_stops_t21_with_same_seq() {
        let c = cfg();
        let r = server_rx(room(&["a", "b"]), "a", &req(0), &c);
        let q = r.s.tq["b"];
        let r = srv_t21(r.s, "b");
        assert_eq!(r.o[0].m.q, q, "T21 재전송은 같은 번호");
        let r = server_rx(r.s, "b", &M { t: T::Ack, a: None, n: 2, d: 0, q }, &c);
        assert!(r.l.iter().any(|(l, _)| *l == "FLR.S.P.NP_TAKEN.rx_ACK_taken"));
        assert!(!armed_participant(&r.s, "b").contains(&"T21"));
    }

    #[test]
    fn first_notice_unicast_no_t7() {
        let r = ready(room(&["a", "b"]), "b");
        assert_eq!(r.o[0].m.t, T::Idle);
        assert_eq!(r.s.g, G::Idle);
        assert!(armed_server(&r.s).is_empty());
    }

    #[test]
    fn t2_revokes_with_retry_after() {
        let c = cfg();
        let r = server_rx(room(&["a", "b"]), "a", &req(0), &c);
        let r = server_rx(r.s, "a", &M::new(T::Rtp, None, 0, 0), &c);
        assert!(armed_server(&r.s).contains(&"T2"));
        let r = srv_t2(r.s, &c);
        assert_eq!(r.s.g, G::Prevoke);
        assert!(r.s.retry["a"]);
        let r = srv_t3(r.s, &c);
        assert_eq!(r.s.g, G::Idle);
        let r = server_rx(r.s, "a", &req(0), &c);
        assert_eq!(r.o[0].m.n, u32::from(deny::RETRY_AFTER));
    }
}
