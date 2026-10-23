// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · 정:RULE-BUS-FATE · model: claude-opus-5-5
//! BUS(sfud) — zenoh client 하나: `room` 토큰 선언(현재 상태에서 전량) · `ev` put(데이터 레인 · `Drop` · 한 줄로 순서 유지) · `node` 토큰 history 구독(홈을 잃은 Peer — 고립이면 거두지 않는다).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use common::bus::{BusConf, Keys, Notice};
use tokio::sync::mpsc;
use zenoh::liveliness::LivelinessToken;
use zenoh::qos::CongestionControl;
use zenoh::sample::SampleKind;

pub const HOME_GRACE_MS: u64 = 3000;

pub type Lost = Arc<dyn Fn(String, String, Option<String>) + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub inst: String,
    session: Option<zenoh::Session>,
    tx: mpsc::UnboundedSender<(String, Vec<u8>)>,
    tokens: Mutex<BTreeMap<String, LivelinessToken>>,
    nodes: Mutex<BTreeMap<String, BTreeSet<String>>>,
    lost: OnceLock<Lost>,
    pub dropped: std::sync::atomic::AtomicU64,
}

impl Bus {
    pub async fn open(node: &str, inst: &str, ns: &str, connect: Option<&str>) -> Result<Arc<Bus>, String> {
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, Vec<u8>)>();
        let session = match connect {
            Some(c) => {
                let conn = vec![c.to_string()];
                Some(common::bus::open(&BusConf { mode: "client", listen: &[], connect: &conn, multicast_scouting: false }).await?)
            }
            None => None,
        };
        let bus = Arc::new(Bus {
            keys: Keys::new(ns),
            node: node.to_string(),
            inst: inst.to_string(),
            session,
            tx,
            tokens: Mutex::new(BTreeMap::new()),
            nodes: Mutex::new(BTreeMap::new()),
            lost: OnceLock::new(),
            dropped: std::sync::atomic::AtomicU64::new(0),
        });
        let b2 = bus.clone();
        tokio::spawn(async move {
            while let Some((k, v)) = rx.recv().await {
                let Some(s) = &b2.session else { continue };
                if s.put(k, v).congestion_control(CongestionControl::Drop).await.is_err() {
                    b2.dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        });
        bus.watch_nodes().await;
        Ok(bus)
    }

    pub fn set_lost(&self, f: Lost) {
        let _ = self.lost.set(f);
    }

    pub fn put_room(&self, room: &str, n: Notice) {
        let k = self.keys.ev_room(room);
        if let Ok(b) = serde_json::to_vec(&n) {
            let _ = self.tx.send((k, b));
        }
    }

    pub fn put_user(&self, user: &str, n: Notice) {
        let k = self.keys.ev_user(user);
        if let Ok(b) = serde_json::to_vec(&n) {
            let _ = self.tx.send((k, b));
        }
    }

    pub async fn declare_room(&self, room: &str) -> bool {
        let Some(s) = &self.session else { return true };
        if lock(&self.tokens).contains_key(room) {
            return true;
        }
        let key = self.keys.room_token(room, &self.node, &self.inst);
        match s.liveliness().declare_token(key).await {
            Ok(t) => {
                lock(&self.tokens).insert(room.to_string(), t);
                true
            }
            Err(e) => {
                tracing::warn!(kind = "diag", room = %room, err = %e, "room 토큰 선언 실패");
                false
            }
        }
    }

    pub fn undeclare_room(&self, room: &str) {
        lock(&self.tokens).remove(room);
    }

    pub fn rooms(&self) -> Vec<String> {
        lock(&self.tokens).keys().cloned().collect()
    }

    pub fn live_nodes(&self) -> BTreeMap<String, BTreeSet<String>> {
        lock(&self.nodes).clone()
    }

    async fn watch_nodes(self: &Arc<Self>) {
        let Some(s) = &self.session else { return };
        let Ok(sub) = s.liveliness().declare_subscriber(self.keys.node_all()).history(true).await else { return };
        let me = self.clone();
        tokio::spawn(async move {
            while let Ok(smp) = sub.recv_async().await {
                let key = smp.key_expr().as_str().to_string();
                let Some(parts) = me.keys.parse(&key) else { continue };
                let [k, node, inst] = parts.as_slice() else { continue };
                if k != "node" {
                    continue;
                }
                if smp.kind() == SampleKind::Put {
                    lock(&me.nodes).entry(node.clone()).or_default().insert(inst.clone());
                    continue;
                }
                let before = {
                    let mut g = lock(&me.nodes);
                    let before = g.len();
                    if let Some(set) = g.get_mut(node) {
                        set.remove(inst);
                        if set.is_empty() {
                            g.remove(node);
                        }
                    }
                    before
                };
                let me2 = me.clone();
                let (node, inst) = (node.clone(), inst.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(HOME_GRACE_MS)).await;
                    me2.judge(&node, &inst, before);
                });
            }
        });
    }

    fn judge(&self, node: &str, inst: &str, before: usize) {
        let g = lock(&self.nodes).clone();
        if g.get(node).map(|s| s.contains(inst)).unwrap_or(false) {
            return;
        }
        let Some(f) = self.lost.get() else { return };
        if let Some(new) = g.get(node).and_then(|s| s.iter().next()) {
            f(node.to_string(), inst.to_string(), Some(new.clone()));
            return;
        }
        let others = g.keys().filter(|n| n.as_str() != self.node).count();
        if others == 0 || before <= 2 {
            tracing::info!(kind = "diag", node = %node, inst = %inst, others, before, "홈 토큰 소멸 — 고립 · 둘뿐이라 거두지 않는다");
            return;
        }
        f(node.to_string(), inst.to_string(), None);
    }
}
