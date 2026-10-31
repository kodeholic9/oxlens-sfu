// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! 그물 코어 — 상태를 혼자 쥐는 태스크 하나 · 세션 신원(dialer · boot · sno)으로 판정(S1 새 세션만 · S2 정리는 세션으로) · 세션이 서면 선언 전부(S3) · 한 홉 · lease · 틱 공백 울타리(S4) · 질의 펜싱.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::{AddAssign, SubAssign};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

use crate::api::{Info, Query, Reply, Sample, SessInfo, TokenEvent};
use crate::keyexpr::{concrete, intersects};
use crate::session::{self, Clock, Outbox, Push, ReadCtx, SessEv};
use crate::wire::{CloseReason, DeclKind, Hello, Msg, PROTO, Role, TokenEvtKind};

const TICK: Duration = Duration::from_millis(100);
const HANDSHAKE: Duration = Duration::from_secs(5);
const BACKOFF_MIN: u64 = 100;
const BACKOFF_MAX: u64 = 1000;

pub enum Cmd {
    DeclareToken { key: String, ack: oneshot::Sender<u64> },
    Subscribe { expr: String, tx: mpsc::UnboundedSender<Sample>, ack: oneshot::Sender<u64> },
    Queryable { expr: String, tx: mpsc::UnboundedSender<Query>, ack: oneshot::Sender<u64> },
    TokenSub { expr: String, history: bool, tx: mpsc::UnboundedSender<TokenEvent>, ack: oneshot::Sender<u64> },
    Undeclare { kind: DeclKind, id: u64 },
    TokenUnsub { id: u64 },
    Put { key: String, payload: Vec<u8> },
    Get { expr: String, payload: Vec<u8>, timeout: Duration, tx: mpsc::UnboundedSender<Reply> },
    LocalReply { qid: u64, qabl: u64, key: String, ok: bool, payload: Vec<u8> },
    LocalFinal { qid: u64, qabl: u64 },
    Info { ack: oneshot::Sender<Info> },
    Close { ack: oneshot::Sender<()> },
}

