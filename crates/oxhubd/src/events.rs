// author: kodeholic (powered by Claude)
//! sfud→hub 통지 소비자 — 정§15-1 배치 학습·급사, §15-2 통지 배달. 노드마다 하나, 끊기면 백오프로 다시 붙는다.

use std::sync::Arc;
use std::time::Duration;

use common::bplane::{self, iop};
use oxsig::body::notify::{ForcedCause, RoomEvent, RoomEventType};
use oxsig::frame;
use oxsig::op::Op;
use oxsig::schema::{Affiliation, Version};
use serde_json::Value;
use tokio_stream::StreamExt;
use tracing::{info, warn};

use crate::backend::Members;
use crate::nodes::Node;
use crate::route::RoomMap;
use crate::ws::Hub;

pub struct EventCtx {
    pub hub: Arc<Hub>,
    pub rooms: Arc<RoomMap>,
    pub members: Arc<Members>,
    pub hub_id: String,
}

const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(8);

pub async fn run_consumer(ctx: Arc<EventCtx>, node: Arc<Node>) {
    let mut backoff = BACKOFF_MIN;
    loop {
        let Some(mut client) = node.client().await else {
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(BACKOFF_MAX);
            continue;
        };
        match client.subscribe(bplane::SubscribeRequest { hub_id: ctx.hub_id.clone() }).await {
            Ok(resp) => {
                info!(node = %node.id, "event stream up");
                backoff = BACKOFF_MIN;
                let mut stream = resp.into_inner();
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(env) => dispatch(&ctx, &node.id, env),
                        Err(e) => {
                            warn!(node = %node.id, error = %e, "event stream error");
                            break;
                        }
                    }
                }
                node.drop_client().await;
                node_down(&ctx, &node.id);
            }
            Err(e) => {
                warn!(node = %node.id, error = %e, "subscribe failed");
                node.drop_client().await;
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// 정§15-1 급사 — 그 노드의 배치 전량을 풀고 입장자 전원에게 `ROOM_EVENT{affiliation, cause:"room_closed"}`.
pub fn node_down(ctx: &EventCtx, node_id: &str) {
    let closed = ctx.rooms.unbind_node(node_id);
    warn!(node = node_id, rooms = closed.len(), "node down — rooms closed");
    for (room_id, cursor) in closed {
        let ev = RoomEvent {
            event_type: RoomEventType::Affiliation,
            room_id: room_id.clone(),
            version: cursor.unwrap_or(Version { epoch: node_id.to_owned(), seq: 0 }),
            affiliation: Some(Affiliation { sub_rooms: Vec::new(), pub_room: None }),
            cause: Some(ForcedCause::RoomClosed),
            reason: None,
        };
        let body = serde_json::to_value(&ev).unwrap_or(Value::Null);
        for user in ctx.members.forget_room(&room_id) {
            ctx.hub.notify_user_json(&user, Op::RoomEvent, &body);
        }
    }
}

/// 통지 하나 — 내부 op 는 hub 가 먹고, 클라 op 는 대상에게 배달한다(wire 그대로).
pub fn dispatch(ctx: &EventCtx, node_id: &str, env: bplane::Envelope) {
    let Ok((h, body)) = frame::decode(&env.wire) else {
        warn!(node = node_id, "event wire undecodable");
        return;
    };
    if h.op == iop::ROOM_LIFECYCLE {
        let Ok(b) = frame::body_json(body) else { return };
        match b["type"].as_str() {
            Some("created") => {
                if let Some(prev) = ctx.rooms.learn(&env.room_id, node_id) {
                    warn!(room = %env.room_id, node = node_id, prev = %prev, "[ROOM:CONFLICT] keeping the first");
                }
            }
            Some("destroyed") => {
                ctx.rooms.unbind(&env.room_id);
                ctx.members.forget_room(&env.room_id);
            }
            _ => {}
        }
        return;
    }
    if iop::is_internal(h.op) {
        return;
    }
    if let Some(op) = Op::from_code(h.op)
        && op.is_notification()
        && let Ok(b) = frame::body_json(body)
        && let Ok(v) = serde_json::from_value::<Version>(b["version"].clone())
    {
        ctx.rooms.touch(&env.room_id, v);
    }
    if !env.target.is_empty() {
        ctx.hub.notify_user(&env.target, h.op, body.to_vec());
        return;
    }
    for user in ctx.members.members(&env.room_id) {
        if !env.exclude.contains(&user) {
            ctx.hub.notify_user(&user, h.op, body.to_vec());
        }
    }
}
