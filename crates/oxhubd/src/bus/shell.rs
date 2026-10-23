// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-SHAPE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:IMPL-BUS-FAULTS · 정:RULE-BUS-FATE · model: claude-opus-5-5
//! BUS I/O — hub 의 zenoh router 세션 · 토큰 선언(node · sess) · 방 → node 맵(history 구독) · 관심 선언(ev/room · ev/user) · 요청 길(로컬 gRPC Handle · 원격 `q/node`).

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use common::bus::{Keys, Notice};
use common::handle::pb::handle_client::HandleClient;
use common::handle::pb::{CallReq, HelloReq};
use common::handle::{Call, Reply};
use oxsig::err::{ErrCode, Failure};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Subscriber;
use zenoh::handlers::FifoChannelHandler;
use zenoh::sample::{Sample, SampleKind};

use super::core::{self, Edge, Interest, NodeSet, Placed, RoomMap};

pub enum Delivery {
    Room { room: String, notice: Notice },
    User { user: String, notice: Notice },
}

pub type Sink = Arc<dyn Fn(Delivery) + Send + Sync>;
pub type BoxFut = Pin<Box<dyn Future<Output = Vec<u8>> + Send>>;
pub type Serve = Arc<dyn Fn(String, Vec<u8>) -> BoxFut + Send + Sync>;

type Sub = Subscriber<FifoChannelHandler<Sample>>;

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub session: zenoh::Session,
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    nodes: Mutex<NodeSet>,
    rooms: Mutex<RoomMap>,
    interest: Mutex<Interest>,
    room_subs: Mutex<HashMap<String, Sub>>,
    user_subs: Mutex<HashMap<String, Sub>>,
    sess_tokens: Mutex<HashMap<String, LivelinessToken>>,
    node_token: Mutex<Option<(String, LivelinessToken)>>,
    local: RwLock<Option<HandleClient<tonic::transport::Channel>>>,
    sink: OnceLock<Sink>,
    serve: OnceLock<Serve>,
    grace_ms: u64,
    last_event: Mutex<Option<(u64, String, String)>>,
    isolated: Mutex<bool>,
    burst: Mutex<(u64, usize, usize)>,
    keep: Mutex<Vec<Box<dyn std::any::Any + Send>>>,
}

fn now() -> u64 {
    common::clock::wall_ms()
}

impl Bus {
    pub async fn open(node: &str, ns: &str, listen: Vec<String>, connect: Vec<String>, multicast: bool, grace_ms: u64) -> Result<Arc<Bus>, String> {
        let conf = common::bus::BusConf { mode: "router", listen: &listen, connect: &connect, multicast_scouting: multicast };
        let session = common::bus::open(&conf).await?;
        let bus = Arc::new(Bus {
            keys: Keys::new(ns),
            node: node.to_string(),
            session,
            listen,
            connect,
            nodes: Mutex::new(NodeSet::default()),
            rooms: Mutex::new(RoomMap::default()),
            interest: Mutex::new(Interest::default()),
            room_subs: Mutex::new(HashMap::new()),
            user_subs: Mutex::new(HashMap::new()),
            sess_tokens: Mutex::new(HashMap::new()),
            node_token: Mutex::new(None),
            local: RwLock::new(None),
            sink: OnceLock::new(),
            serve: OnceLock::new(),
            grace_ms,
            last_event: Mutex::new(None),
            isolated: Mutex::new(false),
            burst: Mutex::new((0, 0, 0)),
            keep: Mutex::new(Vec::new()),
        });
        bus.watch_tokens().await?;
        bus.serve_queries().await?;
        Ok(bus)
    }

    pub fn set_sink(&self, s: Sink) {
        let _ = self.sink.set(s);
    }

    pub fn set_serve(&self, s: Serve) {
        let _ = self.serve.set(s);
    }

    fn note_event(&self, kind: &str, key: &str) {
        if let Ok(mut l) = self.last_event.lock() {
            l.replace((now(), kind.to_string(), key.to_string()));
        }
    }

