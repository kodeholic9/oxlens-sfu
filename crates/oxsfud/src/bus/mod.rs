// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:RULE-BUS-KEYSPACE 정:RULE-BUS-PRESENCE 정:RULE-BUS-EVENTS 정:IMPL-BUS-HOME-LOST 정:IMPL-BUS-FAULTS 정:CAND-BUS-NOTICE-ENVELOPE 정:CAND-BUS-HOME-INST 정:CAND-BUS-NO-ZENOH-BOOT 정:RULE-CONC-SIX · model: claude-opus-5-5
//! sfud 쪽 버스 — zenoh client · `room` 토큰 선언(주체 = sfud · inst = epoch) · `ev` put(Drop · 한 줄 차례) · `node` 토큰 `history` 구독으로 홈 생존 · 고립 판정.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::bus::{Keys, ZenohOpts};
use oxsig::envelope::Notice;
use tokio::sync::{Mutex, mpsc};
use zenoh::liveliness::LivelinessToken;
use zenoh::qos::CongestionControl;
use zenoh::sample::SampleKind;

pub const PUT_QUEUE: usize = 65_536;

fn err_s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Default)]
struct View {
    alive: HashMap<String, BTreeSet<String>>,
    missing_since: HashMap<(String, String), Instant>,
    others_seen: bool,
    isolated: bool,
    nodes_ever: BTreeSet<String>,
}

pub struct Bus {
    z: zenoh::Session,
    pub keys: Keys,
    pub node: String,
    pub epoch: String,
    pub connect: String,
    tx: mpsc::Sender<(String, Vec<u8>)>,
    room_tokens: Mutex<HashMap<String, LivelinessToken>>,
    view: std::sync::Mutex<View>,
    pub put_drop: AtomicU64,
    pub put_sent: AtomicU64,
}

pub enum HomeState {
    Alive,
    Missing,
    Lost,
    Deferred,
}

impl Bus {
    pub async fn open(connect: &str, ns: &str, node: &str, epoch: &str) -> Result<Arc<Bus>, String> {
        let conn = vec![connect.to_string()];
        let zc = common::bus::zenoh_config(&ZenohOpts { mode: "client", listen: &[], connect: &conn, multicast_scouting: false, base_config: None })?;
        let z = zenoh::open(zc).await.map_err(err_s)?;
        let (tx, mut rx) = mpsc::channel::<(String, Vec<u8>)>(PUT_QUEUE);
        let bus = Arc::new(Bus {
            z,
            keys: Keys::new(ns),
            node: node.to_string(),
            epoch: epoch.to_string(),
            connect: connect.to_string(),
            tx,
            room_tokens: Mutex::new(HashMap::new()),
            view: std::sync::Mutex::new(View::default()),
            put_drop: AtomicU64::new(0),
            put_sent: AtomicU64::new(0),
        });
        let b2 = bus.clone();
        tokio::spawn(async move {
            while let Some((k, v)) = rx.recv().await {
                if b2.z.put(&k, v).congestion_control(CongestionControl::Drop).await.is_ok() {
                    b2.put_sent.fetch_add(1, Ordering::Relaxed);
                } else {
                    b2.put_drop.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        let sub = bus.z.liveliness().declare_subscriber(bus.keys.node_all()).history(true).await.map_err(err_s)?;
        let b3 = bus.clone();
        tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                b3.on_node(s.key_expr().as_str(), s.kind());
            }
        });
        Ok(bus)
    }

    fn on_node(&self, key: &str, kind: SampleKind) {
        let Some(t) = self.keys.tail(key, "node") else { return };
        if t.len() != 2 {
            return;
        }
        let (n, i) = (t[0].to_string(), t[1].to_string());
        let mut v = self.view.lock().unwrap_or_else(|p| p.into_inner());
        v.nodes_ever.insert(n.clone());
        if kind == SampleKind::Put {
            v.alive.entry(n.clone()).or_default().insert(i.clone());
            v.missing_since.remove(&(n.clone(), i.clone()));
        } else {
            if let Some(s) = v.alive.get_mut(&n) {
                s.remove(&i);
            }
            v.missing_since.entry((n.clone(), i.clone())).or_insert_with(Instant::now);
        }
        let others = v.alive.iter().filter(|(k, s)| **k != self.node && !s.is_empty()).count();
        if others > 0 {
            v.others_seen = true;
            v.isolated = false;
        } else if v.others_seen && kind == SampleKind::Delete && n != self.node {
            v.isolated = true;
        }
        drop(v);
        tracing::info!("node token {} node={n} inst={i}", if kind == SampleKind::Put { "put" } else { "delete" });
    }

    pub fn home(&self, node: &str, inst: &str, grace: Duration) -> HomeState {
        let mut v = self.view.lock().unwrap_or_else(|p| p.into_inner());
        if v.alive.get(node).is_some_and(|s| s.contains(inst)) {
            return HomeState::Alive;
        }
        let since = *v.missing_since.entry((node.to_string(), inst.to_string())).or_insert_with(Instant::now);
        if since.elapsed() < grace {
            return HomeState::Missing;
        }
        if v.isolated || v.nodes_ever.len() <= 2 {
            return HomeState::Deferred;
        }
        HomeState::Lost
    }

    pub fn isolated(&self) -> bool {
        self.view.lock().map(|v| v.isolated).unwrap_or(false)
    }

    pub fn nodes(&self) -> Vec<(String, Vec<String>)> {
        self.view.lock().map(|v| v.alive.iter().map(|(k, s)| (k.clone(), s.iter().cloned().collect())).collect()).unwrap_or_default()
    }

    pub async fn declare_room(&self, room: &str) -> Result<(), String> {
        let mut g = self.room_tokens.lock().await;
        if g.contains_key(room) {
            return Ok(());
        }
        let t = self.z.liveliness().declare_token(self.keys.room(room, &self.node, &self.epoch)).await.map_err(err_s)?;
        g.insert(room.to_string(), t);
        Ok(())
    }

    pub async fn undeclare_room(&self, room: &str) {
        let t = self.room_tokens.lock().await.remove(room);
        if let Some(t) = t {
            let _ = t.undeclare().await;
        }
    }

    fn put(&self, key: String, n: &Notice) {
        let Ok(v) = serde_json::to_vec(n) else { return };
        if self.tx.try_send((key, v)).is_err() {
            self.put_drop.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn room_event(&self, room: &str, op: u16, body: String, exclude: Vec<String>) {
        let n = Notice { op, body, exclude, room_id: Some(room.to_string()), sfu_id: Some(self.epoch.clone()) };
        self.put(self.keys.ev_room(room), &n);
    }

    pub fn user_event(&self, user: &str, op: u16, body: String, room: Option<&str>) {
        let n = Notice { op, body, exclude: Vec::new(), room_id: room.map(str::to_string), sfu_id: Some(self.epoch.clone()) };
        self.put(self.keys.ev_user(user), &n);
    }

    pub async fn close(&self) {
        let tokens: Vec<LivelinessToken> = self.room_tokens.lock().await.drain().map(|(_, t)| t).collect();
        for t in tokens {
            let _ = t.undeclare().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = self.z.close().await;
    }
}
