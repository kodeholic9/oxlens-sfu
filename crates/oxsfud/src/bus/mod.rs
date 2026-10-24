// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-FATE · 정:IMPL-BUS-FAULTS · 정:IMPL-BUS-HOME-LOST · 정:CAND-BUS-NOTICE-ENVELOPE · 정:CAND-BUS-NO-ZENOH-BOOT · model: claude-opus-5-5
//! 영역 BUS(미디어 서버 쪽) — zenoh client(로컬 hub 하나) · `room` 토큰 선언 · 통지 put(혼잡 Drop · 한 줄로 차례 보존) · node 토큰 구독(홈 판정).

pub mod core;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::bus::envelope::Notice;
use common::bus::keys::Keys;
use tokio::sync::mpsc;
use zenoh::liveliness::LivelinessToken;
use zenoh::qos::CongestionControl;
use zenoh::sample::SampleKind;

use self::core::{Nodes, Verdict};

pub const PUT_QUEUE: usize = 65_536;

pub struct Bus {
    pub keys: Keys,
    pub node: String,
    pub inst: String,
    session: zenoh::Session,
    rooms: tokio::sync::Mutex<HashMap<String, LivelinessToken>>,
    puts: mpsc::Sender<(String, Vec<u8>)>,
    pub nodes: Mutex<Nodes>,
    pub dropped: std::sync::atomic::AtomicU64,
}

impl Bus {
    pub async fn open(connect: &str, namespace: &str, node: &str, inst: &str) -> Result<Arc<Bus>, String> {
        let cfg = common::bus::session::client_config(connect).map_err(|e| e.to_string())?;
        let session = common::bus::session::open(cfg).await.map_err(|e| e.to_string())?;
        let (tx, mut rx) = mpsc::channel::<(String, Vec<u8>)>(PUT_QUEUE);
        let bus = Arc::new(Bus {
            keys: Keys::new(namespace),
            node: node.to_string(),
            inst: inst.to_string(),
            session,
            rooms: tokio::sync::Mutex::new(HashMap::new()),
            puts: tx,
            nodes: Mutex::new(Nodes::new(node)),
            dropped: std::sync::atomic::AtomicU64::new(0),
        });
        let s = bus.session.clone();
        tokio::spawn(async move {
            while let Some((key, body)) = rx.recv().await {
                if let Err(e) = s.put(&key, body).congestion_control(CongestionControl::Drop).await {
                    tracing::warn!(kind = "diag", key = %key, "ev put: {e}");
                }
            }
        });
        Ok(bus)
    }

    pub async fn declare_room(&self, room: &str) {
        let key = self.keys.room(room, &self.node, &self.inst);
        match self.session.liveliness().declare_token(&key).await {
            Ok(t) => {
                self.rooms.lock().await.insert(room.to_string(), t);
                tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", room = %room, key = %key, "room token declared");
            }
            Err(e) => tracing::warn!(kind = "diag", room = %room, "room token: {e}"),
        }
    }

    pub async fn undeclare_room(&self, room: &str) {
        if let Some(t) = self.rooms.lock().await.remove(room) {
            let _ = t.undeclare().await;
            tracing::info!(kind = "ev", spec = "RULE-BUS-PRESENCE", room = %room, "room token withdrawn");
        }
    }

    fn enqueue(&self, key: String, n: &Notice) {
        if self.puts.try_send((key, n.encode())).is_err() {
            self.dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn to_room(&self, room: &str, n: &Notice) {
        self.enqueue(self.keys.ev_room(room), n);
    }

    pub fn to_user(&self, user: &str, n: &Notice) {
        self.enqueue(self.keys.ev_user(user), n);
    }

    pub async fn watch_nodes(self: &Arc<Self>, out: mpsc::UnboundedSender<Verdict>) -> Result<(), String> {
        let sub = self.session.liveliness().declare_subscriber(self.keys.node_all()).history(true).await.map_err(|e| e.to_string())?;
        let b = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Ok(s) = sub.recv_async().await {
                let Some(bus) = b.upgrade() else { break };
                let Some((node, inst)) = bus.keys.parse_node(s.key_expr().as_str()) else { continue };
                let now = common::time::mono_ms();
                let mut n = bus.nodes.lock().unwrap_or_else(|p| p.into_inner());
                if s.kind() == SampleKind::Put {
                    n.put(&node, &inst);
                } else {
                    n.delete(&node, &inst, now);
                }
            }
        });
        let b = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(std::time::Duration::from_millis(500));
            loop {
                iv.tick().await;
                let Some(bus) = b.upgrade() else { break };
                let verdicts = bus.nodes.lock().unwrap_or_else(|p| p.into_inner()).judge(common::time::mono_ms());
                for v in verdicts {
                    let _ = out.send(v);
                }
            }
        });
        Ok(())
    }

    pub async fn close(&self) {
        self.rooms.lock().await.clear();
        let _ = self.session.close().await;
    }
}
