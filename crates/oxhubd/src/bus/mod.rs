// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:IMPL-BUS-FAULTS · 정:CAND-BUS-REQUEST-TIMEOUT · 정:CAND-BUS-HANDLE-SHAPE · model: claude-opus-5-5
//! 영역 BUS(게이트웨이 쪽) — zenoh router(임베드) · 존재 토큰 선언과 구독(history) · 통지 구독(참조계수) · node 간 요청 · node 안 gRPC Handle.

pub mod core;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::bus::envelope::{Call, Notice, NodeQuery, Reply};
use common::bus::grpc::handle_client::HandleClient;
use common::bus::grpc::CallReq;
use common::bus::keys::Keys;
use common::system::Zenoh as ZenohCfg;
use oxsig::Code;
use tokio::sync::{mpsc, oneshot};
use tonic::transport::Channel;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Subscriber;
use zenoh::sample::SampleKind;

use self::core::{NodeSet, RoomMap};

pub const NODE_CALL_TIMEOUT: Duration = Duration::from_secs(10);
pub const SESS_GET_TIMEOUT: Duration = Duration::from_secs(2);
pub const LOCAL_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const INBOUND_MAX: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Room(String),
    User(String),
}

#[derive(Debug, Clone)]
pub struct Inbound {
    pub to: Delivery,
    pub notice: Notice,
}

pub struct NodeAsk {
    pub query: NodeQuery,
    pub reply: oneshot::Sender<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomChange {
    Up,
    Down,
}

pub struct Bus {
    pub keys: Keys,
    pub me: String,
    pub zcfg: ZenohCfg,
    session: zenoh::Session,
    pub nodes: Mutex<NodeSet>,
    pub rooms: Mutex<RoomMap>,
    node_token: tokio::sync::Mutex<Option<(String, LivelinessToken)>>,
    sess_tokens: tokio::sync::Mutex<HashMap<String, (String, LivelinessToken)>>,
    room_subs: tokio::sync::Mutex<HashMap<String, (usize, Subscriber<()>)>>,
    user_subs: tokio::sync::Mutex<HashMap<String, (usize, Subscriber<()>)>>,
    local: Mutex<Option<HandleClient<Channel>>>,
    inbound: mpsc::Sender<Inbound>,
    room_changes: mpsc::UnboundedSender<(String, RoomChange)>,
    node_changes: mpsc::UnboundedSender<(String, String, bool)>,
    grace_ms: u64,
}

pub struct BusStreams {
    pub inbound: mpsc::Receiver<Inbound>,
    pub asks: mpsc::Receiver<NodeAsk>,
    pub room_changes: mpsc::UnboundedReceiver<(String, RoomChange)>,
    pub node_changes: mpsc::UnboundedReceiver<(String, String, bool)>,
}

fn now_ms() -> u64 {
    common::time::wall_ms()
}

impl Bus {
    pub async fn open(me: &str, zcfg: &ZenohCfg, grace_secs: u64) -> Result<(Arc<Bus>, BusStreams), String> {
        let cfg = common::bus::session::router_config(zcfg).map_err(|e| e.to_string())?;
        let session = common::bus::session::open(cfg).await.map_err(|e| e.to_string())?;
        let (in_tx, in_rx) = mpsc::channel(INBOUND_MAX);
        let (ask_tx, ask_rx) = mpsc::channel(1024);
        let (rc_tx, rc_rx) = mpsc::unbounded_channel();
        let (nc_tx, nc_rx) = mpsc::unbounded_channel();
        let bus = Arc::new(Bus {
            keys: Keys::new(&zcfg.namespace),
            me: me.to_string(),
            zcfg: zcfg.clone(),
            session,
            nodes: Mutex::new(NodeSet::new(me)),
            rooms: Mutex::new(RoomMap::default()),
            node_token: tokio::sync::Mutex::new(None),
            sess_tokens: tokio::sync::Mutex::new(HashMap::new()),
            room_subs: tokio::sync::Mutex::new(HashMap::new()),
            user_subs: tokio::sync::Mutex::new(HashMap::new()),
            local: Mutex::new(None),
            inbound: in_tx,
            room_changes: rc_tx,
            node_changes: nc_tx,
            grace_ms: grace_secs * 1000,
        });
        bus.watch_presence().await?;
        bus.serve_queries(ask_tx).await?;
        Ok((bus.clone(), BusStreams { inbound: in_rx, asks: ask_rx, room_changes: rc_rx, node_changes: nc_rx }))
    }

