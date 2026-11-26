// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-SURFACE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · 정:IMPL-SES-HANDOVER · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:IMPL-CHAT-FANOUT · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS · model: claude-opus-5-5
//! 운영 · 조율 — hub 상태 묶음 · 버스 사건(통지 · 방 소멸 · node 사이 요청)을 세션 · 방에 잇는다 · HTTP 표면 · ★방 통지 길목 한 곳에서 가른다(재료 표지는 맡은 hub 만 사실로 · 맡지 않은 hub 는 로그 · 계수 없이 버림 · A 평면 통지는 배달하고 맡은 방이면 사실 하나 더) · 방 소멸에 늘 관심을 거둔다.

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

use crate::bus::{Bus, BusEvent, Query};
use crate::hook::facts::{self, Route};
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
    pub namespace: String,
    pub shutdown: mpsc::UnboundedSender<String>,
    pub log_files: Option<std::path::PathBuf>,
    pub data_root: Option<std::path::PathBuf>,
    pub hook: Arc<crate::hook::Hook>,
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
            BusEvent::RoomNotice { room, notice } => room_notice(&hub, &room, &notice),
            BusEvent::UserNotice { user, notice } => ses::deliver_user(&hub.ses, &user, &notice).await,
            BusEvent::RoomDropped(room) => {
                ses::room_dropped(&hub.ses, &room).await;
                hub.bus.disown_room(&room).await;
            }
            BusEvent::Query(q) => {
                let hub = hub.clone();
                tokio::spawn(async move { answer(&hub, q).await });
            }
        }
    }
}

fn room_notice(hub: &Hub, room: &str, notice: &common::envelope::Notice) {
    match facts::route(notice, hub.bus.owns(room)) {
        Route::Deliver(kind) => {
            ses::deliver_room(&hub.ses, room, notice);
            if let Some(k) = kind {
                hub.hook.offer(hub.hook.build(room, k, notice.body.clone(), common::now_ms()));
            }
        }
        Route::Build(m) => {
            hub.hook.offer(hub.hook.of_material(room, &m, common::now_ms()));
        }
        Route::NotOwner => {}
        Route::Malformed => hub.bus.count_malformed(),
    }
}

async fn answer(hub: &Arc<Hub>, q: Query) {
    let what = q.key_expr().rsplit('/').next().unwrap_or("").to_string();
    let payload = q.payload().to_vec();
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
        crate::chat::FANOUT_WHAT => ses::deliver_chat(&hub.ses, &payload).await,
        common::talk::WHAT => ses::talk(&hub.ses, &payload).await,
        crate::hook::zgate::send::WHAT => ses::deliver_ext(&hub.ses, &payload).await,
        "bus" => {
            let user = String::from_utf8_lossy(&payload).to_string();
            let user = Some(user.as_str()).filter(|u| !u.is_empty());
            serde_json::to_vec(&admin::bus_view(hub, user).await).unwrap_or_default()
        }
        "history" => match serde_json::from_slice::<logger::Ask>(&payload) {
            Ok(ask) => serde_json::to_vec(&admin::history_local(hub, ask).await).unwrap_or_default(),
            Err(_) => serde_json::to_vec(&serde_json::json!({ "node": hub.node, "error": Code::InvalidPayload.num() })).unwrap_or_default(),
        },
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
