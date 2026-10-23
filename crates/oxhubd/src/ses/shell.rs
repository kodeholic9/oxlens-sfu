// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SES-DATA · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-SES-RESUME · 정:RULE-SES-READ-AND-PROCESS · 정:IMPL-SES-SOCKET-LIFE · 정:IMPL-SES-ADMISSION · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:IMPL-OPS-CUT · 정:IMPL-ROOM-JOIN · 연:RULE-SES-AUTH-TWO-WAYS · 연:RULE-SES-EVICT-OLD · 연:RULE-SES-LEAVE-PROC · 연:RULE-TRN-CLOSE · 연:DEV-TRN-WS-ORIGIN · model: claude-opus-5-5
//! SES I/O — WS 소켓마다 읽기 · 처리 · 쓰기를 가른다 · 세션 표(창 · 넘겨받기 · 축출 · 절단) · 방 요청을 미디어 서버로 차례대로 넘긴다 · 통지 배달.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use common::bus::Notice;
use common::handle::{Call, Envelope, Reply};
use common::policy::Policy;
use common::system::SystemFile;
use futures_util::{SinkExt, StreamExt};
use oxsig::err::{ErrCode, Failure, LeaveBody};
use oxsig::op;
use oxsig::room::Permission;
use oxsig::ses::{BindReq, BindRes, PcMode, ResumeRes, CLIENT_VER, SERVER_VER};
use oxsig::trn::{Kind, WsFrame};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::core::{self, Out, Outbox};
use super::token::{self, Verified};
use crate::bus::{Bus, Delivery};

#[derive(Debug)]
pub enum Wcmd {
    Out { lane: u8, out: Out },
    Direct(Vec<u8>),
    Ack(u32),
    Leave(ErrCode),
    Close,
}

#[derive(Clone)]
pub struct Sock {
    pub id: u64,
    pub tx: mpsc::UnboundedSender<Wcmd>,
}

impl Sock {
    pub fn notice(&self, op_code: u16, body: &Value) {
        let b = serde_json::to_vec(body).unwrap_or_default();
        let _ = self.tx.send(Wcmd::Out { lane: op::lane(op_code), out: Out::Notice { op: op_code, body: b } });
    }

    pub fn leave(&self, code: ErrCode) {
        let _ = self.tx.send(Wcmd::Leave(code));
    }
}

pub struct Session {
    pub sid: String,
    pub user_id: String,
    pub v: Verified,
    pub pc_mode: PcMode,
    pub sock: Mutex<Option<Sock>>,
    pub disc_at: AtomicU64,
    pub resumed: AtomicBool,
    pub rooms: Mutex<BTreeMap<String, BTreeSet<String>>>,
    pub nodes: Mutex<BTreeSet<String>>,
    pub gone: AtomicBool,
}

#[derive(Serialize, Deserialize)]
struct Moved {
    sid: String,
    user_id: String,
    participant_type: u8,
    hidden: bool,
    metadata: Option<Value>,
    permission: Permission,
    pc_mode: PcMode,
    rooms: BTreeMap<String, BTreeSet<String>>,
    nodes: BTreeSet<String>,
}

impl Session {
    fn sock(&self) -> Option<Sock> {
        self.sock.lock().ok().and_then(|g| g.clone())
    }

    fn all_rooms(&self) -> BTreeSet<String> {
        self.rooms.lock().map(|m| m.values().flatten().cloned().collect()).unwrap_or_default()
    }

    fn in_room(&self, room: &str) -> bool {
        self.rooms.lock().map(|m| m.values().any(|s| s.contains(room))).unwrap_or(false)
    }
}

pub struct Ses {
    pub bus: Arc<Bus>,
    pub sys: Arc<SystemFile>,
    pub policy: Arc<Policy>,
    pub node: String,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    by_user: Mutex<HashMap<String, String>>,
    sock_seq: AtomicU64,
    shutting: AtomicBool,
    admit: Mutex<(u64, u32)>,
}

