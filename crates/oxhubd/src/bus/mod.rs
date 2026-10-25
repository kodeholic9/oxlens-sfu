// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:RULE-BUS-KEYSPACE 정:RULE-BUS-PRESENCE 정:RULE-BUS-PLACEMENT 정:RULE-BUS-EVENTS 정:RULE-BUS-REQUEST 정:IMPL-BUS-FAULTS 정:CAND-BUS-HOME-INST 정:CAND-BUS-REQUEST-TIMEOUT 운영:RULE-READ-BUS · model: claude-opus-5-5
//! hub 쪽 버스 — zenoh router 세션 · `node` · `sess` 토큰 선언 · `node` · `room` 토큰 `history` 구독(방 → node 맵 · node 집합 · 고립 판정) · `ev` 관심 선언 · node 사이 요청.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::bus::{Keys, NODE_CALL_TIMEOUT, SESS_GET_TIMEOUT};
use oxsig::Code;
use oxsig::envelope::Notice;
use tokio::sync::{Mutex, mpsc};
use zenoh::liveliness::LivelinessToken;
use zenoh::sample::SampleKind;

#[derive(Debug, Clone)]
pub enum Incoming {
    Room { room: String, notice: Notice },
    User { user: String, notice: Notice },
    RoomGone { room: String },
}

pub struct QueryIn {
    pub what: String,
    pub body: Vec<u8>,
    pub query: zenoh::query::Query,
}