    async fn watch_presence(self: &Arc<Self>) -> Result<(), String> {
        let b = Arc::downgrade(self);
        let node_sub = self
            .session
            .liveliness()
            .declare_subscriber(self.keys.node_all())
            .history(true)
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            while let Ok(s) = node_sub.recv_async().await {
                let Some(bus) = b.upgrade() else { break };
                let Some((node, inst)) = bus.keys.parse_node(s.key_expr().as_str()) else { continue };
                let put = s.kind() == SampleKind::Put;
                let was_isolated;
                {
                    let mut ns = bus.nodes.lock().unwrap_or_else(|p| p.into_inner());
                    was_isolated = ns.isolated;
                    if put {
                        ns.put(&node, &inst, now_ms());
                    } else {
                        ns.delete(&node, &inst, now_ms());
                    }
                }
                if put && was_isolated {
                    bus.redeclare_all().await;
                }
                let _ = bus.node_changes.send((node, inst, put));
            }
        });
        let b = Arc::downgrade(self);
        let room_sub = self
            .session
            .liveliness()
            .declare_subscriber(self.keys.room_all())
            .history(true)
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            while let Ok(s) = room_sub.recv_async().await {
                let Some(bus) = b.upgrade() else { break };
                let Some((room, node, inst)) = bus.keys.parse_room(s.key_expr().as_str()) else { continue };
                let up = s.kind() == SampleKind::Put;
                {
                    let mut rm = bus.rooms.lock().unwrap_or_else(|p| p.into_inner());
                    if up {
                        if rm.put(&room, &node, &inst) {
                            tracing::warn!(kind = "ev", spec = "RULE-BUS-PLACEMENT", room = %room, node = %node, "room on two nodes — duplicate detected");
                        }
                    } else {
                        rm.delete(&room, &node, &inst);
                    }
                }
                let _ = bus.room_changes.send((room, if up { RoomChange::Up } else { RoomChange::Down }));
            }
        });
        Ok(())
    }

    async fn serve_queries(self: &Arc<Self>, asks: mpsc::Sender<NodeAsk>) -> Result<(), String> {
        let q = self.session.declare_queryable(self.keys.q_node_queryable(&self.me)).await.map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            while let Ok(query) = q.recv_async().await {
                let asks = asks.clone();
                tokio::spawn(async move {
                    let payload = query.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
                    let reply = match serde_json::from_slice::<NodeQuery>(&payload) {
                        Ok(nq) => {
                            let (tx, rx) = oneshot::channel();
                            if asks.send(NodeAsk { query: nq, reply: tx }).await.is_err() {
                                Reply::fail(Code::InternalError).encode()
                            } else {
                                rx.await.unwrap_or_else(|_| Reply::fail(Code::InternalError).encode())
                            }
                        }
                        Err(e) => Reply::Fail(Code::InvalidPayload.failure_with(e.to_string(), None)).encode(),
                    };
                    let key = query.key_expr().clone();
                    let _ = query.reply(key, reply).await;
                });
            }
        });
        Ok(())
    }

    pub fn set_local(&self, c: Option<HandleClient<Channel>>) {
        let mut g = self.local.lock().unwrap_or_else(|p| p.into_inner());
        let slot: &mut Option<HandleClient<Channel>> = &mut g;
        let _ = std::mem::replace(slot, c);
    }

    pub async fn set_node_inst(&self, inst: Option<&str>) {
        let mut g = self.node_token.lock().await;
        if g.as_ref().map(|(i, _)| i.as_str()) == inst {
            return;
        }
        if let Some((_, t)) = g.take() {
            let _ = t.undeclare().await;
        }
        if let Some(i) = inst {
            match self.session.liveliness().declare_token(self.keys.node(&self.me, i)).await {
                Ok(t) => {
                    tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", node = %self.me, inst = %i, "node token declared");
                    let _ = g.insert((i.to_string(), t));
                }
                Err(e) => tracing::warn!(kind = "diag", "node token: {e}"),
            }
        }
    }

    pub async fn declare_sess(&self, user: &str, sid: &str) {
        let key = self.keys.sess(user, &self.me, sid);
        if let Ok(t) = self.session.liveliness().declare_token(key).await {
            self.sess_tokens.lock().await.insert(sid.to_string(), (user.to_string(), t));
        }
    }

    pub async fn undeclare_sess(&self, sid: &str) {
        if let Some((_, t)) = self.sess_tokens.lock().await.remove(sid) {
            let _ = t.undeclare().await;
        }
    }

    async fn redeclare_all(&self) {
        {
            let mut g = self.node_token.lock().await;
            if let Some((i, t)) = g.take() {
                let _ = t.undeclare().await;
                if let Ok(t2) = self.session.liveliness().declare_token(self.keys.node(&self.me, &i)).await {
                    let _ = g.insert((i, t2));
                }
            }
        }
        let mut st = self.sess_tokens.lock().await;
        let entries: Vec<(String, String)> = st.iter().map(|(sid, (u, _))| (sid.clone(), u.clone())).collect();
        for (sid, user) in entries {
            if let Some((_, t)) = st.remove(&sid) {
                let _ = t.undeclare().await;
            }
            if let Ok(t) = self.session.liveliness().declare_token(self.keys.sess(&user, &self.me, &sid)).await {
                st.insert(sid, (user, t));
            }
        }
        tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", "re-declared node and session tokens after isolation");
    }

    fn sink(&self, to: Delivery) -> impl Fn(zenoh::sample::Sample) + Send + Sync + 'static {
        let tx = self.inbound.clone();
        move |s: zenoh::sample::Sample| {
            if let Some(n) = Notice::decode(&s.payload().to_bytes()) {
                let _ = tx.try_send(Inbound { to: to.clone(), notice: n });
            }
        }
    }

    pub async fn interest_room(&self, room: &str) {
        let mut g = self.room_subs.lock().await;
        if let Some(e) = g.get_mut(room) {
            e.0 += 1;
            return;
        }
        let key = self.keys.ev_room(room);
        match self.session.declare_subscriber(key).callback(self.sink(Delivery::Room(room.to_string()))).await {
            Ok(sub) => {
                g.insert(room.to_string(), (1, sub));
            }
            Err(e) => tracing::warn!(kind = "diag", room = %room, "ev/room subscribe: {e}"),
        }
    }

    pub async fn release_room(&self, room: &str) {
        let mut g = self.room_subs.lock().await;
        let drop_it = match g.get_mut(room) {
            Some(e) => {
                e.0 = e.0.saturating_sub(1);
                e.0 == 0
            }
            None => false,
        };
        if drop_it && let Some((_, sub)) = g.remove(room) {
            let _ = sub.undeclare().await;
        }
    }

    pub async fn interest_user(&self, user: &str) {
        let mut g = self.user_subs.lock().await;
        if let Some(e) = g.get_mut(user) {
            e.0 += 1;
            return;
        }
        let key = self.keys.ev_user(user);
        if let Ok(sub) = self.session.declare_subscriber(key).callback(self.sink(Delivery::User(user.to_string()))).await {
            g.insert(user.to_string(), (1, sub));
        }
    }

    pub async fn release_user(&self, user: &str) {
        let mut g = self.user_subs.lock().await;
        let drop_it = match g.get_mut(user) {
            Some(e) => {
                e.0 = e.0.saturating_sub(1);
                e.0 == 0
            }
            None => false,
        };
        if drop_it && let Some((_, sub)) = g.remove(user) {
            let _ = sub.undeclare().await;
        }
    }

    pub fn room_node(&self, room: &str) -> Option<String> {
        self.rooms.lock().ok().and_then(|m| m.node_of(room).map(str::to_string))
    }

    pub fn node_live(&self, node: &str) -> bool {
        self.nodes.lock().map(|n| n.live(node)).unwrap_or(false)
    }

    pub fn node_of_sfu(&self, sfu_id: &str) -> Option<String> {
        self.nodes.lock().ok().and_then(|n| n.node_of_inst(sfu_id).map(str::to_string))
    }

    pub fn my_inst(&self) -> Option<String> {
        self.nodes.lock().ok().and_then(|n| n.inst_of(&self.me).map(str::to_string))
    }

    pub fn candidates(&self) -> Vec<String> {
        self.nodes.lock().map(|n| n.candidates(now_ms(), self.grace_ms).into_iter().map(str::to_string).collect()).unwrap_or_default()
    }

    pub fn alive_nodes(&self) -> Vec<String> {
        self.nodes.lock().map(|n| n.alive_nodes()).unwrap_or_default()
    }

    pub fn isolated(&self) -> bool {
        self.nodes.lock().map(|n| n.isolated).unwrap_or(false)
    }

    pub async fn call(&self, node: &str, call: &Call) -> Reply {
        if node == self.me {
            return self.call_local(call).await;
        }
        if !self.node_live(node) {
            return Reply::fail(Code::SfuUnavailable);
        }
        let q = NodeQuery::Call { call: call.clone(), unit: None };
        match self.query(node, "call", &q, NODE_CALL_TIMEOUT).await {
            Some(b) => Reply::decode(&b),
            None => Reply::fail(Code::SfuError),
        }
    }

    pub async fn call_local(&self, call: &Call) -> Reply {
        let client = self.local.lock().ok().and_then(|g| g.clone());
        let Some(mut c) = client else { return Reply::fail(Code::SfuUnavailable) };
        match tokio::time::timeout(LOCAL_CALL_TIMEOUT, c.call(CallReq { envelope: call.encode() })).await {
            Ok(Ok(r)) => Reply::decode(&r.into_inner().reply),
            Ok(Err(e)) => Reply::Fail(Code::SfuError.failure_with(e.message().to_string(), None)),
            Err(_) => Reply::fail(Code::SfuError),
        }
    }

    pub async fn query(&self, node: &str, what: &str, q: &NodeQuery, timeout: Duration) -> Option<Vec<u8>> {
        let body = serde_json::to_vec(q).ok()?;
        let replies = self.session.get(self.keys.q_node(node, what)).payload(body).timeout(timeout).await.ok()?;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result() {
                return Some(s.payload().to_bytes().to_vec());
            }
        }
        None
    }

    pub async fn sess_by_sid(&self, sid: &str) -> Option<(String, String)> {
        let replies = self.session.liveliness().get(self.keys.sess_by_sid(sid)).timeout(SESS_GET_TIMEOUT).await.ok()?;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some((user, node, _)) = self.keys.parse_sess(s.key_expr().as_str())
            {
                return Some((user, node));
            }
        }
        None
    }

    pub async fn sess_of_user(&self, user: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(replies) = self.session.liveliness().get(self.keys.sess_of_user(user)).timeout(SESS_GET_TIMEOUT).await else { return out };
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some((_, node, sid)) = self.keys.parse_sess(s.key_expr().as_str())
            {
                out.push((node, sid));
            }
        }
        out
    }

    pub async fn sess_count(&self) -> usize {
        let Ok(replies) = self.session.liveliness().get(format!("ox/{}/int/sess/**", self.keys.ns)).timeout(SESS_GET_TIMEOUT).await else { return 0 };
        let mut n = 0;
        while let Ok(r) = replies.recv_async().await {
            if r.result().is_ok() {
                n += 1;
            }
        }
        n
    }

    pub async fn info(&self) -> serde_json::Value {
        let zid = self.session.info().zid().await.to_string();
        let links: Vec<(String, String)> = self.session.info().links().await.map(|l| (l.zid().to_string(), l.dst().to_string())).collect();
        let peers: Vec<serde_json::Value> = self
            .session
            .info()
            .transports()
            .await
            .map(|t| {
                let z = t.zid().to_string();
                let ls: Vec<&String> = links.iter().filter(|(lz, _)| lz == &z).map(|(_, d)| d).collect();
                serde_json::json!({"zid": z, "whatami": t.whatami().to_string(), "links": ls})
            })
            .collect();
        let (nodes, last, my_inst) = {
            let ns = self.nodes.lock().unwrap_or_else(|p| p.into_inner());
            let nodes: Vec<serde_json::Value> = ns
                .nodes
                .iter()
                .filter(|(_, e)| e.alive)
                .map(|(n, e)| if n == &self.me { serde_json::json!({"node": n, "inst": e.inst, "self": true}) } else { serde_json::json!({"node": n, "inst": e.inst}) })
                .collect();
            let last = ns.last_event.clone().map(|(at, kind, key)| serde_json::json!({"at_ms": at, "kind": kind, "key": key}));
            (nodes, last, ns.inst_of(&self.me).map(str::to_string))
        };
        let (rooms_seen, dup) = {
            let rm = self.rooms.lock().unwrap_or_else(|p| p.into_inner());
            let seen: Vec<serde_json::Value> = rm
                .rooms
                .iter()
                .flat_map(|(r, s)| s.iter().filter(|(n, _)| n != &self.me).map(move |(n, i)| serde_json::json!({"room_id": r, "node": n, "inst": i})))
                .collect();
            (seen, rm.duplicates_now())
        };
        let mut v = serde_json::json!({
            "node": self.me, "mode": self.zcfg.mode, "namespace": self.zcfg.namespace,
            "listen": self.zcfg.listen, "connect": self.zcfg.connect, "zid": zid,
            "peers": peers, "nodes": nodes, "rooms_seen": rooms_seen,
            "pending": {"rooms_unplaced": serde_json::Value::Array(vec![]), "rooms_no_view": serde_json::Value::Array(vec![]), "rooms_duplicated": dup},
        });
        if let Some(i) = my_inst {
            v["inst"] = serde_json::Value::String(i);
        }
        if let Some(l) = last {
            v["last_event"] = l;
        }
        v
    }

    pub async fn close(&self) {
        if let Some((_, t)) = self.node_token.lock().await.take() {
            let _ = t.undeclare().await;
        }
        let toks: Vec<(String, (String, LivelinessToken))> = self.sess_tokens.lock().await.drain().collect();
        for (_, (_, t)) in toks {
            let _ = t.undeclare().await;
        }
    }

    pub async fn shutdown_session(&self) {
        let _ = self.session.close().await;
    }
}