enum Res {
    Reply(Result<Value, Failure>),
    Empty,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn now() -> u64 {
    common::clock::wall_ms()
}

fn fail(c: ErrCode) -> Failure {
    Failure::new(c)
}

impl Ses {
    pub fn new(bus: Arc<Bus>, sys: Arc<SystemFile>, policy: Arc<Policy>, node: &str) -> Arc<Ses> {
        Arc::new(Ses {
            bus,
            sys,
            policy,
            node: node.to_string(),
            sessions: Mutex::new(HashMap::new()),
            by_user: Mutex::new(HashMap::new()),
            sock_seq: AtomicU64::new(1),
            shutting: AtomicBool::new(false),
            admit: Mutex::new((0, 0)),
        })
    }

    pub fn count(&self) -> usize {
        lock(&self.sessions).len()
    }

    pub fn session_of(&self, sid: &str) -> Option<Arc<Session>> {
        lock(&self.sessions).get(sid).cloned()
    }

    pub fn by_user(&self, user: &str) -> Option<Arc<Session>> {
        let sid = lock(&self.by_user).get(user).cloned()?;
        self.session_of(&sid)
    }

    pub fn list(&self) -> Vec<Arc<Session>> {
        lock(&self.sessions).values().cloned().collect()
    }

    fn envelope(&self, s: &Session) -> Envelope {
        Envelope {
            session_id: s.sid.clone(),
            user_id: s.user_id.clone(),
            pc_mode: s.pc_mode,
            hidden: s.v.hidden,
            participant_type: s.v.participant_type,
            metadata: s.v.metadata.clone(),
            permission: s.v.permission,
            home_node: self.node.clone(),
            home_inst: self.bus.node_inst().unwrap_or_default(),
        }
    }

    pub fn deliver(&self, d: Delivery) {
        match d {
            Delivery::Room { room, notice } => {
                let targets: Vec<Arc<Session>> = lock(&self.sessions).values().filter(|s| s.in_room(&room) && !notice.exclude.contains(&s.user_id)).cloned().collect();
                for s in targets {
                    if let Some(k) = s.sock() {
                        k.notice(notice.op, &notice.body);
                    }
                }
            }
            Delivery::User { user, notice } => {
                let Some(s) = self.by_user(&user) else { return };
                self.on_user_notice(&s, &notice);
                if let Some(k) = s.sock() {
                    k.notice(notice.op, &notice.body);
                }
            }
        }
    }

    fn on_user_notice(&self, s: &Arc<Session>, n: &Notice) {
        if n.op != op::ROOM_EVENT || n.body.get("type").and_then(Value::as_str) != Some("affiliation") {
            return;
        }
        let Some(room) = n.body.get("room_id").and_then(Value::as_str) else { return };
        let node = self.bus.room_of(room).map(|p| p.node);
        let subs: Option<BTreeSet<String>> = n.body.pointer("/affiliation/sub_rooms").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
        let mut changed: Vec<String> = Vec::new();
        {
            let mut m = lock(&s.rooms);
            match (node, subs) {
                (Some(node), Some(set)) => {
                    let old = m.get(&node).cloned().unwrap_or_default();
                    changed.extend(old.difference(&set).cloned());
                    m.insert(node, set);
                }
                _ => {
                    for set in m.values_mut() {
                        if set.remove(room) {
                            changed.push(room.to_string());
                        }
                    }
                }
            }
        }
        for r in changed {
            self.bus.uninterest_room(&r);
        }
    }

    async fn set_rooms(&self, s: &Arc<Session>, node: &str, new: BTreeSet<String>, tentative: Option<&str>) {
        let old = lock(&s.rooms).get(node).cloned().unwrap_or_default();
        for r in new.difference(&old) {
            if Some(r.as_str()) != tentative {
                self.bus.interest_room(r).await;
            }
        }
        for r in old.difference(&new) {
            self.bus.uninterest_room(r);
        }
        if let Some(t) = tentative {
            if !new.contains(t) && !old.contains(t) {
                self.bus.uninterest_room(t);
            }
            if old.contains(t) {
                self.bus.uninterest_room(t);
            }
        }
        lock(&s.rooms).insert(node.to_string(), new);
    }

