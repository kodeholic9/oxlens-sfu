// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · model: claude-opus-5-5
//! sfud 의 버스 — zenoh client(로컬 hub 에만) · `room` 토큰 선언 · `ev` put 은 낸 차례 그대로 한 줄로(`Drop`) · `node` 토큰 history 구독(홈 잃음).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use common::envelope::Notice;
use common::keyspace::{Keys, Parsed};
use tokio::sync::mpsc;
use zenoh::Session;
use zenoh::liveliness::LivelinessToken;
use zenoh::qos::CongestionControl;

static BUS: OnceLock<Arc<Bus>> = OnceLock::new();

enum Put {
    Room(String, Notice),
    User(String, Notice),
}

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub epoch: String,
    session: Session,
    tx: mpsc::UnboundedSender<Put>,
    room_tokens: Mutex<HashMap<String, LivelinessToken>>,
    pub nodes: Mutex<HashMap<String, (String, bool, u64)>>,
    pub seen_other: Mutex<bool>,
    pub ev_drop: std::sync::atomic::AtomicU64,
    _subs: Mutex<Vec<zenoh::pubsub::Subscriber<()>>>,
}

pub fn get() -> Option<&'static Arc<Bus>> {
    BUS.get()
}

impl Bus {
    pub async fn open(node: &str, epoch: &str, ns: &str, connect: &str) -> Result<Arc<Bus>, String> {
        let mut cfg = zenoh::Config::default();
        let set = |cfg: &mut zenoh::Config, k: &str, v: String| cfg.insert_json5(k, &v).map_err(|e| format!("zenoh {k}: {e}"));
        set(&mut cfg, "mode", "\"client\"".into())?;
        set(&mut cfg, "connect/endpoints", serde_json::to_string(&vec![connect]).unwrap_or_default())?;
        set(&mut cfg, "scouting/multicast/enabled", "false".into())?;
        set(&mut cfg, "transport/unicast/qos/enabled", "true".into())?;
        set(&mut cfg, "transport/unicast/lowlatency", "false".into())?;
        let session = zenoh::open(cfg).await.map_err(|e| format!("zenoh open: {e}"))?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Put>();
        let bus = Arc::new(Bus {
            keys: Keys::new(ns),
            node: node.to_string(),
            epoch: epoch.to_string(),
            session,
            tx,
            room_tokens: Mutex::new(HashMap::new()),
            nodes: Mutex::new(HashMap::new()),
            seen_other: Mutex::new(false),
            ev_drop: std::sync::atomic::AtomicU64::new(0),
            _subs: Mutex::new(Vec::new()),
        });
        let b2 = bus.clone();
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                let (key, n) = match p {
                    Put::Room(r, n) => (b2.keys.ev_room(&r), n),
                    Put::User(u, n) => (b2.keys.ev_user(&u), n),
                };
                let payload = serde_json::to_vec(&n).unwrap_or_default();
                if b2.session.put(key, payload).congestion_control(CongestionControl::Drop).await.is_err() {
                    b2.ev_drop.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        let me = Arc::downgrade(&bus);
        let keys = bus.keys.clone();
        let sub = bus
            .session
            .liveliness()
            .declare_subscriber(keys.node_all())
            .history(true)
            .callback(move |s| {
                if let (Some(b), Some(Parsed::Node { node, inst })) = (me.upgrade(), keys.parse(s.key_expr().as_str())) {
                    let put = s.kind() == zenoh::sample::SampleKind::Put;
                    if let Ok(mut g) = b.nodes.lock() {
                        if node != b.node && put && let Ok(mut so) = b.seen_other.lock() {
                            let _ = std::mem::replace(&mut *so, true);
                        }
                        g.insert(format!("{node}/{inst}"), (node, put, common::now_ms()));
                    }
                }
            })
            .await
            .map_err(|e| format!("node sub: {e}"))?;
        if let Ok(mut g) = bus._subs.lock() {
            g.push(sub);
        }
        let _ = BUS.set(bus.clone());
        Ok(bus)
    }

    pub fn room_notice(&self, room: &str, n: Notice) {
        let _ = self.tx.send(Put::Room(room.to_string(), n));
    }

    pub fn user_notice(&self, user: &str, n: Notice) {
        let _ = self.tx.send(Put::User(user.to_string(), n));
    }

    pub async fn declare_room(&self, room: &str) {
        let key = self.keys.room_token(room, &self.node, &self.epoch);
        match self.session.liveliness().declare_token(key).await {
            Ok(t) => {
                if let Ok(mut g) = self.room_tokens.lock() {
                    g.insert(room.to_string(), t);
                }
            }
            Err(e) => tracing::warn!(kind = "diag", room = %room, "room token: {e}"),
        }
    }

    pub fn undeclare_room(&self, room: &str) {
        if let Ok(mut g) = self.room_tokens.lock() {
            g.remove(room);
        }
    }

    pub fn home_alive(&self, node: &str, inst: &str) -> Option<(bool, u64)> {
        let g = self.nodes.lock().ok()?;
        g.get(&format!("{node}/{inst}")).map(|(_, live, at)| (*live, *at))
    }

    pub fn isolated(&self) -> bool {
        let seen = self.seen_other.lock().map(|g| *g).unwrap_or(false);
        if !seen {
            return false;
        }
        let g = match self.nodes.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        !g.values().any(|(n, live, _)| n != &self.node && *live)
    }

    pub fn live_nodes(&self) -> usize {
        let g = match self.nodes.lock() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let s: HashSet<&String> = g.values().filter(|(_, l, _)| *l).map(|(n, _, _)| n).collect();
        s.len()
    }

    pub async fn close(&self) {
        if let Ok(mut g) = self.room_tokens.lock() {
            g.clear();
        }
        let _ = self.session.close().await;
    }
}

pub fn room_notice(room: &str, n: Notice) {
    if let Some(b) = get() {
        b.room_notice(room, n);
    }
}

pub fn user_notice(user: &str, n: Notice) {
    if let Some(b) = get() {
        b.user_notice(user, n);
    }
}
