// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-SURFACE · 정:RULE-OPS-OBSERVE · 정:RULE-OPS-DROPS · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:RULE-GC-FOUR · 정:IMPL-SES-DATA · 정:RULE-BUS-EVENTS · model: claude-opus-5-5
//! hub 의 조율 — 한 요청이 세션 · 방 · 버스를 차례로 건드리는 길(op 길 · 세션 폐기 · 통지 배달)은 가로지르는 이 영역이 든다.

pub mod admin;
pub mod http;
pub mod query;
pub mod route;
pub mod sup;
pub mod ws;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use common::bus::env::{Call, Notice};
use common::bus::keys::KeySpace;
use common::bus::place::Candidates;
use oxsig::code::Code;
use oxsig::op::Op;
use tokio::sync::{Notify, OnceCell, mpsc};
use tokio::task::JoinHandle;
use zenoh::liveliness::LivelinessToken;

use crate::aux::Probes;
use crate::bus::Bus;
use crate::cfg::HubCfg;
use crate::room::{Interest, RoomMap};
use crate::ses::{OutQueue, SessionTable, core::Push};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoneBy {
    Window,
    ClientLeave,
    Server,
}

#[derive(Debug)]
pub enum Out {
    Frame(Vec<u8>),
    Leave(Code),
    Close,
}

#[derive(Clone)]
pub struct SockTx {
    pub out: mpsc::UnboundedSender<Out>,
    pub q: Arc<Mutex<OutQueue>>,
    pub wake: Arc<Notify>,
}

#[derive(Default)]
pub struct Drops {
    pub notice_no_socket: AtomicU64,
    pub bus_ev_drop: AtomicU64,
    pub codes: Mutex<BTreeMap<u16, u64>>,
}

impl Drops {
    pub fn code(&self, c: u16) {
        if let Ok(mut m) = self.codes.lock() {
            m.entry(c).and_modify(|v| *v += 1).or_insert(1);
        }
    }
}

pub struct Hub {
    pub cfg: HubCfg,
    pub epoch: String,
    pub ks: KeySpace,
    pub bus: OnceCell<Arc<Bus>>,
    pub sessions: Mutex<SessionTable>,
    pub socks: Mutex<HashMap<u64, SockTx>>,
    pub next_sock: AtomicU64,
    pub rooms: Mutex<RoomMap>,
    pub cands: Mutex<Candidates>,
    pub interest: Mutex<Interest>,
    pub joining: Mutex<HashMap<String, BTreeSet<String>>>,
    pub room_subs: tokio::sync::Mutex<HashMap<String, JoinHandle<()>>>,
    pub user_subs: tokio::sync::Mutex<HashMap<String, (u32, JoinHandle<()>)>>,
    pub sess_tokens: tokio::sync::Mutex<HashMap<String, LivelinessToken>>,
    pub node_token: tokio::sync::Mutex<Option<LivelinessToken>>,
    pub sup: sup::Supervisor,
    pub drops: Drops,
    pub admission: Mutex<(u64, u32)>,
    pub probes: Probes,
    pub shutting: AtomicBool,
    pub fault: Mutex<Option<String>>,
    pub shutdown: Notify,
}

pub type HubRef = Arc<Hub>;

impl Hub {
    pub fn new(cfg: HubCfg) -> HubRef {
        let epoch = common::build::new_epoch();
        let ks = KeySpace::new(&cfg.system.zenoh.namespace);
        let me = cfg.node_id.clone();
        let sup = sup::Supervisor::new(&cfg);
        Arc::new(Hub {
            epoch,
            ks,
            bus: OnceCell::new(),
            sessions: Mutex::new(SessionTable::default()),
            socks: Mutex::new(HashMap::new()),
            next_sock: AtomicU64::new(1),
            rooms: Mutex::new(RoomMap::new(&me)),
            cands: Mutex::new(Candidates::new(&me)),
            interest: Mutex::new(Interest::default()),
            joining: Mutex::new(HashMap::new()),
            room_subs: tokio::sync::Mutex::new(HashMap::new()),
            user_subs: tokio::sync::Mutex::new(HashMap::new()),
            sess_tokens: tokio::sync::Mutex::new(HashMap::new()),
            node_token: tokio::sync::Mutex::new(None),
            sup,
            drops: Drops::default(),
            admission: Mutex::new((0, 0)),
            probes: Probes::default(),
            shutting: AtomicBool::new(false),
            fault: Mutex::new(None),
            shutdown: Notify::new(),
            cfg,
        })
    }