    async fn discard(&self, s: &Arc<Session>, path: &str) {
        if s.gone.swap(true, Ordering::SeqCst) {
            return;
        }
        lock(&self.sessions).remove(&s.sid);
        {
            let mut bu = lock(&self.by_user);
            if bu.get(&s.user_id) == Some(&s.sid) {
                bu.remove(&s.user_id);
                self.bus.uninterest_user(&s.user_id);
            }
        }
        self.bus.undeclare_sess(&s.sid);
        for r in s.all_rooms() {
            self.bus.uninterest_room(&r);
        }
        let nodes: Vec<String> = lock(&s.nodes).iter().cloned().collect();
        let call = Call::SessionGone { session_id: s.sid.clone(), user_id: s.user_id.clone(), path: path.to_string() };
        for n in nodes {
            let _ = self.bus.call(&n, &call).await;
        }
        tracing::info!(kind = "diag", session = %s.sid, user = %s.user_id, path = %path, "session discarded");
    }

    pub async fn cut_user(&self, user: &str) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(s) = self.by_user(user) {
            if let Some(k) = s.sock() {
                k.leave(ErrCode::SessionRevoked);
            }
            self.discard(&s, "cut").await;
            out.push(json!({"session_id": s.sid, "code": ErrCode::SessionRevoked.code()}));
        }
        out
    }

