// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:IMPL-BUS-FAULTS · model: claude-opus-5-5
//! hub 의 버스 — zenoh router(임베드) · 토큰 선언 · 방 → node 맵 · 관심 구독 · node 안 Handle · node 사이 요청.

pub mod placement;
pub mod view;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::envelope::handle::handle_client::HandleClient;
use common::envelope::handle::{CallReq, HelloReq};
use common::envelope::{Envelope, Notice, Reply};
use common::keyspace::{Keys, Parsed};
use oxsig::Code;
use tokio::sync::mpsc;
use zenoh::Session;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Subscriber;
use zenoh::query::Query;

pub use view::{TokenEv, View};

pub const NODE_CALL_TIMEOUT: Duration = Duration::from_secs(10);
pub const SESS_GET_TIMEOUT: Duration = Duration::from_secs(2);
pub const LOCAL_CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub enum BusEvent {
    RoomNotice { room: String, notice: Notice },
    UserNotice { user: String, notice: Notice },
    RoomDropped(String),
    Query(Query),
}

pub struct ZenohCfg {
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    pub multicast_scouting: bool,
    pub namespace: String,
    pub base_config: Option<String>,
}

#[derive(Clone)]
pub struct Local {
    pub client: HandleClient<tonic::transport::Channel>,
    pub epoch: String,
    pub unit: String,
}

type SubMap = Mutex<HashMap<String, (usize, Subscriber<()>)>>;

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub inst: String,
    pub cfg_listen: Vec<String>,
    pub cfg_connect: Vec<String>,
    pub view: Mutex<View>,
    session: Session,
    tx: mpsc::UnboundedSender<BusEvent>,
    node_token: Mutex<Option<LivelinessToken>>,
    sess_tokens: Mutex<HashMap<String, LivelinessToken>>,
    room_subs: SubMap,
    user_subs: SubMap,
    local: Mutex<Option<Local>>,
    keep: Mutex<Vec<Subscriber<()>>>,
    pub ev_drop: std::sync::atomic::AtomicU64,
}

enum Ev {
    Room(String),
    User(String),
}