    pub fn me(&self) -> &str {
        &self.cfg.node_id
    }

    pub fn bus(&self) -> Option<Arc<Bus>> {
        self.bus.get().cloned()
    }

    pub fn grace_ms(&self) -> u64 {
        self.cfg.system.supervisor.node_grace_secs * 1000
    }

    pub fn node_live(&self, node: &str) -> bool {
        if node == self.me() {
            return self.sup.node_declared();
        }
        self.cands.lock().map(|c| c.live(node)).unwrap_or(false)
    }

    pub fn sock(&self, id: u64) -> Option<SockTx> {
        self.socks.lock().ok().and_then(|s| s.get(&id).cloned())
    }

    pub fn push_frame(&self, sock: u64, op: u16, body: &serde_json::Value) {
        let Some(tx) = self.sock(sock) else {
            self.drops.notice_no_socket.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let lane = Op::from_u16(op).map(|o| o.lane()).unwrap_or(oxsig::op::Lane::Session);
        let bytes = oxsig::frame::to_body(body);
        let r = tx.q.lock().map(|mut q| q.push(lane, op, bytes)).unwrap_or(Push::Overflow);
        match r {
            Push::Queued => tx.wake.notify_one(),
            Push::Overflow => {
                let _ = tx.out.send(Out::Leave(Code::FLOW_OVERFLOW));
            }
        }
    }

    pub fn send_leave(&self, sock: u64, code: Code) {
        if let Some(tx) = self.sock(sock) {
            let _ = tx.out.send(Out::Leave(code));
        }
    }

    pub fn deliver_room(&self, room: &str, n: &Notice) {
        let mut targets: Vec<(String, Option<u64>, String)> = Vec::new();
        if let Ok(t) = self.sessions.lock() {
            for s in t.all() {
                let member = s.data.servers.values().any(|r| r.rooms.contains(room));
                let joining = self.joining.lock().map(|j| j.get(&s.data.sid).is_some_and(|x| x.contains(room))).unwrap_or(false);
                if member || joining {
                    targets.push((s.data.sid.clone(), s.sock, s.data.user_id.clone()));
                }
            }
        }
        for (_, sock, user) in targets {
            if n.exclude.contains(&user) {
                continue;
            }
            match sock {
                Some(k) => self.push_frame(k, n.op, &n.body),
                None => {
                    self.drops.notice_no_socket.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn deliver_user(self: &Arc<Self>, user: &str, n: &Notice) {
        if n.exclude.iter().any(|u| u == user) {
            return;
        }
        let target = self.sessions.lock().ok().and_then(|t| t.of_user(user).map(|s| (s.data.sid.clone(), s.sock)));
        let Some((sid, sock)) = target else { return };
        if n.op == Op::RoomEvent.code()
            && n.body.get("type").and_then(|v| v.as_str()) == Some("affiliation")
            && let (Some(sfu), Some(sub)) = (n.sfu_id.clone(), n.body.pointer("/affiliation/sub_rooms").and_then(|v| v.as_array()))
        {
            let rooms: BTreeSet<String> = sub.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            let node = self.node_of_sfu(&sfu).unwrap_or_else(|| self.me().to_string());
            self.set_server_rooms(&sid, &sfu, &node, rooms);
        }
        match sock {
            Some(k) => self.push_frame(k, n.op, &n.body),
            None => {
                self.drops.notice_no_socket.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn node_of_sfu(&self, sfu_id: &str) -> Option<String> {
        if self.sup.unit_by_epoch(sfu_id).is_some() {
            return Some(self.me().to_string());
        }
        self.rooms.lock().ok().and_then(|r| r.node_of_inst(sfu_id))
    }

    pub fn set_server_rooms(self: &Arc<Self>, sid: &str, sfu_id: &str, node: &str, rooms: BTreeSet<String>) {
        let (before, after) = {
            let Ok(mut t) = self.sessions.lock() else { return };
            let Some(s) = t.get_mut(sid) else { return };
            let before = s.data.all_rooms();
            s.data.set_server_rooms(sfu_id, node, rooms);
            (before, s.data.all_rooms())
        };
        let added: Vec<String> = after.difference(&before).cloned().collect();
        let removed: Vec<String> = before.difference(&after).cloned().collect();
        let mut first = Vec::new();
        let mut last = Vec::new();
        if let Ok(mut i) = self.interest.lock() {
            for r in &added {
                if i.inc(r) {
                    first.push(r.clone());
                }
            }
            for r in &removed {
                if i.dec(r) {
                    last.push(r.clone());
                }
            }
        }
        if !first.is_empty() || !last.is_empty() {
            let me = self.clone();
            tokio::spawn(async move {
                for r in first {
                    me.declare_room_interest(&r).await;
                }
                for r in last {
                    me.drop_room_interest(&r).await;
                }
            });
        }
    }

    pub async fn interest_inc(self: &Arc<Self>, room: &str) {
        let first = self.interest.lock().map(|mut i| i.inc(room)).unwrap_or(false);
        if first {
            self.declare_room_interest(room).await;
        }
    }

    pub async fn interest_dec(self: &Arc<Self>, room: &str) {
        let last = self.interest.lock().map(|mut i| i.dec(room)).unwrap_or(false);
        if last {
            self.drop_room_interest(room).await;
        }
    }

    pub async fn declare_room_interest(self: &Arc<Self>, room: &str) {
        let Some(bus) = self.bus() else { return };
        if self.room_subs.lock().await.contains_key(room) {
            return;
        }
        let key = self.ks.ev_room(room);
        let Ok(sub) = bus.subscribe(&key).await else { return };
        let me = self.clone();
        let r = room.to_string();
        let h = tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                match Notice::decode(&s.payload().to_bytes()) {
                    Ok(n) => me.deliver_room(&r, &n),
                    Err(_) => {
                        me.drops.bus_ev_drop.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        let mut subs = self.room_subs.lock().await;
        if let Some(old) = subs.insert(room.to_string(), h) {
            old.abort();
        }
    }

    pub async fn drop_room_interest(&self, room: &str) {
        let still = self.interest.lock().map(|i| i.held(room)).unwrap_or(false);
        if still {
            return;
        }
        if let Some(h) = self.room_subs.lock().await.remove(room) {
            h.abort();
        }
    }

    pub async fn user_interest(self: &Arc<Self>, user: &str) {
        let Some(bus) = self.bus() else { return };
        let mut subs = self.user_subs.lock().await;
        if let Some(e) = subs.get_mut(user) {
            e.0 += 1;
            return;
        }
        let Ok(sub) = bus.subscribe(&self.ks.ev_user(user)).await else { return };
        let me = self.clone();
        let u = user.to_string();
        let h = tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                match Notice::decode(&s.payload().to_bytes()) {
                    Ok(n) => me.deliver_user(&u, &n),
                    Err(_) => {
                        me.drops.bus_ev_drop.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        subs.insert(user.to_string(), (1, h));
    }

    pub async fn user_interest_drop(&self, user: &str) {
        let mut subs = self.user_subs.lock().await;
        let gone = match subs.get_mut(user) {
            Some(e) if e.0 > 1 => {
                e.0 -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if gone && let Some((_, h)) = subs.remove(user) {
            h.abort();
        }
    }

    pub async fn discard_session(self: &Arc<Self>, sid: &str, code: Code, by: GoneBy) {
        let s = self.sessions.lock().ok().and_then(|mut t| t.remove(sid));
        let Some(s) = s else { return };
        if let Some(sock) = s.sock
            && matches!(code, Code::SESSION_REVOKED | Code::DUPLICATE_SESSION | Code::SERVER_SHUTDOWN)
        {
            self.send_leave(sock, code);
        }
        for r in s.data.all_rooms() {
            self.interest_dec(&r).await;
        }
        if let Ok(mut j) = self.joining.lock() {
            j.remove(sid);
        }
        self.sess_tokens.lock().await.remove(sid);
        self.user_interest_drop(&s.data.user_id).await;
        let fp = s.data.fingerprint();
        let mut targets: BTreeMap<String, String> = BTreeMap::new();
        for (sfu, r) in &s.data.servers {
            targets.insert(sfu.clone(), r.node.clone());
        }
        for (sfu, node) in targets {
            let call = Call::SessionGone { user_id: s.data.user_id.clone(), session: fp.clone(), code: code.0 };
            let _ = route::call(self, &route::Target { node, inst: Some(sfu) }, &call).await;
        }
        match by {
            GoneBy::Window => common::ev!(spec = "SES.S.TWindow", area = "SES", key = %fp, from = "DETACHED", to = "-", cause = "TWindow", session = %fp, user = %s.data.user_id, "세션 폐기"),
            GoneBy::ClientLeave => common::ev!(spec = "SES.S.leave", area = "SES", key = %fp, from = "BOUND", to = "-", cause = "LEAVE", session = %fp, user = %s.data.user_id, "세션 폐기"),
            GoneBy::Server => {
                let spec = match code {
                    Code::SESSION_REVOKED => "CP-CODE-SESSION-REVOKED",
                    Code::DUPLICATE_SESSION => "CP-CODE-DUPLICATE-SESSION",
                    Code::SERVER_SHUTDOWN => "CP-CODE-SERVER-SHUTDOWN",
                    _ => "CP-OP-LEAVE",
                };
                common::ev!(spec = spec, session = %fp, user = %s.data.user_id, code = code.0, "세션 폐기");
            }
        }
    }

    pub fn admit_join(&self, now_ms: u64) -> Result<(), u64> {
        let cap = self.cfg.policy.hub.join_admit_per_sec;
        if cap == 0 {
            return Ok(());
        }
        let sec = now_ms / 1000;
        let Ok(mut a) = self.admission.lock() else { return Ok(()) };
        if a.0 != sec {
            a.0 = sec;
            a.1 = 0;
        }
        if a.1 >= cap {
            let base = u64::from(self.cfg.policy.hub.join_retry_after_ms);
            let mut b = [0u8; 8];
            let _ = getrandom::fill(&mut b);
            return Err(base + u64::from_le_bytes(b) % base.max(1));
        }
        a.1 += 1;
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    common::time::mono_ms()
}

pub async fn fault_shutdown(hub: &HubRef, why: &str) {
    if let Ok(mut f) = hub.fault.lock() {
        f.get_or_insert_with(|| why.to_string());
    }
    begin_shutdown(hub, why).await;
}

pub async fn begin_shutdown(hub: &HubRef, why: &str) {
    if hub.shutting.swap(true, Ordering::Relaxed) {
        return;
    }
    common::ev!(spec = "CP-CODE-SERVER-SHUTDOWN", why, "종료 시작");
    let socks: Vec<u64> = hub.socks.lock().map(|s| s.keys().copied().collect()).unwrap_or_default();
    for s in socks {
        hub.send_leave(s, Code::SERVER_SHUTDOWN);
    }
    tokio::time::sleep(std::time::Duration::from_millis(common::bus::SHUTDOWN_LEAVE_WAIT_MS)).await;
    sup::stop_all(hub, false).await;
    hub.node_token.lock().await.take();
    hub.sess_tokens.lock().await.clear();
    if let Some(b) = hub.bus() {
        let _ = b.z.close().await;
    }
    sup::stop_all(hub, true).await;
    hub.shutdown.notify_waiters();
    hub.shutdown.notify_one();
}

pub async fn run(hub: HubRef) -> Result<(), String> {
    sup::start_all(&hub);
    let h = hub.clone();
    let server = tokio::spawn(async move { http::serve(h).await });
    let h = hub.clone();
    tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let int = tokio::signal::ctrl_c();
        tokio::select! {
            _ = async { match term.as_mut() { Some(t) => { t.recv().await; } None => std::future::pending::<()>().await } } => {}
            _ = int => {}
        }
        begin_shutdown(&h, "signal").await;
    });
    tokio::select! {
        r = server => {
            if let Ok(Err(e)) = r {
                return Err(e);
            }
        }
        _ = hub.shutdown.notified() => {}
    }
    match hub.fault.lock().ok().and_then(|f| f.clone()) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}