pub enum Ev {
    Accepted { stream: TcpStream, hello: Hello, addr: SocketAddr },
    Dialed { target: String, sno: u64, res: Option<(TcpStream, Hello, SocketAddr)> },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Owner {
    Local(u64),
    Sess(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Target {
    Local(u64),
    Sess(u64),
}

enum Origin {
    Local(mpsc::UnboundedSender<Reply>),
    Sess { uid: u64, qid: u64 },
}

struct Pending {
    origin: Origin,
    targets: HashSet<Target>,
    deadline: u64,
}

struct LTokenSub {
    expr: String,
    tx: mpsc::UnboundedSender<TokenEvent>,
    seen: HashSet<String>,
}

struct Sess {
    peer: String,
    role: Role,
    dialer: String,
    boot: u64,
    sno: u64,
    addr: String,
    adv: Option<String>,
    ob: Arc<Outbox>,
    last_rx: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    reader: tokio::task::AbortHandle,
    decls: HashMap<(DeclKind, u64), String>,
    tsubs: HashMap<u64, String>,
    peers: Vec<(String, String)>,
    since: u64,
}

struct Dial {
    seed: bool,
    node: Option<String>,
    busy: bool,
    next: u64,
    backoff: u64,
}

#[derive(Default)]
struct SubIndex {
    exprs: HashMap<(Owner, u64), String>,
    exact: HashMap<String, HashSet<(Owner, u64)>>,
    wild: HashSet<(Owner, u64)>,
}

impl SubIndex {
    fn add(&mut self, o: Owner, id: u64, expr: &str) {
        self.remove(o, id);
        if concrete(expr) {
            self.exact.entry(expr.to_string()).or_default().insert((o, id));
        } else {
            self.wild.insert((o, id));
        }
        self.exprs.insert((o, id), expr.to_string());
    }

    fn remove(&mut self, o: Owner, id: u64) {
        let Some(e) = self.exprs.remove(&(o, id)) else { return };
        if let Some(set) = self.exact.get_mut(&e) {
            set.remove(&(o, id));
            if set.is_empty() {
                self.exact.remove(&e);
            }
        }
        self.wild.remove(&(o, id));
    }

    fn matching(&self, key: &str) -> Vec<(Owner, u64)> {
        let mut out: Vec<(Owner, u64)> = self.exact.get(key).map(|s| s.iter().copied().collect()).unwrap_or_default();
        out.extend(self.wild.iter().filter(|k| self.exprs.get(k).is_some_and(|e| intersects(e, key))).copied());
        out
    }
}

pub struct Core {
    node: String,
    router: bool,
    lease_ms: u64,
    fence_ms: u64,
    ka: Duration,
    boot: u64,
    listen: Vec<String>,
    clock: Arc<Clock>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    sess_tx: mpsc::UnboundedSender<SessEv>,
    ev_tx: mpsc::UnboundedSender<Ev>,
    next_uid: u64,
    next_id: u64,
    next_sno: u64,
    sessions: HashMap<u64, Sess>,
    by_peer: HashMap<String, u64>,
    l_tokens: HashMap<u64, String>,
    l_subs: HashMap<u64, (String, mpsc::UnboundedSender<Sample>)>,
    l_qabls: HashMap<u64, (String, mpsc::UnboundedSender<Query>)>,
    l_tsubs: HashMap<u64, LTokenSub>,
    tok: HashMap<String, usize>,
    subs: SubIndex,
    out: BTreeMap<u64, (DeclKind, String)>,
    south: HashMap<(u64, DeclKind, u64), u64>,
    pending: HashMap<u64, Pending>,
    dials: HashMap<String, Dial>,
    known: HashMap<String, String>,
    partial: HashMap<String, (u64, bool)>,
    last_partial_check: u64,
    accepts: Vec<tokio::task::AbortHandle>,
}

pub struct Wiring {
    pub cmd_tx: mpsc::UnboundedSender<Cmd>,
    pub ev_tx: mpsc::UnboundedSender<Ev>,
    pub sess_tx: mpsc::UnboundedSender<SessEv>,
    pub accepts: Vec<tokio::task::AbortHandle>,
}

impl Core {
    pub fn new(node: String, router: bool, lease: Duration, listen: Vec<String>, seeds: Vec<String>, w: Wiring) -> Core {
        let lease_ms = lease.as_millis().max(400) as u64;
        let ka_ms = lease_ms / 4;
        let boot = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(1);
        let dials = seeds.into_iter().map(|a| (a, Dial { seed: true, node: None, busy: false, next: 0, backoff: BACKOFF_MIN })).collect();
        Core {
            node,
            router,
            lease_ms,
            fence_ms: lease_ms - ka_ms,
            ka: Duration::from_millis(ka_ms),
            boot,
            listen,
            clock: Arc::new(Clock::new()),
            cmd_tx: w.cmd_tx,
            sess_tx: w.sess_tx,
            ev_tx: w.ev_tx,
            next_uid: 1,
            next_id: 1,
            next_sno: 1,
            sessions: HashMap::new(),
            by_peer: HashMap::new(),
            l_tokens: HashMap::new(),
            l_subs: HashMap::new(),
            l_qabls: HashMap::new(),
            l_tsubs: HashMap::new(),
            tok: HashMap::new(),
            subs: SubIndex::default(),
            out: BTreeMap::new(),
            south: HashMap::new(),
            pending: HashMap::new(),
            dials,
            known: HashMap::new(),
            partial: HashMap::new(),
            last_partial_check: 0,
            accepts: w.accepts,
        }
    }

    pub async fn run(mut self, mut cmd_rx: mpsc::UnboundedReceiver<Cmd>, mut ev_rx: mpsc::UnboundedReceiver<Ev>, mut sess_rx: mpsc::UnboundedReceiver<SessEv>) {
        self.clock.tick();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = tick.tick() => self.on_tick(),
                Some(e) = sess_rx.recv() => self.on_sess(e),
                Some(e) = ev_rx.recv() => self.on_ev(e).await,
                c = cmd_rx.recv() => match c {
                    Some(Cmd::Close { ack }) => {
                        self.shutdown();
                        let _ = ack.send(());
                        return;
                    }
                    Some(c) => self.on_cmd(c),
                    None => {
                        self.shutdown();
                        return;
                    }
                },
            }
        }
    }

    fn id(&mut self) -> u64 {
        let v = self.next_id;
        self.next_id += 1;
        v
    }

    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    fn shutdown(&mut self) {
        for a in self.accepts.drain(..) {
            a.abort();
        }
        self.l_tsubs.clear();
        self.l_subs.clear();
        self.l_qabls.clear();
        self.pending.clear();
        let uids: Vec<u64> = self.sessions.keys().copied().collect();
        for u in uids {
            self.close(u, CloseReason::Shutdown);
        }
        tracing::info!(kind = "diag", me = %self.node, "mesh closed");
    }

    fn on_tick(&mut self) {
        let gap = self.clock.tick();
        if gap > self.fence_ms {
            self.fence(gap);
            return;
        }
        let now = self.now();
        let expired: Vec<u64> = self
            .sessions
            .iter()
            .filter(|(_, s)| now.saturating_sub(s.last_rx.load(Ordering::Acquire)) > self.lease_ms)
            .map(|(u, _)| *u)
            .collect();
        for u in expired {
            self.close(u, CloseReason::Expired);
        }
        let late: Vec<u64> = self.pending.iter().filter(|(_, p)| p.deadline <= now).map(|(q, _)| *q).collect();
        for q in late {
            self.finish(q);
        }
        self.dial_wanted(now);
        if self.router && now.saturating_sub(self.last_partial_check) >= 1000 {
            self.last_partial_check = now;
            self.check_partial(now);
        }
    }

    fn fence(&mut self, gap: u64) {
        tracing::warn!(kind = "diag", me = %self.node, gap_ms = gap, "mesh fenced — tick gap over lease − keepalive · all sessions closed · stale bytes not trusted");
        let uids: Vec<u64> = self.sessions.keys().copied().collect();
        for u in uids {
            self.close(u, CloseReason::Fenced);
        }
        for d in self.dials.values_mut() {
            d.next = 0;
            d.backoff = BACKOFF_MIN;
        }
        self.clock.tick();
    }

    fn dial_wanted(&mut self, now: u64) {
        let mut want: Vec<String> = Vec::new();
        if self.router {
            let mut by_seed: HashSet<&String> = HashSet::new();
            for (a, d) in &self.dials {
                if d.seed && !d.node.as_ref().is_some_and(|n| self.by_peer.contains_key(n)) {
                    want.push(a.clone());
                    if let Some(n) = &d.node {
                        by_seed.insert(n);
                    }
                }
            }
            for (n, a) in &self.known {
                if *n > self.node && !self.by_peer.contains_key(n) && !by_seed.contains(n) {
                    want.push(a.clone());
                }
            }
        } else if self.sessions.is_empty() {
            want.extend(self.dials.iter().filter(|(_, d)| d.seed).map(|(a, _)| a.clone()));
        }
        want.sort();
        want.dedup();
        for a in want {
            let d = self.dials.entry(a.clone()).or_insert(Dial { seed: false, node: None, busy: false, next: 0, backoff: BACKOFF_MIN });
            if d.busy || now < d.next {
                continue;
            }
            d.busy = true;
            let sno = self.next_sno;
            self.next_sno += 1;
            let hello = self.my_hello(sno);
            let tx = self.ev_tx.clone();
            tokio::spawn(async move {
                let res = dial(&a, &hello).await;
                let _ = tx.send(Ev::Dialed { target: a, sno, res });
            });
        }
    }

    fn check_partial(&mut self, now: u64) {
        let mut seen: HashMap<String, Vec<String>> = HashMap::new();
        for s in self.sessions.values().filter(|s| s.role == Role::Router) {
            for (n, _) in &s.peers {
                if *n != self.node && !self.by_peer.contains_key(n) {
                    seen.entry(n.clone()).or_default().push(s.peer.clone());
                }
            }
        }
        self.partial.retain(|n, _| seen.contains_key(n));
        for (n, by) in seen {
            let e = self.partial.entry(n.clone()).or_insert((now, false));
            if !e.1 && now.saturating_sub(e.0) > self.lease_ms {
                e.1 = true;
                tracing::warn!(kind = "diag", me = %self.node, missing = %n, seen_by = %by.join(","), "mesh partial — router seen by neighbours but not linked to me");
            }
        }
    }

    fn my_hello(&self, sno: u64) -> Hello {
        Hello {
            proto: PROTO,
            role: if self.router { Role::Router } else { Role::Client },
            node: self.node.clone(),
            boot: self.boot,
            sno,
            lease_ms: self.lease_ms as u32,
            listen: self.listen.clone(),
            peers: self.peer_list(),
        }
    }

    fn peer_list(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            self.sessions.values().filter(|s| s.role == Role::Router).filter_map(|s| s.adv.clone().map(|a| (s.peer.clone(), a))).collect();
        v.sort();
        v
    }

    async fn on_ev(&mut self, e: Ev) {
        match e {
            Ev::Accepted { mut stream, hello, addr } => {
                let mine = self.my_hello(hello.sno);
                if session::send_direct(&mut stream, &Msg::Hello(mine)).await.is_err() {
                    return;
                }
                if let Some(r) = self.accept_verdict(&hello) {
                    let _ = session::send_direct(&mut stream, &Msg::Close(r)).await;
                    return;
                }
                if let Some(&u) = self.by_peer.get(&hello.node) {
                    let r = if self.sessions.get(&u).is_some_and(|s| s.dialer == hello.node) { CloseReason::Replaced } else { CloseReason::Duplicate };
                    self.close(u, r);
                }
                let (dialer, boot, sno) = (hello.node.clone(), hello.boot, hello.sno);
                self.install(stream, hello, addr, dialer, boot, sno);
            }
            Ev::Dialed { target, sno, res } => {
                let now = self.now();
                let Some(d) = self.dials.get_mut(&target) else { return };
                d.busy = false;
                let Some((mut stream, hello, addr)) = res else {
                    d.next = now + d.backoff;
                    d.backoff = (d.backoff * 2).min(BACKOFF_MAX);
                    return;
                };
                d.backoff = BACKOFF_MIN;
                d.next = now + BACKOFF_MIN;
                d.node = Some(hello.node.clone());
                if hello.proto != PROTO || hello.node == self.node || (!self.router && hello.role != Role::Router) {
                    let _ = session::send_direct(&mut stream, &Msg::Close(CloseReason::Protocol)).await;
                    return;
                }
                if let Some(&u) = self.by_peer.get(&hello.node) {
                    let keep_old = match self.sessions.get(&u) {
                        Some(s) if s.dialer == self.node => s.sno > sno,
                        Some(_) => self.node > hello.node,
                        None => false,
                    };
                    if keep_old {
                        let _ = session::send_direct(&mut stream, &Msg::Close(CloseReason::Duplicate)).await;
                        return;
                    }
                    self.close(u, CloseReason::Duplicate);
                }
                let me = self.node.clone();
                let boot = self.boot;
                self.install(stream, hello, addr, me, boot, sno);
            }
        }
    }

    fn accept_verdict(&self, h: &Hello) -> Option<CloseReason> {
        if h.proto != PROTO || h.node == self.node || !self.router {
            return Some(CloseReason::Protocol);
        }
        let u = self.by_peer.get(&h.node)?;
        let s = self.sessions.get(u)?;
        if s.dialer == h.node {
            if (h.boot, h.sno) > (s.boot, s.sno) { None } else { Some(CloseReason::Refused) }
        } else if h.node < self.node {
            None
        } else {
            Some(CloseReason::Duplicate)
        }
    }

    fn install(&mut self, stream: TcpStream, hello: Hello, addr: SocketAddr, dialer: String, boot: u64, sno: u64) {
        let _ = stream.set_nodelay(true);
        let (r, w) = stream.into_split();
        let uid = self.next_uid;
        self.next_uid += 1;
        let ob = Arc::new(Outbox::new());
        let last_rx = Arc::new(AtomicU64::new(self.now()));
        let stop = Arc::new(AtomicBool::new(false));
        tokio::spawn(session::write_loop(w, ob.clone(), self.ka));
        let reader = tokio::spawn(session::read_loop(
            r,
            ReadCtx { uid, tx: self.sess_tx.clone(), clock: self.clock.clone(), last_rx: last_rx.clone(), fence_ms: self.fence_ms, stop: stop.clone() },
        ))
        .abort_handle();
        let adv = hello.listen.first().map(|l| advertise(l, addr));
        if let Some(a) = &adv
            && hello.role == Role::Router
        {
            self.known.insert(hello.node.clone(), a.clone());
        }
        for (n, a) in &hello.peers {
            if *n != self.node {
                self.known.insert(n.clone(), a.clone());
            }
        }
        tracing::info!(kind = "diag", me = %self.node, peer = %hello.node, dialer = %dialer, sno, addr = %addr, role = ?hello.role, "mesh session up");
        let s = Sess {
            peer: hello.node.clone(),
            role: hello.role,
            dialer,
            boot,
            sno,
            addr: addr.to_string(),
            adv,
            ob: ob.clone(),
            last_rx,
            stop,
            reader,
            decls: HashMap::new(),
            tsubs: HashMap::new(),
            peers: hello.peers.clone(),
            since: self.now(),
        };
        self.by_peer.insert(hello.node.clone(), uid);
        self.sessions.insert(uid, s);
        if self.router {
            if hello.role == Role::Router {
                for (rid, (k, e)) in &self.out {
                    ob.push(&Msg::Declare { kind: *k, id: *rid, expr: e.clone() });
                }
                ob.push(&Msg::DeclareDone);
                self.broadcast_peers();
            }
        } else {
            for (id, k) in &self.l_tokens {
                ob.push(&Msg::Declare { kind: DeclKind::Token, id: *id, expr: k.clone() });
            }
            for (id, (e, _)) in &self.l_subs {
                ob.push(&Msg::Declare { kind: DeclKind::Sub, id: *id, expr: e.clone() });
            }
            for (id, (e, _)) in &self.l_qabls {
                ob.push(&Msg::Declare { kind: DeclKind::Qabl, id: *id, expr: e.clone() });
            }
            for (id, t) in &self.l_tsubs {
                ob.push(&Msg::TokenSub { id: *id, expr: t.expr.clone(), history: true });
            }
            ob.push(&Msg::DeclareDone);
        }
    }

    fn broadcast_peers(&self) {
        let list = self.peer_list();
        for s in self.sessions.values().filter(|s| s.role == Role::Router) {
            s.ob.push(&Msg::Peers(list.clone()));
        }
    }

    fn close(&mut self, uid: u64, reason: CloseReason) {
        let Some(s) = self.sessions.remove(&uid) else { return };
        if self.by_peer.get(&s.peer) == Some(&uid) {
            self.by_peer.remove(&s.peer);
        }
        s.stop.store(true, Ordering::Release);
        s.reader.abort();
        let say = matches!(reason, CloseReason::Shutdown | CloseReason::Replaced | CloseReason::Refused | CloseReason::Duplicate | CloseReason::Protocol);
        s.ob.close(say.then_some(reason));
        tracing::info!(kind = "diag", me = %self.node, peer = %s.peer, dialer = %s.dialer, sno = s.sno, reason = reason.name(), "mesh session down");
        let mut deleted = false;
        for ((k, cid), e) in &s.decls {
            match k {
                DeclKind::Token => deleted |= self.tok_del(e),
                DeclKind::Sub => self.subs.remove(Owner::Sess(uid), *cid),
                DeclKind::Qabl => {}
            }
            if let Some(rid) = self.south.remove(&(uid, *k, *cid)) {
                self.out.remove(&rid);
                self.announce(&Msg::Undeclare { kind: *k, id: rid });
            }
        }
        if !self.router {
            for t in self.l_tsubs.values_mut() {
                for k in t.seen.drain() {
                    let _ = t.tx.send(TokenEvent::Delete(k));
                }
                let _ = t.tx.send(TokenEvent::BatchEnd);
            }
        } else if deleted {
            self.batch_end();
        }
        let qs: Vec<u64> = self.pending.keys().copied().collect();
        for q in qs {
            let Some(p) = self.pending.get_mut(&q) else { continue };
            if matches!(p.origin, Origin::Sess { uid: o, .. } if o == uid) {
                self.pending.remove(&q);
                continue;
            }
            if p.targets.remove(&Target::Sess(uid)) && p.targets.is_empty() {
                self.finish(q);
            }
        }
        if self.router && s.role == Role::Router {
            self.broadcast_peers();
        }
    }

    fn announce(&self, m: &Msg) {
        if !self.router {
            return;
        }
        for s in self.sessions.values().filter(|s| s.role == Role::Router) {
            s.ob.push(m);
        }
    }

    fn upstream(&self) -> Option<&Sess> {
        if self.router { None } else { self.sessions.values().next() }
    }

    fn tok_add(&mut self, key: &str) {
        let first = match self.tok.get_mut(key) {
            Some(c) => {
                c.add_assign(1);
                false
            }
            None => {
                self.tok.insert(key.to_string(), 1);
                true
            }
        };
        if first {
            self.tok_notify(TokenEvtKind::Put, key);
        }
    }

    fn tok_del(&mut self, key: &str) -> bool {
        match self.tok.get_mut(key) {
            None => return false,
            Some(c) if 1 < *c => {
                c.sub_assign(1);
                return false;
            }
            Some(_) => {}
        }
        self.tok.remove(key);
        self.tok_notify(TokenEvtKind::Delete, key);
        true
    }

    fn tok_notify(&mut self, kind: TokenEvtKind, key: &str) {
        for t in self.l_tsubs.values_mut().filter(|t| intersects(&t.expr, key)) {
            let ev = if kind == TokenEvtKind::Put { TokenEvent::Put(key.to_string()) } else { TokenEvent::Delete(key.to_string()) };
            let _ = t.tx.send(ev);
        }
        for s in self.sessions.values().filter(|s| s.role == Role::Client) {
            for (id, e) in &s.tsubs {
                if intersects(e, key) {
                    s.ob.push(&Msg::TokenEvt { id: *id, kind, key: key.to_string() });
                }
            }
        }
    }

    fn batch_end(&self) {
        for t in self.l_tsubs.values() {
            let _ = t.tx.send(TokenEvent::BatchEnd);
        }
        for s in self.sessions.values().filter(|s| s.role == Role::Client) {
            for id in s.tsubs.keys() {
                s.ob.push(&Msg::TokenEvt { id: *id, kind: TokenEvtKind::BatchEnd, key: String::new() });
            }
        }
    }

    fn history(&self, expr: &str) -> Vec<String> {
        let mut v: Vec<String> = self.tok.keys().filter(|k| intersects(expr, k)).cloned().collect();
        v.sort();
        v
    }

    fn on_cmd(&mut self, c: Cmd) {
        match c {
            Cmd::DeclareToken { key, ack } => {
                let id = self.id();
                self.l_tokens.insert(id, key.clone());
                if self.router {
                    self.tok_add(&key);
                    self.out.insert(id, (DeclKind::Token, key.clone()));
                    self.announce(&Msg::Declare { kind: DeclKind::Token, id, expr: key });
                } else if let Some(u) = self.upstream() {
                    u.ob.push(&Msg::Declare { kind: DeclKind::Token, id, expr: key });
                }
                let _ = ack.send(id);
            }
            Cmd::Subscribe { expr, tx, ack } => {
                let id = self.id();
                self.subs.add(Owner::Local(id), id, &expr);
                self.l_subs.insert(id, (expr.clone(), tx));
                if self.router {
                    self.out.insert(id, (DeclKind::Sub, expr.clone()));
                    self.announce(&Msg::Declare { kind: DeclKind::Sub, id, expr });
                } else if let Some(u) = self.upstream() {
                    u.ob.push(&Msg::Declare { kind: DeclKind::Sub, id, expr });
                }
                let _ = ack.send(id);
            }
            Cmd::Queryable { expr, tx, ack } => {
                let id = self.id();
                self.l_qabls.insert(id, (expr.clone(), tx));
                if self.router {
                    self.out.insert(id, (DeclKind::Qabl, expr.clone()));
                    self.announce(&Msg::Declare { kind: DeclKind::Qabl, id, expr });
                } else if let Some(u) = self.upstream() {
                    u.ob.push(&Msg::Declare { kind: DeclKind::Qabl, id, expr });
                }
                let _ = ack.send(id);
            }
            Cmd::TokenSub { expr, history, tx, ack } => {
                let id = self.id();
                if self.router {
                    if history {
                        for k in self.history(&expr) {
                            let _ = tx.send(TokenEvent::Put(k));
                        }
                        let _ = tx.send(TokenEvent::BatchEnd);
                    }
                } else if let Some(u) = self.upstream() {
                    u.ob.push(&Msg::TokenSub { id, expr: expr.clone(), history });
                } else if history {
                    let _ = tx.send(TokenEvent::BatchEnd);
                }
                self.l_tsubs.insert(id, LTokenSub { expr, tx, seen: HashSet::new() });
                let _ = ack.send(id);
            }
            Cmd::Undeclare { kind, id } => {
                let had = match kind {
                    DeclKind::Token => self.l_tokens.remove(&id).inspect(|k| {
                        if self.router {
                            self.tok_del(k);
                        }
                    }),
                    DeclKind::Sub => {
                        self.subs.remove(Owner::Local(id), id);
                        self.l_subs.remove(&id).map(|(e, _)| e)
                    }
                    DeclKind::Qabl => self.l_qabls.remove(&id).map(|(e, _)| e),
                };
                if had.is_some() {
                    if self.router {
                        self.out.remove(&id);
                        self.announce(&Msg::Undeclare { kind, id });
                    } else if let Some(u) = self.upstream() {
                        u.ob.push(&Msg::Undeclare { kind, id });
                    }
                }
            }
            Cmd::TokenUnsub { id } => {
                if self.l_tsubs.remove(&id).is_some()
                    && let Some(u) = self.upstream()
                {
                    u.ob.push(&Msg::TokenUnsub { id });
                }
            }
            Cmd::Put { key, payload } => self.route_put(None, &key, &payload),
            Cmd::Get { expr, payload, timeout, tx } => {
                let ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
                self.route_get(Origin::Local(tx), &expr, payload, ms);
            }
            Cmd::LocalReply { qid, qabl, key, ok, payload } => {
                if self.pending.get(&qid).is_some_and(|p| p.targets.contains(&Target::Local(qabl))) {
                    self.forward_reply(qid, key, ok, payload);
                }
            }
            Cmd::LocalFinal { qid, qabl } => self.target_done(qid, Target::Local(qabl)),
            Cmd::Info { ack } => {
                let _ = ack.send(self.info());
            }
            Cmd::Close { ack } => {
                self.shutdown();
                let _ = ack.send(());
            }
        }
    }

    fn info(&self) -> Info {
        let now = self.now();
        let mut sessions: Vec<SessInfo> = self
            .sessions
            .values()
            .map(|s| SessInfo {
                peer: s.peer.clone(),
                router: s.role == Role::Router,
                dialer: s.dialer.clone(),
                boot: s.boot,
                sno: s.sno,
                addr: s.addr.clone(),
                decls: s.decls.len(),
                depths: s.ob.depths(),
                dropped: s.ob.dropped.load(Ordering::Relaxed),
                full: s.ob.full.load(Ordering::Relaxed),
                age_ms: now.saturating_sub(s.since),
            })
            .collect();
        sessions.sort_by(|a, b| a.peer.cmp(&b.peer));
        Info { node: self.node.clone(), router: self.router, boot: self.boot, tokens: self.tok.len(), sessions }
    }

    fn on_sess(&mut self, e: SessEv) {
        match e {
            SessEv::Paused => {
                let gap = self.clock.gap_ms();
                self.fence(gap);
            }
            SessEv::Closed(uid, reason) => self.close(uid, reason),
            SessEv::Msg(uid, m) => {
                if self.sessions.contains_key(&uid) {
                    self.on_msg(uid, m);
                }
            }
        }
    }

    fn on_msg(&mut self, uid: u64, m: Msg) {
        let Some(role) = self.sessions.get(&uid).map(|s| s.role) else { return };
        match m {
            Msg::Close(r) => self.close(uid, r),
            Msg::Hello(_) | Msg::KeepAlive | Msg::DeclareDone => {}
            Msg::Declare { kind, id, expr } => self.remote_declare(uid, role, kind, id, expr),
            Msg::Undeclare { kind, id } => self.remote_undeclare(uid, kind, id),
            Msg::TokenSub { id, expr, history } => {
                if !self.router || role != Role::Client {
                    return;
                }
                let hist = if history { self.history(&expr) } else { Vec::new() };
                if let Some(s) = self.sessions.get_mut(&uid) {
                    for k in hist {
                        s.ob.push(&Msg::TokenEvt { id, kind: TokenEvtKind::Put, key: k });
                    }
                    if history {
                        s.ob.push(&Msg::TokenEvt { id, kind: TokenEvtKind::BatchEnd, key: String::new() });
                    }
                    s.tsubs.insert(id, expr);
                }
            }
            Msg::TokenUnsub { id } => {
                if let Some(s) = self.sessions.get_mut(&uid) {
                    s.tsubs.remove(&id);
                }
            }
            Msg::TokenEvt { id, kind, key } => {
                if self.router {
                    return;
                }
                let Some(t) = self.l_tsubs.get_mut(&id) else { return };
                let ev = match kind {
                    TokenEvtKind::Put => {
                        if !t.seen.insert(key.clone()) {
                            return;
                        }
                        TokenEvent::Put(key)
                    }
                    TokenEvtKind::Delete => {
                        t.seen.remove(&key);
                        TokenEvent::Delete(key)
                    }
                    TokenEvtKind::BatchEnd => TokenEvent::BatchEnd,
                };
                let _ = t.tx.send(ev);
            }
            Msg::Peers(list) => {
                for (n, a) in &list {
                    if *n != self.node {
                        self.known.insert(n.clone(), a.clone());
                    }
                }
                if let Some(s) = self.sessions.get_mut(&uid) {
                    s.peers = list;
                }
            }
            Msg::Put { key, payload } => self.route_put(Some(uid), &key, &payload),
            Msg::Request { qid, expr, payload, timeout_ms } => self.route_get(Origin::Sess { uid, qid }, &expr, payload, timeout_ms),
            Msg::Reply { qid, key, ok, payload } => {
                if self.pending.get(&qid).is_some_and(|p| p.targets.contains(&Target::Sess(uid))) {
                    self.forward_reply(qid, key, ok, payload);
                }
            }
            Msg::Final { qid } => self.target_done(qid, Target::Sess(uid)),
        }
    }

    fn remote_declare(&mut self, uid: u64, role: Role, kind: DeclKind, id: u64, expr: String) {
        if !self.router {
            return;
        }
        let Some(s) = self.sessions.get_mut(&uid) else { return };
        if s.decls.insert((kind, id), expr.clone()).is_some() {
            return;
        }
        match kind {
            DeclKind::Token => self.tok_add(&expr),
            DeclKind::Sub => self.subs.add(Owner::Sess(uid), id, &expr),
            DeclKind::Qabl => {}
        }
        if role == Role::Client {
            let rid = self.id();
            self.south.insert((uid, kind, id), rid);
            self.out.insert(rid, (kind, expr.clone()));
            self.announce(&Msg::Declare { kind, id: rid, expr });
        }
    }

    fn remote_undeclare(&mut self, uid: u64, kind: DeclKind, id: u64) {
        let Some(e) = self.sessions.get_mut(&uid).and_then(|s| s.decls.remove(&(kind, id))) else { return };
        match kind {
            DeclKind::Token => {
                self.tok_del(&e);
            }
            DeclKind::Sub => self.subs.remove(Owner::Sess(uid), id),
            DeclKind::Qabl => {}
        }
        if let Some(rid) = self.south.remove(&(uid, kind, id)) {
            self.out.remove(&rid);
            self.announce(&Msg::Undeclare { kind, id: rid });
        }
    }

    fn route_put(&mut self, from: Option<u64>, key: &str, payload: &[u8]) {
        let from_router = from.and_then(|u| self.sessions.get(&u)).is_some_and(|s| s.role == Role::Router);
        let mut sent: HashSet<u64> = HashSet::new();
        for (o, id) in self.subs.matching(key) {
            match o {
                Owner::Local(_) => {
                    if let Some((_, tx)) = self.l_subs.get(&id) {
                        let _ = tx.send(Sample { key: key.to_string(), payload: payload.to_vec() });
                    }
                }
                Owner::Sess(u) => {
                    if Some(u) == from || !sent.insert(u) {
                        continue;
                    }
                    let Some(s) = self.sessions.get(&u) else { continue };
                    if from_router && s.role == Role::Router {
                        continue;
                    }
                    s.ob.push(&Msg::Put { key: key.to_string(), payload: payload.to_vec() });
                }
            }
        }
        if let Some(u) = self.upstream()
            && from.is_none()
        {
            u.ob.push(&Msg::Put { key: key.to_string(), payload: payload.to_vec() });
        }
    }

    fn route_get(&mut self, origin: Origin, expr: &str, payload: Vec<u8>, timeout_ms: u32) {
        let from = match &origin {
            Origin::Sess { uid, .. } => Some(*uid),
            Origin::Local(_) => None,
        };
        let from_router = from.and_then(|u| self.sessions.get(&u)).is_some_and(|s| s.role == Role::Router);
        let qid = self.id();
        let mut targets: HashSet<Target> = HashSet::new();
        for (id, (e, _)) in &self.l_qabls {
            if intersects(e, expr) {
                targets.insert(Target::Local(*id));
            }
        }
        for (u, s) in &self.sessions {
            if Some(*u) == from {
                continue;
            }
            let hit = if self.router {
                !(from_router && s.role == Role::Router) && s.decls.iter().any(|((k, _), e)| *k == DeclKind::Qabl && intersects(e, expr))
            } else {
                from.is_none()
            };
            if hit {
                targets.insert(Target::Sess(*u));
            }
        }
        let deadline = self.now() + u64::from(timeout_ms);
        self.pending.insert(qid, Pending { origin, targets: targets.clone(), deadline });
        for t in targets {
            match t {
                Target::Local(id) => {
                    if let Some((_, tx)) = self.l_qabls.get(&id) {
                        let q = Query::new(qid, id, expr.to_string(), payload.clone(), self.cmd_tx.clone());
                        if tx.send(q).is_err() {
                            self.target_done(qid, t);
                        }
                    }
                }
                Target::Sess(u) => {
                    let pushed = self.sessions.get(&u).map(|s| s.ob.push(&Msg::Request { qid, expr: expr.to_string(), payload: payload.clone(), timeout_ms }));
                    if pushed != Some(Push::Queued) {
                        let peer = self.sessions.get(&u).map(|s| s.peer.clone()).unwrap_or_default();
                        self.forward_reply(qid, peer, false, b"mesh: request lane full".to_vec());
                        self.target_done(qid, t);
                    }
                }
            }
        }
        if self.pending.get(&qid).is_some_and(|p| p.targets.is_empty()) {
            self.finish(qid);
        }
    }

    fn forward_reply(&mut self, qid: u64, key: String, ok: bool, payload: Vec<u8>) {
        let Some(p) = self.pending.get(&qid) else { return };
        match &p.origin {
            Origin::Local(tx) => {
                let _ = tx.send(Reply { key, result: if ok { Ok(payload) } else { Err(payload) } });
            }
            Origin::Sess { uid, qid: theirs } => {
                if let Some(s) = self.sessions.get(uid) {
                    s.ob.push(&Msg::Reply { qid: *theirs, key, ok, payload });
                }
            }
        }
    }

    fn target_done(&mut self, qid: u64, t: Target) {
        let Some(p) = self.pending.get_mut(&qid) else { return };
        if p.targets.remove(&t) && p.targets.is_empty() {
            self.finish(qid);
        }
    }

    fn finish(&mut self, qid: u64) {
        let Some(p) = self.pending.remove(&qid) else { return };
        if let Origin::Sess { uid, qid: theirs } = p.origin
            && let Some(s) = self.sessions.get(&uid)
        {
            s.ob.push(&Msg::Final { qid: theirs });
        }
    }
}

fn advertise(listen: &str, seen: SocketAddr) -> String {
    match listen.parse::<SocketAddr>() {
        Ok(a) if a.ip().is_unspecified() => SocketAddr::new(seen.ip(), a.port()).to_string(),
        _ => listen.to_string(),
    }
}

async fn dial(addr: &str, hello: &Hello) -> Option<(TcpStream, Hello, SocketAddr)> {
    let mut s = tokio::time::timeout(HANDSHAKE, TcpStream::connect(addr)).await.ok()?.ok()?;
    let peer = s.peer_addr().ok()?;
    session::send_direct(&mut s, &Msg::Hello(hello.clone())).await.ok()?;
    match session::recv_direct(&mut s, HANDSHAKE).await? {
        Msg::Hello(h) => Some((s, h, peer)),
        _ => None,
    }
}

pub async fn accept_loop(l: tokio::net::TcpListener, tx: mpsc::UnboundedSender<Ev>) {
    while let Ok((mut s, addr)) = l.accept().await {
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Some(Msg::Hello(hello)) = session::recv_direct(&mut s, HANDSHAKE).await {
                let _ = tx.send(Ev::Accepted { stream: s, hello, addr });
            }
        });
    }
}