    pub async fn shutdown(&self) {
        self.shutting.store(true, Ordering::SeqCst);
        let all = self.list();
        for s in &all {
            if let Some(k) = s.sock() {
                k.leave(ErrCode::ServerShutdown);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        for s in all {
            self.discard(&s, "shutdown").await;
        }
    }

    pub fn spawn_sweeper(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_millis(1000));
            loop {
                t.tick().await;
                let window = me.policy.hub.resume_window_ms as u64;
                let n = now();
                let expired: Vec<Arc<Session>> = me
                    .list()
                    .into_iter()
                    .filter(|s| s.sock().is_none() && s.disc_at.load(Ordering::Relaxed) != 0 && n.saturating_sub(s.disc_at.load(Ordering::Relaxed)) >= window)
                    .collect();
                for s in expired {
                    me.discard(&s, "window").await;
                }
            }
        });
    }

    pub async fn serve(&self, what: &str, payload: &[u8]) -> Vec<u8> {
        match what {
            "sess" => {
                let sid = String::from_utf8_lossy(payload).to_string();
                let Some(s) = self.session_of(&sid) else { return Vec::new() };
                if s.gone.swap(true, Ordering::SeqCst) {
                    return Vec::new();
                }
                if let Some(k) = s.sock() {
                    k.leave(ErrCode::DuplicateSession);
                }
                lock(&self.sessions).remove(&sid);
                {
                    let mut bu = lock(&self.by_user);
                    if bu.get(&s.user_id) == Some(&s.sid) {
                        bu.remove(&s.user_id);
                        self.bus.uninterest_user(&s.user_id);
                    }
                }
                self.bus.undeclare_sess(&sid);
                for r in s.all_rooms() {
                    self.bus.uninterest_room(&r);
                }
                let m = Moved {
                    sid: s.sid.clone(),
                    user_id: s.user_id.clone(),
                    participant_type: s.v.participant_type,
                    hidden: s.v.hidden,
                    metadata: s.v.metadata.clone(),
                    permission: s.v.permission,
                    pc_mode: s.pc_mode,
                    rooms: lock(&s.rooms).clone(),
                    nodes: lock(&s.nodes).clone(),
                };
                serde_json::to_vec(&m).unwrap_or_default()
            }
            "evict" => {
                let user = String::from_utf8_lossy(payload).to_string();
                if let Some(s) = self.by_user(&user) {
                    if let Some(k) = s.sock() {
                        k.leave(ErrCode::DuplicateSession);
                    }
                    self.discard(&s, "evict").await;
                }
                b"{}".to_vec()
            }
            "cut" => {
                let user = String::from_utf8_lossy(payload).to_string();
                serde_json::to_vec(&self.cut_user(&user).await).unwrap_or_default()
            }
            "users" => serde_json::to_vec(&self.users_view()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub fn users_view(&self) -> Vec<Value> {
        self.list()
            .iter()
            .map(|s| {
                json!({
                    "session_id": s.sid,
                    "user_id": s.user_id,
                    "participant_type": s.v.participant_type,
                    "hidden": s.v.hidden,
                    "bound": s.sock().is_some(),
                    "nodes": lock(&s.nodes).iter().cloned().collect::<Vec<_>>(),
                    "rooms": lock(&s.rooms).clone(),
                })
            })
            .collect()
    }

    pub fn origin_ok(&self, origin: Option<&str>, host: Option<&str>) -> bool {
        let Some(o) = origin else { return true };
        let allowed = &self.policy.hub.allowed_origins;
        if allowed.iter().any(|a| a == "*" || a == o) {
            return true;
        }
        match host {
            Some(h) => o.ends_with(&format!("//{h}")),
            None => false,
        }
    }

    pub async fn run_socket(self: Arc<Self>, ws: WebSocket) {
        let id = self.sock_seq.fetch_add(1, Ordering::Relaxed);
        let (sink, mut stream) = ws.split();
        let (tx, rx) = mpsc::unbounded_channel::<Wcmd>();
        let sock = Sock { id, tx: tx.clone() };
        let window = self.policy.hub.ws_flow_window as usize;
        let alive = Arc::new(AtomicBool::new(true));
        let w_alive = alive.clone();
        tokio::spawn(async move {
            writer(sink, rx, window).await;
            w_alive.store(false, Ordering::SeqCst);
        });
        let bound: Arc<Mutex<Option<Arc<Session>>>> = Arc::new(Mutex::new(None));
        let (ptx, mut prx) = mpsc::unbounded_channel::<WsFrame>();
        let backlog = Arc::new(AtomicU64::new(0));
        let me = self.clone();
        let p_bound = bound.clone();
        let p_sock = sock.clone();
        let p_backlog = backlog.clone();
        tokio::spawn(async move {
            while let Some(f) = prx.recv().await {
                p_backlog.fetch_sub(1, Ordering::Relaxed);
                me.process(&p_sock, &p_bound, f).await;
            }
        });
        let mut last_rx = now();
        loop {
            if !alive.load(Ordering::SeqCst) {
                break;
            }
            let is_bound = lock(&bound).is_some();
            let limit = if is_bound { self.policy.hub.heartbeat_timeout_ms as u64 } else { core::BIND_TIMEOUT_MS };
            let left = (last_rx + limit).saturating_sub(now());
            let next = tokio::time::timeout(Duration::from_millis(left.max(1)), stream.next()).await;
            let msg = match next {
                Err(_) => {
                    sock.leave(ErrCode::HeartbeatTimeout);
                    break;
                }
                Ok(None) | Ok(Some(Err(_))) => break,
                Ok(Some(Ok(m))) => m,
            };
            last_rx = now();
            let data = match msg {
                Message::Binary(b) => b,
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Text(_) => {
                    sock.leave(ErrCode::ProtocolError);
                    break;
                }
            };
            let f = match WsFrame::decode(&data) {
                Ok(f) => f,
                Err(_) => {
                    sock.leave(ErrCode::ProtocolError);
                    break;
                }
            };
            match f.kind() {
                Some(Kind::Ok) | Some(Kind::Fail) => {
                    let _ = tx.send(Wcmd::Ack(f.pid));
                }
                Some(Kind::Req) => {
                    if f.op == op::HEARTBEAT {
                        let _ = tx.send(Wcmd::Direct(core::reply_frame(op::HEARTBEAT, f.pid, true, &[])));
                        continue;
                    }
                    if f.op == op::LEAVE {
                        let s = lock(&bound).take();
                        if let Some(s) = s {
                            self.discard(&s, "leave").await;
                        }
                        let _ = tx.send(Wcmd::Close);
                        break;
                    }
                    if f.body_json().is_err() {
                        let body = serde_json::to_vec(&fail(ErrCode::ProtocolError)).unwrap_or_default();
                        let _ = tx.send(Wcmd::Direct(core::reply_frame(f.op, f.pid, false, &body)));
                        sock.leave(ErrCode::ProtocolError);
                        break;
                    }
                    if backlog.fetch_add(1, Ordering::Relaxed) >= core::BACKLOG_MAX as u64 {
                        sock.leave(ErrCode::FlowOverflow);
                        break;
                    }
                    let _ = ptx.send(f);
                }
                None => {
                    sock.leave(ErrCode::ProtocolError);
                    break;
                }
            }
        }
        let s = lock(&bound).take();
        if let Some(s) = s {
            let mine = lock(&s.sock).as_ref().map(|k| k.id == id).unwrap_or(false);
            if mine {
                lock(&s.sock).take();
                s.disc_at.store(now(), Ordering::Relaxed);
                tracing::info!(kind = "diag", session = %s.sid, user = %s.user_id, "socket lost — window starts");
            }
        }
        let _ = tx.send(Wcmd::Close);
    }

    async fn process(&self, sock: &Sock, bound: &Arc<Mutex<Option<Arc<Session>>>>, f: WsFrame) {
        let body = f.body_json().unwrap_or(Value::Null);
        let cur = lock(bound).clone();
        let res = if f.op == op::BIND {
            self.bind(sock, bound, &body).await
        } else if !op::known(f.op) {
            Res::Reply(Err(fail(ErrCode::UnknownOp)))
        } else if let Some(s) = cur {
            if let Some(missing) = op::required(f.op).iter().find(|k| body.get(**k).is_none()) {
                Res::Reply(Err(fail(ErrCode::MissingField).msg(format!("{missing} 없음"))))
            } else {
                self.handle(&s, f.op, body).await
            }
        } else {
            Res::Reply(Err(fail(ErrCode::NotBound)))
        };
        let frame = match res {
            Res::Reply(Ok(v)) => core::reply_frame(f.op, f.pid, true, &serde_json::to_vec(&v).unwrap_or_default()),
            Res::Reply(Err(e)) => core::reply_frame(f.op, f.pid, false, &serde_json::to_vec(&e).unwrap_or_default()),
            Res::Empty => core::reply_frame(f.op, f.pid, true, &[]),
        };
        let _ = sock.tx.send(Wcmd::Out { lane: op::lane(f.op), out: Out::Reply { frame } });
    }

    async fn bind(&self, sock: &Sock, bound: &Arc<Mutex<Option<Arc<Session>>>>, body: &Value) -> Res {
        if lock(bound).is_some() {
            return Res::Reply(Err(fail(ErrCode::InvalidPayload).msg("이미 BIND 했다")));
        }
        let req: BindReq = match serde_json::from_value(body.clone()) {
            Ok(r) => r,
            Err(_) if body.get("token").is_none() => return Res::Reply(Err(fail(ErrCode::MissingField).msg("token 없음"))),
            Err(e) => return Res::Reply(Err(fail(ErrCode::InvalidPayload).msg(e.to_string()))),
        };
        let pc = match req.pc_mode.as_deref() {
            None => PcMode::Two,
            Some(m) => match PcMode::parse(m) {
                Some(p) => p,
                None => return Res::Reply(Err(fail(ErrCode::InvalidPayload).msg(format!("pc_mode {m}")))),
            },
        };
        if req.client_ver.unwrap_or(CLIENT_VER) != CLIENT_VER {
            return Res::Reply(Err(fail(ErrCode::VersionMismatch)));
        }
        if let Some(sid) = req.session_id.as_deref().filter(|s| !s.is_empty())
            && let Some(s) = self.attach_existing(sid).await {
                let old = lock(&s.sock).replace(sock.clone());
                if let Some(o) = old
                    && o.id != sock.id {
                        o.leave(ErrCode::DuplicateSession);
                    }
                s.disc_at.store(0, Ordering::Relaxed);
                s.resumed.store(true, Ordering::Relaxed);
                lock(bound).replace(s.clone());
                tracing::info!(kind = "ev", spec = "CP-OP-BIND", session = %s.sid, user = %s.user_id, path = "session_id", "bind resume");
                return Res::Reply(Ok(self.bind_res(&s)));
            }
        let v = match token::verify(&self.sys, &req.token) {
            Ok(v) => v,
            Err(c) => return Res::Reply(Err(fail(c))),
        };
        let q = self.policy.quota.max_sessions;
        if q != 0 && self.count() as u32 >= q {
            return Res::Reply(Err(fail(ErrCode::QuotaExceeded)));
        }
        self.evict_user(&v.user_id).await;
        let s = Arc::new(Session {
            sid: format!("s-{}", common::ident::random_alnum(24)),
            user_id: v.user_id.clone(),
            v,
            pc_mode: pc,
            sock: Mutex::new(Some(sock.clone())),
            disc_at: AtomicU64::new(0),
            resumed: AtomicBool::new(false),
            rooms: Mutex::new(BTreeMap::new()),
            nodes: Mutex::new(BTreeSet::new()),
            gone: AtomicBool::new(false),
        });
        self.install(&s).await;
        lock(bound).replace(s.clone());
        tracing::info!(kind = "ev", spec = "CP-OP-BIND", session = %s.sid, user = %s.user_id, path = "token", pc_mode = s.pc_mode.as_str(), "bind new");
        Res::Reply(Ok(self.bind_res(&s)))
    }

    async fn install(&self, s: &Arc<Session>) {
        self.bus.interest_user(&s.user_id).await;
        self.bus.declare_sess(&s.user_id, &s.sid).await;
        lock(&self.sessions).insert(s.sid.clone(), s.clone());
        lock(&self.by_user).insert(s.user_id.clone(), s.sid.clone());
    }

    fn bind_res(&self, s: &Session) -> Value {
        serde_json::to_value(BindRes {
            user_id: s.user_id.clone(),
            server_ver: SERVER_VER,
            heartbeat_interval: self.policy.hub.heartbeat_interval_ms as u64,
            session_id: s.sid.clone(),
            resume_window_ms: self.policy.hub.resume_window_ms as u64,
            pc_mode: s.pc_mode,
        })
        .unwrap_or(Value::Null)
    }

    async fn attach_existing(&self, sid: &str) -> Option<Arc<Session>> {
        if let Some(s) = self.session_of(sid)
            && !s.gone.load(Ordering::SeqCst) {
                return Some(s);
            }
        let found = self.bus.find_sess(self.bus.keys.sess_by_sid(sid)).await;
        let (_, node, _) = found.into_iter().find(|(_, n, _)| n != &self.node)?;
        let raw = self.bus.ask(&node, "sess", sid.as_bytes().to_vec()).await?;
        let m: Moved = serde_json::from_slice(&raw).ok()?;
        let s = Arc::new(Session {
            sid: m.sid,
            user_id: m.user_id.clone(),
            v: Verified { user_id: m.user_id, participant_type: m.participant_type, hidden: m.hidden, metadata: m.metadata, permission: m.permission },
            pc_mode: m.pc_mode,
            sock: Mutex::new(None),
            disc_at: AtomicU64::new(0),
            resumed: AtomicBool::new(true),
            rooms: Mutex::new(BTreeMap::new()),
            nodes: Mutex::new(m.nodes),
            gone: AtomicBool::new(false),
        });
        self.install(&s).await;
        for (node, set) in m.rooms {
            self.set_rooms(&s, &node, set, None).await;
        }
        tracing::info!(kind = "diag", session = %s.sid, from = %node, "session handed over");
        Some(s)
    }

    async fn evict_user(&self, user: &str) {
        if let Some(old) = self.by_user(user) {
            if let Some(k) = old.sock() {
                k.leave(ErrCode::DuplicateSession);
            }
            self.discard(&old, "evict").await;
        }
        let remote = self.bus.find_sess(self.bus.keys.sess_of_user(user)).await;
        for (_, node, _) in remote.into_iter().filter(|(_, n, _)| n != &self.node) {
            let _ = self.bus.ask(&node, "evict", user.as_bytes().to_vec()).await;
        }
    }

    fn admit_join(&self) -> Option<Failure> {
        let limit = self.policy.hub.join_admit_per_sec;
        if limit == 0 {
            return None;
        }
        let sec = now() / 1000;
        let mut g = lock(&self.admit);
        if g.0 != sec {
            g.clone_from(&(sec, 0));
        }
        g.1 += 1;
        if g.1 <= limit {
            return None;
        }
        let base = self.policy.hub.join_retry_after_ms as u64;
        let jitter = common::ident::random_u64() % base.max(1);
        Some(fail(ErrCode::Overloaded).details(json!({"retry_after_ms": base + jitter})))
    }

    async fn route(&self, s: &Arc<Session>, room: &str, op_code: u16, body: Value) -> (Option<String>, Result<Value, Failure>) {
        let Some(p) = self.bus.room_of(room) else { return (None, Err(fail(ErrCode::RoomNotFound))) };
        lock(&s.nodes).insert(p.node.clone());
        let r = self.bus.call(&p.node, &Call::Op { env: self.envelope(s), op: op_code, body }).await;
        (Some(p.node), reply_result(r))
    }

    async fn handle(&self, s: &Arc<Session>, op_code: u16, body: Value) -> Res {
        let room = body.get("room_id").and_then(Value::as_str).map(str::to_string);
        match op_code {
            op::RESUME => self.resume(s).await,
            op::ROOM_JOIN => {
                let Some(room) = room else { return Res::Reply(Err(fail(ErrCode::MissingField))) };
                if !oxsig::room::room_id_ok(&room) {
                    return Res::Reply(Err(fail(ErrCode::InvalidPayload).msg("room_id 형식")));
                }
                let tags = body.get("extmap").is_some() || body.get("codecs").is_some();
                match s.pc_mode {
                    PcMode::One if body.get("extmap").is_none() || body.get("codecs").is_none() => {
                        return Res::Reply(Err(fail(ErrCode::MissingField).msg("1pc 번호표 없음")));
                    }
                    PcMode::Two if tags => return Res::Reply(Err(fail(ErrCode::InvalidPayload).msg("2pc 에 번호표"))),
                    _ => {}
                }
                if self.bus.room_of(&room).is_none() {
                    return Res::Reply(Err(fail(ErrCode::RoomNotFound)));
                }
                if let Some(f) = self.admit_join() {
                    return Res::Reply(Err(f));
                }
                self.bus.interest_room(&room).await;
                let (node, r) = self.route(s, &room, op_code, body).await;
                match (&node, &r) {
                    (Some(n), Ok(v)) => {
                        let set = sub_rooms(v.get("affiliation"));
                        self.set_rooms(s, n, set, Some(&room)).await;
                    }
                    _ => {
                        if !s.in_room(&room) {
                            self.bus.uninterest_room(&room);
                        }
                    }
                }
                Res::Reply(r)
            }
            op::ROOM_LEAVE => {
                let Some(room) = room else { return Res::Reply(Err(fail(ErrCode::MissingField))) };
                let (node, r) = self.route(s, &room, op_code, body).await;
                if let (Some(n), Ok(v)) = (&node, &r) {
                    self.set_rooms(s, n, sub_rooms(v.get("affiliation")), None).await;
                }
                Res::Reply(r)
            }
            op::AFFILIATION => {
                let target = body.get("pub_select").and_then(Value::as_str).or(body.get("pub_deselect").and_then(Value::as_str)).map(str::to_string);
                let Some(room) = target else {
                    return Res::Reply(Ok(json!({"sub_rooms": [], "pub_room": null, "cause": "user", "change_id": body.get("change_id")})));
                };
                let (node, r) = self.route(s, &room, op_code, body).await;
                if let (Some(n), Ok(v)) = (&node, &r) {
                    self.set_rooms(s, n, sub_rooms(Some(v)), None).await;
                }
                Res::Reply(r)
            }
            op::PUBLISH_TRACKS | op::READY | op::SUBSCRIBE_LAYER | op::TRACK_SET | op::MESSAGE => {
                let Some(room) = room else { return Res::Reply(Err(fail(ErrCode::MissingField))) };
                let (_, r) = self.route(s, &room, op_code, body).await;
                match (op_code, r) {
                    (op::READY | op::SUBSCRIBE_LAYER, Ok(_)) => Res::Empty,
                    (_, r) => Res::Reply(r),
                }
            }
            op::ICE_RESTART => {
                let sfu = body.get("sfu_id").and_then(Value::as_str).unwrap_or("").to_string();
                let Some(node) = self.bus.node_of_inst(&sfu) else { return Res::Reply(Err(fail(ErrCode::PeerNotFound))) };
                lock(&s.nodes).insert(node.clone());
                let r = self.bus.call(&node, &Call::Op { env: self.envelope(s), op: op_code, body }).await;
                Res::Reply(reply_result(r))
            }
            op::TASK => task_reply(crate::aux::task(&body)),
            op::HEARTBEAT => Res::Empty,
            _ => Res::Reply(Err(fail(ErrCode::UnknownOp))),
        }
    }

    async fn resume(&self, s: &Arc<Session>) -> Res {
        if !s.resumed.load(Ordering::Relaxed) {
            return Res::Reply(Err(fail(ErrCode::SessionNotFound)));
        }
        let nodes: Vec<String> = lock(&s.nodes).iter().cloned().collect();
        let mut out = ResumeRes::default();
        for n in nodes {
            let r = self.bus.call(&n, &Call::Resume { env: self.envelope(s) }).await;
            let Reply::Ok { body } = r else { continue };
            let Ok(part) = serde_json::from_value::<ResumeRes>(body) else { continue };
            let set: BTreeSet<String> = part.snapshot.keys().cloned().collect();
            self.set_rooms(s, &n, set, None).await;
            out.snapshot.extend(part.snapshot);
            out.publications.extend(part.publications);
            out.mline_tables.extend(part.mline_tables);
        }
        tracing::info!(kind = "ev", spec = "CP-OP-RESUME", session = %s.sid, user = %s.user_id, rooms = out.snapshot.len(), "resume");
        Res::Reply(Ok(serde_json::to_value(out).unwrap_or(Value::Null)))
    }

    pub fn identify_header(&self, session: &str) -> Option<(String, String)> {
        let s = self.session_of(session)?;
        Some((s.user_id.clone(), s.sid.clone()))
    }
}

fn sub_rooms(v: Option<&Value>) -> BTreeSet<String> {
    v.and_then(|a| a.get("sub_rooms")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

pub fn reply_result(r: Reply) -> Result<Value, Failure> {
    match r {
        Reply::Ok { body } => Ok(body),
        Reply::Fail { failure } => Err(failure),
        Reply::Http { status, body } => Err(fail(ErrCode::InternalError).details(json!({"status": status, "body": body}))),
    }
}

fn task_reply(r: Result<Option<Value>, Failure>) -> Res {
    match r {
        Ok(None) => Res::Empty,
        Ok(Some(v)) => Res::Reply(Ok(v)),
        Err(f) => Res::Reply(Err(f)),
    }
}

async fn writer(mut sink: futures_util::stream::SplitSink<WebSocket, Message>, mut rx: mpsc::UnboundedReceiver<Wcmd>, window: usize) {
    let mut ob = Outbox::new(window);
    let mut tick = tokio::time::interval(Duration::from_millis(1000));
    let mut leave_pid: u32 = 0x8000_0000;
    loop {
        while let Some(f) = ob.next(now()) {
            if sink.send(Message::Binary(f)).await.is_err() {
                return;
            }
        }
        let cmd = tokio::select! {
            c = rx.recv() => c,
            _ = tick.tick() => {
                if ob.ack_overdue(now()) {
                    Some(Wcmd::Leave(ErrCode::FlowTimeout))
                } else if ob.queued() > core::BACKLOG_MAX {
                    Some(Wcmd::Leave(ErrCode::FlowOverflow))
                } else {
                    continue;
                }
            }
        };
        match cmd {
            None | Some(Wcmd::Close) => {
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
            Some(Wcmd::Out { lane, out }) => ob.push(lane, out),
            Some(Wcmd::Direct(f)) => {
                if sink.send(Message::Binary(f)).await.is_err() {
                    return;
                }
            }
            Some(Wcmd::Ack(pid)) => {
                ob.ack(pid);
            }
            Some(Wcmd::Leave(code)) => {
                leave_pid = leave_pid.wrapping_add(1);
                let body = serde_json::to_vec(&LeaveBody::new(code)).unwrap_or_default();
                let _ = sink.send(Message::Binary(WsFrame::new(Kind::Req, op::LEAVE, leave_pid, body).encode())).await;
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        }
    }
}