#[derive(Debug, Clone, Default)]
struct NodeView {
    insts: BTreeSet<String>,
    gone_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct LastEvent {
    pub at_ms: u64,
    pub kind: &'static str,
    pub key: String,
}

#[derive(Default)]
struct State {
    nodes: BTreeMap<String, NodeView>,
    rooms: BTreeMap<String, BTreeSet<(String, String)>>,
    last: Option<LastEvent>,
    isolated: bool,
    others_seen: bool,
}

pub struct Bus {
    pub z: zenoh::Session,
    pub keys: Keys,
    pub node: String,
    pub inst: String,
    pub grace: Duration,
    pub mode: String,
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    st: std::sync::Mutex<State>,
    node_token: Mutex<Option<LivelinessToken>>,
    sess_tokens: Mutex<HashMap<String, LivelinessToken>>,
    user_subs: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    room_subs: Mutex<HashMap<String, (usize, tokio::task::JoinHandle<()>)>>,
    ev_tx: mpsc::UnboundedSender<Incoming>,
    pub dup_rooms: AtomicU64,
}

pub struct BusOpen {
    pub bus: Arc<Bus>,
    pub events: mpsc::UnboundedReceiver<Incoming>,
    pub queries: mpsc::UnboundedReceiver<QueryIn>,
}

fn err_s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Bus {
    pub async fn open(cfg: &common::system::Zenoh, node: &str, inst: &str, grace: Duration) -> Result<BusOpen, String> {
        let zc = common::bus::zenoh_config(&common::bus::ZenohOpts {
            mode: &cfg.mode,
            listen: &cfg.listen,
            connect: &cfg.connect,
            multicast_scouting: cfg.multicast_scouting,
            base_config: cfg.base_config.as_deref(),
        })?;
        let z = zenoh::open(zc).await.map_err(err_s)?;
        let (ev_tx, events) = mpsc::unbounded_channel();
        let (q_tx, queries) = mpsc::unbounded_channel();
        let bus = Arc::new(Bus {
            z,
            keys: Keys::new(&cfg.namespace),
            node: node.to_string(),
            inst: inst.to_string(),
            grace,
            mode: cfg.mode.clone(),
            listen: cfg.listen.clone(),
            connect: cfg.connect.clone(),
            st: std::sync::Mutex::new(State::default()),
            node_token: Mutex::new(None),
            sess_tokens: Mutex::new(HashMap::new()),
            user_subs: Mutex::new(HashMap::new()),
            room_subs: Mutex::new(HashMap::new()),
            ev_tx,
            dup_rooms: AtomicU64::new(0),
        });
        let node_sub = bus.z.liveliness().declare_subscriber(bus.keys.node_all()).history(true).await.map_err(err_s)?;
        let b = bus.clone();
        tokio::spawn(async move {
            while let Ok(s) = node_sub.recv_async().await {
                b.on_node_token(s.key_expr().as_str(), s.kind());
            }
        });
        let room_sub = bus.z.liveliness().declare_subscriber(bus.keys.room_all()).history(true).await.map_err(err_s)?;
        let b = bus.clone();
        tokio::spawn(async move {
            while let Ok(s) = room_sub.recv_async().await {
                b.on_room_token(s.key_expr().as_str(), s.kind());
            }
        });
        let qable = bus.z.declare_queryable(bus.keys.q_node_all(node)).await.map_err(err_s)?;
        let pre = format!("{}/", bus.keys.q_node(node, "").trim_end_matches('/'));
        tokio::spawn(async move {
            while let Ok(q) = qable.recv_async().await {
                let what = q.key_expr().as_str().strip_prefix(pre.as_str()).unwrap_or("").to_string();
                let body = q.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
                if q_tx.send(QueryIn { what, body, query: q }).is_err() {
                    break;
                }
            }
        });
        Ok(BusOpen { bus, events, queries })
    }

    fn note(st: &mut State, kind: &'static str, key: &str) {
        st.last = Some(LastEvent { at_ms: common::log::now_ms(), kind, key: key.to_string() });
    }

    fn on_node_token(&self, key: &str, kind: SampleKind) {
        let Some(t) = self.keys.tail(key, "node") else {
            return;
        };
        if t.len() != 2 {
            return;
        }
        let (n, i) = (t[0].to_string(), t[1].to_string());
        let mut st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        let put = kind == SampleKind::Put;
        Self::note(&mut st, if put { "put" } else { "delete" }, key);
        {
            let v = st.nodes.entry(n.clone()).or_default();
            if put {
                v.insts.insert(i.clone());
                v.gone_at = None;
            } else {
                v.insts.remove(&i);
                if v.insts.is_empty() {
                    v.gone_at = Some(Instant::now());
                }
            }
        }
        let others = st.nodes.iter().filter(|(k, v)| **k != self.node && !v.insts.is_empty()).count();
        if others > 0 {
            st.others_seen = true;
            if st.isolated {
                st.isolated = false;
                tracing::warn!("bus isolation cleared — other node tokens visible again");
            }
        } else if st.others_seen && !put && n != self.node && !st.isolated {
            st.isolated = true;
            tracing::warn!("bus isolation — every other node token vanished; routing kept · home reaping deferred");
        }
        drop(st);
        tracing::info!("node token {} node={n} inst={i}", if put { "put" } else { "delete" });
    }

    fn on_room_token(&self, key: &str, kind: SampleKind) {
        let Some(t) = self.keys.tail(key, "room") else {
            return;
        };
        if t.len() != 3 {
            return;
        }
        let (r, n, i) = (t[0].to_string(), t[1].to_string(), t[2].to_string());
        let mut st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        Self::note(&mut st, if kind == SampleKind::Put { "put" } else { "delete" }, key);
        if kind == SampleKind::Put {
            let e = st.rooms.entry(r.clone()).or_default();
            e.insert((n, i));
            let nodes: BTreeSet<&String> = e.iter().map(|(n, _)| n).collect();
            if nodes.len() > 1 {
                self.dup_rooms.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(room = r.as_str(), "room duplicate — two nodes hold room {r}");
            }
            return;
        }
        let isolated = st.isolated;
        if let Some(e) = st.rooms.get_mut(&r) {
            if isolated {
                return;
            }
            e.remove(&(n, i));
            if e.is_empty() {
                st.rooms.remove(&r);
                drop(st);
                let _ = self.ev_tx.send(Incoming::RoomGone { room: r });
            }
        }
    }

    pub fn last_event(&self) -> Option<LastEvent> {
        self.st.lock().ok().and_then(|s| s.last.clone())
    }

    pub fn node_live(&self, node: &str) -> bool {
        self.st.lock().map(|s| s.nodes.get(node).is_some_and(|v| !v.insts.is_empty())).unwrap_or(false)
    }

    pub fn nodes(&self) -> Vec<(String, String)> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.nodes.iter().flat_map(|(n, v)| v.insts.iter().map(move |i| (n.clone(), i.clone()))).collect()
    }

