// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · 정:CAND-BUS-NO-ZENOH-BOOT · 정:CAND-BUS-NOTICE-ENVELOPE · model: claude-opus-5-5
//! sfud 의 버스 — zenoh client(로컬 hub 하나에만 붙는다). `room` 토큰은 sfud 가 선언(수명 = sfud 세션) · `ev` put 은 낸 차례 그대로 한 줄로(`Drop` · 데이터 레인) · `node` 토큰은 `history` 로 본다(홈 잃음).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use common::bus::keys::KeySpace;
use tokio::sync::mpsc;
use zenoh::bytes::ZBytes;
use zenoh::handlers::FifoChannelHandler;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Subscriber;
use zenoh::qos::{CongestionControl, Priority};
use zenoh::sample::Sample;

pub struct Bus {
    pub ks: KeySpace,
    pub z: zenoh::Session,
    pub node: String,
    pub inst: String,
    tokens: Mutex<HashMap<String, LivelinessToken>>,
    tx: mpsc::UnboundedSender<(String, Vec<u8>)>,
    pub dropped: AtomicU64,
}

impl Bus {
    pub async fn open(connect: &str, namespace: &str, node: &str, inst: &str) -> Result<std::sync::Arc<Bus>, String> {
        let zc = common::bus::zcfg::client(connect)?;
        let z = zenoh::open(zc).await.map_err(|e| format!("zenoh open: {e}"))?;
        let (tx, mut rx) = mpsc::unbounded_channel::<(String, Vec<u8>)>();
        let bus = std::sync::Arc::new(Bus { ks: KeySpace::new(namespace), z, node: node.to_string(), inst: inst.to_string(), tokens: Mutex::new(HashMap::new()), tx, dropped: AtomicU64::new(0) });
        let b = bus.clone();
        tokio::spawn(async move {
            while let Some((key, bytes)) = rx.recv().await {
                let r = b.z.put(&key, ZBytes::from(bytes)).congestion_control(CongestionControl::Drop).priority(Priority::Data).await;
                if r.is_err() {
                    b.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok(bus)
    }

    pub fn put(&self, key: String, bytes: Vec<u8>) {
        if self.tx.send((key, bytes)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn put_room(&self, room: &str, bytes: Vec<u8>) {
        self.put(self.ks.ev_room(room), bytes);
    }

    pub fn put_user(&self, user: &str, bytes: Vec<u8>) {
        self.put(self.ks.ev_user(user), bytes);
    }

    pub async fn declare_room(&self, room: &str) -> Result<(), String> {
        let key = self.ks.room(room, &self.node, &self.inst);
        let tok = self.z.liveliness().declare_token(&key).await.map_err(|e| format!("token {key}: {e}"))?;
        if let Ok(mut t) = self.tokens.lock() {
            t.insert(room.to_string(), tok);
        }
        Ok(())
    }

    pub fn undeclare_room(&self, room: &str) {
        let tok = self.tokens.lock().ok().and_then(|mut t| t.remove(room));
        drop(tok);
    }

    pub fn declared(&self) -> Vec<String> {
        self.tokens.lock().map(|t| t.keys().cloned().collect()).unwrap_or_default()
    }

    pub async fn watch_nodes(&self) -> Result<Subscriber<FifoChannelHandler<Sample>>, String> {
        let key = self.ks.node_all();
        self.z.liveliness().declare_subscriber(&key).history(true).await.map_err(|e| format!("watch {key}: {e}"))
    }

    pub async fn close(&self) {
        if let Ok(mut t) = self.tokens.lock() {
            t.clear();
        }
        let _ = self.z.close().await;
    }
}