impl Bus {
    pub async fn open(
        node: &str,
        inst: &str,
        grace_ms: u64,
        z: ZenohCfg,
    ) -> Result<(Arc<Bus>, mpsc::UnboundedReceiver<BusEvent>), String> {
        let mut cfg = match &z.base_config {
            Some(p) => zenoh::Config::from_file(p).map_err(|e| format!("zenoh base_config: {e}"))?,
            None => zenoh::Config::default(),
        };
        let set = |cfg: &mut zenoh::Config, k: &str, v: String| cfg.insert_json5(k, &v).map_err(|e| format!("zenoh {k}: {e}"));
        set(&mut cfg, "mode", "\"router\"".into())?;
        set(&mut cfg, "listen/endpoints", serde_json::to_string(&z.listen).unwrap_or_default())?;
        set(&mut cfg, "connect/endpoints", serde_json::to_string(&z.connect).unwrap_or_default())?;
        set(&mut cfg, "scouting/multicast/enabled", z.multicast_scouting.to_string())?;
        set(&mut cfg, "transport/unicast/qos/enabled", "true".into())?;
        set(&mut cfg, "transport/unicast/lowlatency", "false".into())?;
        let session = zenoh::open(cfg).await.map_err(|e| format!("zenoh open: {e}"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        let bus = Arc::new(Bus {
            keys: Keys::new(&z.namespace),
            node: node.to_string(),
            inst: inst.to_string(),
            cfg_listen: z.listen,
            cfg_connect: z.connect,
            view: Mutex::new(View::new(node, inst, grace_ms)),
            session,
            tx,
            node_token: Mutex::new(None),
            sess_tokens: Mutex::new(HashMap::new()),
            room_subs: Mutex::new(HashMap::new()),
            user_subs: Mutex::new(HashMap::new()),
            local: Mutex::new(None),
            keep: Mutex::new(Vec::new()),
            ev_drop: std::sync::atomic::AtomicU64::new(0),
        });
        bus.start().await?;
        Ok((bus, rx))
    }

    async fn token_sub(self: &Arc<Self>, key: String) -> Result<Subscriber<()>, String> {
        let me = Arc::downgrade(self);
        let keys = self.keys.clone();
        self.session
            .liveliness()
            .declare_subscriber(key)
            .history(true)
            .callback(move |s| {
                if let Some(b) = me.upgrade() {
                    b.on_token_sample(&keys, s.key_expr().as_str(), s.kind() == zenoh::sample::SampleKind::Put);
                }
            })
            .await
            .map_err(|e| format!("token sub: {e}"))
    }

    async fn start(self: &Arc<Self>) -> Result<(), String> {
        let node_sub = self.token_sub(self.keys.node_all()).await?;
        let room_sub = self.token_sub(self.keys.room_all()).await?;
        if let Ok(mut k) = self.keep.lock() {
            k.push(node_sub);
            k.push(room_sub);
        }
        let q = self
            .session
            .declare_queryable(self.keys.q_node_serve(&self.node))
            .await
            .map_err(|e| format!("queryable: {e}"))?;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Ok(query) = q.recv_async().await {
                if tx.send(BusEvent::Query(query)).is_err() {
                    break;
                }
            }
        });
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let Some(b) = me.upgrade() else { break };
                let dropped = match b.view.lock() {
                    Ok(mut v) => v.settle(common::now_ms()).dropped_rooms,
                    Err(_) => Vec::new(),
                };
                for r in dropped {
                    if let Some((node, inst)) = b.room_token(&r).await {
                        if let Ok(mut v) = b.view.lock() {
                            v.on_token(TokenEv::RoomPut { room: r.clone(), node, inst }, common::now_ms());
                        }
                        tracing::info!(kind = "diag", room = %r, "room token delete was stale — token still stands, routing kept");
                        continue;
                    }
                    tracing::info!(kind = "diag", room = %r, "room token gone — local routing dropped");
                    let _ = b.tx.send(BusEvent::RoomDropped(r));
                }
            }
        });
        Ok(())
    }

    fn on_token_sample(&self, keys: &Keys, key: &str, put: bool) {
        let ev = match keys.parse(key) {
            Some(Parsed::Node { node, inst }) if put => TokenEv::NodePut { node, inst },
            Some(Parsed::Node { node, inst }) => TokenEv::NodeDel { node, inst },
            Some(Parsed::Room { room, node, inst }) if put => TokenEv::RoomPut { room, node, inst },
            Some(Parsed::Room { room, node, inst }) => TokenEv::RoomDel { room, node, inst },
            _ => return,
        };
        if let Ok(mut v) = self.view.lock() {
            v.on_token(ev, common::now_ms());
        }
    }

    pub async fn declare_self(&self) -> Result<(), String> {
        let t = self
            .session
            .liveliness()
            .declare_token(self.keys.node_token(&self.node, &self.inst))
            .await
            .map_err(|e| format!("node token: {e}"))?;
        if let Ok(mut g) = self.node_token.lock() {
            g.replace(t);
        }
        if let Ok(mut v) = self.view.lock() {
            v.self_declared = true;
        }
        Ok(())
    }

    pub fn self_declared(&self) -> bool {
        self.view.lock().map(|v| v.self_declared).unwrap_or(false)
    }

    pub async fn declare_sess(&self, user: &str, sid: &str) {
        let key = self.keys.sess_token(user, &self.node, sid);
        if let Ok(t) = self.session.liveliness().declare_token(key).await
            && let Ok(mut g) = self.sess_tokens.lock()
        {
            g.insert(sid.to_string(), t);
        }
    }

    pub fn undeclare_sess(&self, sid: &str) {
        if let Ok(mut g) = self.sess_tokens.lock() {
            g.remove(sid);
        }
    }

    pub async fn find_sess(&self, sid: &str) -> Option<(String, String)> {
        let sel = format!("ox/{}/int/sess/*/*/{sid}", self.keys.ns);
        let replies = self.session.liveliness().get(sel).timeout(SESS_GET_TIMEOUT).await.ok()?;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some(Parsed::Sess { user, node, .. }) = self.keys.parse(s.key_expr().as_str())
            {
                return Some((user, node));
            }
        }
        None
    }

    pub async fn room_token(&self, room: &str) -> Option<(String, String)> {
        let replies = self.session.liveliness().get(self.keys.room_where(room)).timeout(SESS_GET_TIMEOUT).await.ok()?;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(s) = r.result()
                && let Some(Parsed::Room { node, inst, .. }) = self.keys.parse(s.key_expr().as_str())
            {
                return Some((node, inst));
            }
        }
        None
    }

    pub async fn session_users(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(replies) = self.session.liveliness().get(self.keys.sess_all()).timeout(SESS_GET_TIMEOUT).await {
            while let Ok(r) = replies.recv_async().await {
                if let Ok(s) = r.result()
                    && let Some(Parsed::Sess { user, .. }) = self.keys.parse(s.key_expr().as_str())
                {
                    out.push(user);
                }
            }
        }
        out
    }

    pub async fn sessions_of_user(&self, user: &str) -> Vec<(String, String)> {
        let sel = format!("ox/{}/int/sess/{user}/**", self.keys.ns);
        let mut out = Vec::new();
        if let Ok(replies) = self.session.liveliness().get(sel).timeout(SESS_GET_TIMEOUT).await {
            while let Ok(r) = replies.recv_async().await {
                if let Ok(s) = r.result()
                    && let Some(Parsed::Sess { node, sid, .. }) = self.keys.parse(s.key_expr().as_str())
                {
                    out.push((node, sid));
                }
            }
        }
        out
    }

    async fn interest(&self, which: Ev, delta: i32) {
        let (map, name, key) = match &which {
            Ev::Room(r) => (&self.room_subs, r.clone(), self.keys.ev_room(r)),
            Ev::User(u) => (&self.user_subs, u.clone(), self.keys.ev_user(u)),
        };
        if delta < 0 {
            if let Ok(mut g) = map.lock()
                && let Some(e) = g.get_mut(&name)
            {
                e.0 = e.0.saturating_sub(1);
                if e.0 == 0 {
                    g.remove(&name);
                }
            }
            return;
        }
        let bumped = map
            .lock()
            .map(|mut g| match g.get_mut(&name) {
                Some(e) => {
                    e.0 += 1;
                    true
                }
                None => false,
            })
            .unwrap_or(true);
        if bumped {
            return;
        }
        let tx = self.tx.clone();
        let n2 = name.clone();
        let is_room = matches!(which, Ev::Room(_));
        let sub = self
            .session
            .declare_subscriber(key)
            .callback(move |s| {
                if let Ok(n) = serde_json::from_slice::<Notice>(&s.payload().to_bytes()) {
                    let ev = if is_room {
                        BusEvent::RoomNotice { room: n2.clone(), notice: n }
                    } else {
                        BusEvent::UserNotice { user: n2.clone(), notice: n }
                    };
                    let _ = tx.send(ev);
                }
            })
            .await;
        if let (Ok(sub), Ok(mut g)) = (sub, map.lock()) {
            match g.get_mut(&name) {
                Some(e) => e.0 += 1,
                None => {
                    g.insert(name, (1, sub));
                }
            }
        }
    }

    pub async fn interest_room(&self, room: &str, delta: i32) {
        self.interest(Ev::Room(room.to_string()), delta).await
    }

    pub async fn interest_user(&self, user: &str, delta: i32) {
        self.interest(Ev::User(user.to_string()), delta).await
    }

    pub fn interested(&self, room: &str) -> usize {
        self.room_subs.lock().ok().and_then(|g| g.get(room).map(|e| e.0)).unwrap_or(0)
    }

    pub fn set_local(&self, l: Option<Local>) {
        if let Ok(mut g) = self.local.lock() {
            let _old = std::mem::replace(&mut *g, l);
        }
    }

    pub fn local(&self) -> Option<Local> {
        self.local.lock().ok().and_then(|g| g.clone())
    }

    pub async fn hello(
        addr: &str,
        node: &str,
        hub_inst: &str,
    ) -> Result<(HandleClient<tonic::transport::Channel>, String, String), String> {
        let mut c = HandleClient::connect(format!("http://{addr}")).await.map_err(|e| e.to_string())?;
        let r = c
            .hello(HelloReq { node: node.to_string(), hub_inst: hub_inst.to_string() })
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        Ok((c, r.epoch, r.build))
    }

    pub async fn call_local(&self, env: &Envelope) -> Reply {
        let Some(mut l) = self.local() else {
            return Reply::fail(Code::SfuUnavailable);
        };
        let fut = l.client.call(CallReq { envelope: env.encode() });
        match tokio::time::timeout(LOCAL_CALL_TIMEOUT, fut).await {
            Ok(Ok(r)) => Reply::decode(&r.into_inner().reply).unwrap_or_else(|_| Reply::fail(Code::SfuError)),
            Ok(Err(_)) => Reply::fail(Code::SfuUnavailable),
            Err(_) => Reply::fail(Code::SfuError),
        }
    }

    pub async fn call(&self, node: &str, env: &Envelope) -> Reply {
        if node == self.node {
            return self.call_local(env).await;
        }
        let live = self.view.lock().map(|v| v.node_live(node)).unwrap_or(false);
        if !live {
            return Reply::fail(Code::SfuUnavailable);
        }
        match self.query(node, "call", env.encode()).await {
            Ok(b) => Reply::decode(&b).unwrap_or_else(|_| Reply::fail(Code::SfuError)),
            Err(c) => Reply::fail(c),
        }
    }

    pub async fn query(&self, node: &str, what: &str, payload: Vec<u8>) -> Result<Vec<u8>, Code> {
        let key = self.keys.q_node(node, what);
        let replies = self.session.get(key).payload(payload).timeout(NODE_CALL_TIMEOUT).await.map_err(|_| Code::SfuError)?;
        match replies.recv_async().await {
            Ok(r) => match r.result() {
                Ok(s) => Ok(s.payload().to_bytes().to_vec()),
                Err(_) => Err(Code::SfuError),
            },
            Err(_) => Err(Code::SfuError),
        }
    }

    pub async fn reply_query(query: &Query, key: &str, payload: Vec<u8>) {
        let _ = query.reply(key, payload).await;
    }

    pub fn room_route(&self, room: &str) -> Option<view::Loc> {
        self.view.lock().ok().and_then(|v| v.room_node(room).cloned())
    }

    pub fn node_of_epoch(&self, epoch: &str) -> Option<String> {
        if let Some(l) = self.local()
            && l.epoch == epoch
        {
            return Some(self.node.clone());
        }
        self.view.lock().ok().and_then(|v| v.node_of_inst(epoch))
    }

    pub fn place(&self, room: &str) -> Option<(String, bool)> {
        let v = self.view.lock().ok()?;
        let c = v.candidates(common::now_ms());
        let n = placement::pick(c.iter().map(|s| s.as_str()), room)?;
        let live = v.node_live(&n);
        Some((n, live))
    }

    pub fn live_nodes(&self) -> Vec<(String, String)> {
        self.view.lock().map(|v| v.live_nodes()).unwrap_or_default()
    }

    pub async fn peers(&self) -> Vec<(String, String, Vec<String>)> {
        let mut links: HashMap<String, Vec<String>> = HashMap::new();
        for l in self.session.info().links().await {
            links.entry(l.zid().to_string()).or_default().push(format!("{} -> {}", l.src(), l.dst()));
        }
        let mut out = Vec::new();
        for z in self.session.info().routers_zid().await {
            let k = z.to_string();
            let l = links.get(&k).cloned().unwrap_or_default();
            out.push((k, "router".to_string(), l));
        }
        for z in self.session.info().peers_zid().await {
            let k = z.to_string();
            let l = links.get(&k).cloned().unwrap_or_default();
            out.push((k, "peer".to_string(), l));
        }
        out
    }

    pub async fn close(&self) {
        if let Ok(mut g) = self.node_token.lock() {
            g.take();
        }
        if let Ok(mut g) = self.sess_tokens.lock() {
            g.clear();
        }
        let _ = self.session.close().await;
    }
}