    async fn watch_tokens(self: &Arc<Self>) -> Result<(), String> {
        let sub = self.session.liveliness().declare_subscriber(format!("{}/int/**", self.keys.ns)).history(true).await.map_err(|e| e.to_string())?;
        let me = self.clone();
        tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                let key = s.key_expr().as_str().to_string();
                let put = s.kind() == SampleKind::Put;
                me.note_event(if put { "put" } else { "delete" }, &key);
                let Some(parts) = me.keys.parse(&key) else { continue };
                match parts.as_slice() {
                    [k, node, inst] if k == "node" => me.on_node(node, inst, put),
                    [k, room, node, inst] if k == "room" => me.on_room(room, node, inst, put),
                    _ => {}
                }
            }
        });
        Ok(())
    }

    fn on_node(&self, node: &str, inst: &str, put: bool) {
        let mut ns = match self.nodes.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let remote_before = ns.live().keys().filter(|n| n.as_str() != self.node).count();
        if put {
            ns.put(node, inst);
            if node != self.node
                && let Ok(mut iso) = self.isolated.lock() {
                    iso.clone_from(&false);
                }
            tracing::info!(kind = "diag", node = %node, inst = %inst, "node token up");
            return;
        }
        ns.delete(node, inst, now());
        let remote_after = ns.live().keys().filter(|n| n.as_str() != self.node).count();
        drop(ns);
        let t = now();
        let mut b = match self.burst.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if t.saturating_sub(b.0) > 1_000 {
            b.clone_from(&(t, remote_before, 0));
        }
        b.2 += 1;
        let iso = core::isolated(b.1, remote_after, b.2);
        drop(b);
        if iso {
            if let Ok(mut i) = self.isolated.lock() {
                i.clone_from(&true);
            }
            tracing::warn!(kind = "diag", "remote node tokens vanished together — self isolation");
            return;
        }
        tracing::warn!(kind = "diag", node = %node, inst = %inst, "node token gone");
    }

    fn on_room(&self, room: &str, node: &str, inst: &str, put: bool) {
        let mut rm = match self.rooms.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if put {
            if rm.put(room, node, inst) {
                tracing::warn!(kind = "diag", room = %room, node = %node, duplicates = rm.duplicates, "room held by two nodes");
            }
        } else if !self.is_isolated() {
            rm.delete(room, node, inst);
        }
    }

    pub fn is_isolated(&self) -> bool {
        self.isolated.lock().map(|g| g.to_owned()).unwrap_or(false)
    }

    pub fn room_of(&self, room: &str) -> Option<Placed> {
        self.rooms.lock().ok().and_then(|m| m.get(room))
    }

    pub fn rooms(&self) -> Vec<(String, Placed)> {
        self.rooms.lock().map(|m| m.all()).unwrap_or_default()
    }

    pub fn rooms_of_node(&self, node: &str) -> Vec<String> {
        self.rooms.lock().map(|m| m.of_node(node)).unwrap_or_default()
    }

    pub fn duplicates(&self) -> u64 {
        self.rooms.lock().map(|m| m.duplicates).unwrap_or(0)
    }

    pub fn place(&self, room: &str) -> Option<String> {
        let mut ns = self.nodes.lock().ok()?;
        let cands = ns.candidates(now(), self.grace_ms);
        core::place(&cands, room)
    }

    pub fn node_live(&self, node: &str) -> bool {
        self.nodes.lock().map(|n| n.is_live(node)).unwrap_or(false)
    }

    pub fn live_nodes(&self) -> BTreeMap<String, String> {
        self.nodes.lock().map(|n| n.live().clone()).unwrap_or_default()
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<String> {
        self.nodes.lock().ok().and_then(|n| n.node_of_inst(inst).map(str::to_string))
    }

    pub async fn declare_node(&self, inst: &str) -> Result<(), String> {
        let key = self.keys.node_token(&self.node, inst);
        let tok = self.session.liveliness().declare_token(key.clone()).await.map_err(|e| e.to_string())?;
        if let Ok(mut g) = self.node_token.lock() {
            g.replace((inst.to_string(), tok));
        }
        tracing::info!(kind = "diag", key = %key, "node token declared");
        Ok(())
    }

    pub fn undeclare_node(&self) {
        if let Ok(mut g) = self.node_token.lock() {
            g.take();
        }
    }

    pub fn node_inst(&self) -> Option<String> {
        self.node_token.lock().ok().and_then(|g| g.as_ref().map(|(i, _)| i.clone()))
    }

    pub async fn declare_sess(&self, user: &str, sid: &str) {
        let key = self.keys.sess_token(user, &self.node, sid);
        if let Ok(tok) = self.session.liveliness().declare_token(key).await
            && let Ok(mut m) = self.sess_tokens.lock() {
                m.insert(sid.to_string(), tok);
            }
    }

    pub fn undeclare_sess(&self, sid: &str) {
        if let Ok(mut m) = self.sess_tokens.lock() {
            m.remove(sid);
        }
    }

    pub async fn find_sess(&self, key: String) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let Ok(rx) = self.session.liveliness().get(key).timeout(Duration::from_secs(2)).await else { return out };
        while let Ok(r) = rx.recv_async().await {
            if let Ok(s) = r.result()
                && let Some(p) = self.keys.parse(s.key_expr().as_str())
                    && let [k, user, node, sid] = p.as_slice()
                        && k == "sess" {
                            out.push((user.clone(), node.clone(), sid.clone()));
                        }
        }
        out
    }

    pub async fn interest_room(self: &Arc<Self>, room: &str) {
        let edge = self.interest.lock().map(|mut i| i.add(room)).unwrap_or(Edge::Same);
        if edge != Edge::Declare {
            return;
        }
        let key = self.keys.ev_room(room);
        let Ok(sub) = self.session.declare_subscriber(key).await else { return };
        let me = self.clone();
        let r = room.to_string();
        let handler = sub.handler().clone();
        tokio::spawn(async move {
            while let Ok(s) = handler.recv_async().await {
                let Ok(n) = serde_json::from_slice::<Notice>(&s.payload().to_bytes()) else { continue };
                if let Some(sink) = me.sink.get() {
                    sink(Delivery::Room { room: r.clone(), notice: n });
                }
            }
        });
        if let Ok(mut m) = self.room_subs.lock() {
            m.insert(room.to_string(), sub);
        }
    }

    pub fn uninterest_room(&self, room: &str) {
        let edge = self.interest.lock().map(|mut i| i.remove(room)).unwrap_or(Edge::Same);
        if edge == Edge::Undeclare
            && let Ok(mut m) = self.room_subs.lock() {
                m.remove(room);
            }
    }

    pub async fn interest_user(self: &Arc<Self>, user: &str) {
        if self.user_subs.lock().map(|m| m.contains_key(user)).unwrap_or(true) {
            return;
        }
        let key = self.keys.ev_user(user);
        let Ok(sub) = self.session.declare_subscriber(key).await else { return };
        let me = self.clone();
        let u = user.to_string();
        let handler = sub.handler().clone();
        tokio::spawn(async move {
            while let Ok(s) = handler.recv_async().await {
                let Ok(n) = serde_json::from_slice::<Notice>(&s.payload().to_bytes()) else { continue };
                if let Some(sink) = me.sink.get() {
                    sink(Delivery::User { user: u.clone(), notice: n });
                }
            }
        });
        if let Ok(mut m) = self.user_subs.lock() {
            m.insert(user.to_string(), sub);
        }
    }

    pub fn uninterest_user(&self, user: &str) {
        if let Ok(mut m) = self.user_subs.lock() {
            m.remove(user);
        }
    }

    pub async fn set_local(&self, addr: &str) -> Result<(String, String), String> {
        let url = if addr.starts_with("http") { addr.to_string() } else { format!("http://{addr}") };
        let mut c = HandleClient::connect(url).await.map_err(|e| e.to_string())?;
        let r = c.hello(HelloReq { node: self.node.clone(), hub_inst: String::new() }).await.map_err(|e| e.to_string())?.into_inner();
        let mut g = self.local.write().await;
        g.replace(c);
        Ok((r.epoch, r.build))
    }

    pub async fn clear_local(&self) {
        self.local.write().await.take();
    }

    pub async fn call_local(&self, call: &Call) -> Reply {
        let c = self.local.read().await.clone();
        let Some(mut c) = c else { return Reply::fail(Failure::new(ErrCode::SfuUnavailable)) };
        match tokio::time::timeout(CALL_TIMEOUT, c.call(CallReq { envelope: call.encode() })).await {
            Ok(Ok(r)) => Reply::decode(&r.into_inner().reply).unwrap_or_else(|e| Reply::fail(Failure::new(ErrCode::InternalError).msg(e))),
            Ok(Err(e)) => Reply::fail(Failure::new(ErrCode::SfuError).msg(e.to_string())),
            Err(_) => Reply::fail(Failure::new(ErrCode::SfuError).msg("timeout")),
        }
    }

    pub async fn call(&self, node: &str, call: &Call) -> Reply {
        if node == self.node {
            return self.call_local(call).await;
        }
        if !self.node_live(node) {
            return Reply::fail(Failure::new(ErrCode::SfuUnavailable));
        }
        match self.get_one(&self.keys.q_node(node, "call"), call.encode()).await {
            Some(b) => Reply::decode(&b).unwrap_or_else(|e| Reply::fail(Failure::new(ErrCode::SfuError).msg(e))),
            None => Reply::fail(Failure::new(ErrCode::SfuError)),
        }
    }

    pub async fn call_room(&self, room: &str, call: &Call) -> Reply {
        match self.room_of(room) {
            Some(p) => self.call(&p.node, call).await,
            None => Reply::fail(Failure::new(ErrCode::RoomNotFound)),
        }
    }

    pub async fn fanout(&self, call: &Call) -> Vec<(String, Reply)> {
        let nodes: Vec<String> = self.live_nodes().keys().cloned().collect();
        let mut out = Vec::new();
        for n in nodes {
            let r = self.call(&n, call).await;
            out.push((n, r));
        }
        out
    }

    pub async fn ask(&self, node: &str, what: &str, payload: Vec<u8>) -> Option<Vec<u8>> {
        if node == self.node {
            let s = self.serve.get()?.clone();
            return Some(s(what.to_string(), payload).await);
        }
        self.get_one(&self.keys.q_node(node, what), payload).await
    }

    async fn get_one(&self, key: &str, payload: Vec<u8>) -> Option<Vec<u8>> {
        let rx = self.session.get(key).payload(payload).timeout(CALL_TIMEOUT).await.ok()?;
        while let Ok(r) = rx.recv_async().await {
            if let Ok(s) = r.result() {
                return Some(s.payload().to_bytes().to_vec());
            }
        }
        None
    }

    async fn serve_queries(self: &Arc<Self>) -> Result<(), String> {
        let q = self.session.declare_queryable(self.keys.q_node_all_of(&self.node)).await.map_err(|e| e.to_string())?;
        let me = self.clone();
        let handler = q.handler().clone();
        tokio::spawn(async move {
            while let Ok(query) = handler.recv_async().await {
                let me = me.clone();
                tokio::spawn(async move {
                    let key = query.key_expr().as_str().to_string();
                    let what = key.rsplit('/').next().unwrap_or("").to_string();
                    let payload = query.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
                    let out = if what == "call" {
                        match Call::decode(&payload) {
                            Ok(c) => me.call_local(&c).await.encode(),
                            Err(e) => Reply::fail(Failure::new(ErrCode::InvalidPayload).msg(e)).encode(),
                        }
                    } else if what == "bus" {
                        serde_json::to_vec(&me.view()).unwrap_or_default()
                    } else if let Some(s) = me.serve.get().cloned() {
                        s(what, payload).await
                    } else {
                        Vec::new()
                    };
                    let _ = query.reply(query.key_expr().clone(), out).await;
                });
            }
        });
        if let Ok(mut k) = self.keep.lock() {
            k.push(Box::new(q));
        }
        Ok(())
    }

    pub fn view(&self) -> Value {
        let nodes: Vec<Value> = self
            .live_nodes()
            .iter()
            .map(|(n, i)| if n == &self.node { json!({"node": n, "inst": i, "self": true}) } else { json!({"node": n, "inst": i}) })
            .collect();
        let rooms_seen: Vec<Value> = self
            .rooms()
            .into_iter()
            .filter(|(_, p)| p.node != self.node)
            .map(|(r, p)| json!({"room_id": r, "node": p.node, "inst": p.inst}))
            .collect();
        let peers: Vec<Value> = Vec::new();
        let last = self.last_event.lock().ok().and_then(|g| g.clone()).map(|(at, k, key)| json!({"at_ms": at, "kind": k, "key": key}));
        json!({
            "node": self.node,
            "inst": self.node_inst(),
            "mode": "router",
            "namespace": self.keys.ns,
            "listen": self.listen,
            "connect": self.connect,
            "peers": peers,
            "nodes": nodes,
            "rooms_seen": rooms_seen,
            "pending": {"rooms_unplaced": Vec::<String>::new(), "rooms_no_view": Vec::<String>::new()},
            "last_event": last,
            "isolated": self.is_isolated(),
            "duplicates": self.duplicates(),
        })
    }

    pub async fn local_hello(&self) -> Option<(String, String)> {
        let c = self.local.read().await.clone();
        let mut c = c?;
        let r = c.hello(HelloReq { node: self.node.clone(), hub_inst: String::new() }).await.ok()?.into_inner();
        Some((r.epoch, r.build))
    }

    pub async fn close(&self) {
        self.undeclare_node();
        if let Ok(mut m) = self.sess_tokens.lock() {
            m.clear();
        }
    }
}