    pub fn live_nodes(&self) -> Vec<String> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.nodes.iter().filter(|(_, v)| !v.insts.is_empty()).map(|(n, _)| n.clone()).collect()
    }

    pub fn candidates(&self) -> Vec<String> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.nodes.iter().filter(|(_, v)| !v.insts.is_empty() || v.gone_at.is_some_and(|g| g.elapsed() < self.grace)).map(|(n, _)| n.clone()).collect()
    }

    pub fn room_where(&self, room: &str) -> Option<(String, String)> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.rooms.get(room).and_then(|s| s.iter().next().cloned())
    }

    pub fn rooms(&self) -> Vec<(String, String, String)> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.rooms.iter().flat_map(|(r, s)| s.iter().map(move |(n, i)| (r.clone(), n.clone(), i.clone()))).collect()
    }

    pub fn room_count(&self) -> usize {
        self.st.lock().map(|s| s.rooms.len()).unwrap_or(0)
    }

    pub fn inst_node(&self, inst: &str) -> Option<String> {
        let st = self.st.lock().unwrap_or_else(|p| p.into_inner());
        st.rooms.values().flat_map(|s| s.iter()).find(|(_, i)| i == inst).map(|(n, _)| n.clone())
    }

    pub async fn declare_node(&self) -> Result<(), String> {
        let mut g = self.node_token.lock().await;
        if g.is_some() {
            return Ok(());
        }
        let t = self.z.liveliness().declare_token(self.keys.node(&self.node, &self.inst)).await.map_err(err_s)?;
        g.replace(t);
        Ok(())
    }

    pub async fn declare_sess(&self, user: &str, sid: &str) -> Result<(), String> {
        let t = self.z.liveliness().declare_token(self.keys.sess(user, &self.node, sid)).await.map_err(err_s)?;
        self.sess_tokens.lock().await.insert(sid.to_string(), t);
        Ok(())
    }

    pub async fn undeclare_sess(&self, sid: &str) {
        let t = self.sess_tokens.lock().await.remove(sid);
        if let Some(t) = t {
            let _ = t.undeclare().await;
        }
    }

    pub async fn watch_user(&self, user: &str) -> Result<(), String> {
        let mut subs = self.user_subs.lock().await;
        if subs.contains_key(user) {
            return Ok(());
        }
        let sub = self.z.declare_subscriber(self.keys.ev_user(user)).await.map_err(err_s)?;
        let tx = self.ev_tx.clone();
        let u = user.to_string();
        let h = tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                if let Ok(n) = serde_json::from_slice::<Notice>(&s.payload().to_bytes()) {
                    let _ = tx.send(Incoming::User { user: u.clone(), notice: n });
                }
            }
        });
        subs.insert(user.to_string(), h);
        Ok(())
    }

    pub async fn unwatch_user(&self, user: &str) {
        if let Some(h) = self.user_subs.lock().await.remove(user) {
            h.abort();
        }
    }

    pub async fn room_interest(&self, room: &str, delta: i32) -> Result<(), String> {
        let mut subs = self.room_subs.lock().await;
        if delta > 0 {
            if let Some(e) = subs.get_mut(room) {
                e.0 += 1;
                return Ok(());
            }
            let sub = self.z.declare_subscriber(self.keys.ev_room(room)).await.map_err(err_s)?;
            let tx = self.ev_tx.clone();
            let r = room.to_string();
            let h = tokio::spawn(async move {
                while let Ok(s) = sub.recv_async().await {
                    if let Ok(n) = serde_json::from_slice::<Notice>(&s.payload().to_bytes()) {
                        let _ = tx.send(Incoming::Room { room: r.clone(), notice: n });
                    }
                }
            });
            subs.insert(room.to_string(), (1, h));
        } else if let Some(e) = subs.get_mut(room) {
            e.0 = e.0.saturating_sub(1);
            if e.0 == 0
                && let Some((_, h)) = subs.remove(room)
            {
                h.abort();
            }
        }
        Ok(())
    }

    pub async fn drop_room_interest(&self, room: &str) {
        if let Some((_, h)) = self.room_subs.lock().await.remove(room) {
            h.abort();
        }
    }

    pub async fn ask(&self, node: &str, what: &str, body: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, Code> {
        if !self.node_live(node) {
            return Err(Code::SfuUnavailable);
        }
        let replies = self.z.get(self.keys.q_node(node, what)).payload(body).timeout(timeout).await.map_err(|_| Code::SfuError)?;
        match replies.recv_async().await {
            Ok(r) => match r.result() {
                Ok(s) => Ok(s.payload().to_bytes().to_vec()),
                Err(_) => Err(Code::SfuError),
            },
            Err(_) => Err(Code::SfuError),
        }
    }

    pub async fn ask_call(&self, node: &str, body: Vec<u8>) -> Result<Vec<u8>, Code> {
        self.ask(node, "call", body, NODE_CALL_TIMEOUT).await
    }

    pub async fn sess_by_sid(&self, sid: &str) -> Option<(String, String)> {
        let replies = self.z.liveliness().get(self.keys.sess_by_sid(sid)).timeout(SESS_GET_TIMEOUT).await.ok()?;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some(t) = self.keys.tail(s.key_expr().as_str(), "sess")
                && t.len() == 3
            {
                return Some((t[0].to_string(), t[1].to_string()));
            }
        }
        None
    }

    pub async fn sess_of_user(&self, user: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Ok(replies) = self.z.liveliness().get(self.keys.sess_of_user(user)).timeout(SESS_GET_TIMEOUT).await else {
            return out;
        };
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some(t) = self.keys.tail(s.key_expr().as_str(), "sess")
                && t.len() == 3
            {
                out.push((t[1].to_string(), t[2].to_string()));
            }
        }
        out
    }

    pub async fn peers(&self) -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        for z in self.z.info().routers_zid().await {
            out.push((z.to_string(), "router"));
        }
        for z in self.z.info().peers_zid().await {
            out.push((z.to_string(), "peer"));
        }
        out
    }

    pub async fn close(&self) {
        self.node_token.lock().await.take();
        self.sess_tokens.lock().await.clear();
        let _ = self.z.close().await;
    }
}
