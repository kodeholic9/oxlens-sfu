// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-SURFACE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:IMPL-SES-HANDOVER · 정:CAND-DOC-ORCHESTRATION-ASPECT · model: claude-opus-5-5
//! 운영 · 조율 — hub 상태 묶음 · 버스 사건(통지 · 방 소멸 · node 사이 요청)을 세션 · 방에 잇는다 · HTTP 표면.

pub mod admin;
pub mod http;
pub mod logger;
pub mod supervisor;

use std::sync::Arc;

use common::envelope::{Envelope, Reply};
use common::log::Sender;
use common::policy::Policy;
use oxsig::Code;
use tokio::sync::mpsc;

use crate::bus::{Bus, BusEvent};
use crate::ses;
use supervisor::Supervisor;

pub struct Hub {
    pub node: String,
    pub inst: String,
    pub build: String,
    pub base_path: String,
    pub policy: Policy,
    pub ses: Arc<ses::Ctx>,
    pub bus: Arc<Bus>,
    pub sup: Arc<Supervisor>,
    pub sender: Arc<Sender>,
    pub mode: String,
    pub namespace: String,
    pub shutdown: mpsc::UnboundedSender<String>,
}

impl Hub {
    pub fn ready(&self) -> (bool, Vec<String>) {
        let down = self.sup.down_units();
        (down.is_empty(), down)
    }
}

pub async fn dispatch(hub: Arc<Hub>, mut rx: mpsc::UnboundedReceiver<BusEvent>) {
    while let Some(ev) = rx.recv().await {
        match ev {
            BusEvent::RoomNotice { room, notice } => ses::deliver_room(&hub.ses, &room, &notice),
            BusEvent::UserNotice { user, notice } => ses::deliver_user(&hub.ses, &user, &notice).await,
            BusEvent::RoomDropped(room) => ses::room_dropped(&hub.ses, &room).await,
            BusEvent::Query(q) => {
                let hub = hub.clone();
                tokio::spawn(async move { answer(&hub, q).await });
            }
        }
    }
}

async fn answer(hub: &Arc<Hub>, q: zenoh::query::Query) {
    let what = q.key_expr().as_str().rsplit('/').next().unwrap_or("").to_string();
    let payload = q.payload().map(|p| p.to_bytes().to_vec()).unwrap_or_default();
    let key = hub.bus.keys.q_node(&hub.node, &what);
    let out: Vec<u8> = match what.as_str() {
        "call" => match Envelope::decode(&payload) {
            Ok(env) => hub.bus.call_local(&env).await.encode(),
            Err(_) => Reply::fail(Code::InvalidPayload).encode(),
        },
        "sess" => {
            let sid = serde_json::from_slice::<serde_json::Value>(&payload)
                .ok()
                .and_then(|v| v.get("sid").and_then(|s| s.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            match ses::hand_over(&hub.ses, &sid).await {
                Some(d) => serde_json::to_vec(&d).unwrap_or_default(),
                None => b"null".to_vec(),
            }
        }
        "evict" => {
            let sid = serde_json::from_slice::<serde_json::Value>(&payload)
                .ok()
                .and_then(|v| v.get("sid").and_then(|s| s.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            ses::evict_sid(&hub.ses, &sid).await;
            b"{}".to_vec()
        }
        "cut" => {
            let user = String::from_utf8_lossy(&payload).to_string();
            serde_json::to_vec(&ses::cut(&hub.ses, &user).await).unwrap_or_default()
        }
        "probe" => {
            let user = String::from_utf8_lossy(&payload).to_string();
            serde_json::to_vec(&admin::probe_local(hub, &user).await).unwrap_or_default()
        }
        "bus" => serde_json::to_vec(&admin::bus_view(hub).await).unwrap_or_default(),
        _ => Reply::fail(Code::UnknownOp).encode(),
    };
    Bus::reply_query(&q, &key, out).await;
}

pub async fn sweeper(hub: Arc<Hub>) {
    let mut t = tokio::time::interval(std::time::Duration::from_millis(1000));
    loop {
        t.tick().await;
        ses::sweep(&hub.ses).await;
        if let Some((_, until)) = hub.sender.level_override()
            && common::now_ms() >= until
        {
            let _ = hub.sender.level(common::now_ms());
        }
    }
}
