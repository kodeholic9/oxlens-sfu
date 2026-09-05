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
        // ★값을 지어내지 않는다 — `node_id` 를 epoch 자리에 넣으면 그것이 `sfu_id` 인 척한다(연§4-6).
        // 커서가 없으면 빈 epoch 다: 클라는 규칙 1 로 그 방 보관본을 버리는데, 어차피 없어질 방이라 같은 결말이다.
        let ev = RoomEvent {
            event_type: RoomEventType::Affiliation,
            room_id: room_id.clone(),
            version: cursor.unwrap_or(Version { epoch: String::new(), seq: 0 }),
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
    // ★커서는 여기서 채워진다 — target·exclude 를 가르기 **전에** 찍는다.
    // 입장 직후 혼자인 방의 `PARTICIPANT_EVENT` 는 자기 제외라 아무에게도 안 가지만 version 은 지나간다.
    // 급사 통지가 실을 값이 그것이다(정§15-1) — 순서를 뒤집으면 조용한 방의 커서가 빈다.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::NoBackend;
    use crate::session::SessionRegistry;
    use crate::ws::ConnHandle;
    use common::auth;
    use oxsig::body::session::BindReq;
    use oxsig::schema::PcMode;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc;

    type Inbox = mpsc::Receiver<(u16, Vec<u8>)>;

    struct Rig {
        ctx: Arc<EventCtx>,
        inbox: Vec<(String, Inbox)>,
    }

    fn rig(users: &[&str]) -> Rig {
        let registry = Arc::new(SessionRegistry::new("s", Duration::from_secs(60), 10_000));
        let hub = Arc::new(Hub::new(registry.clone(), Arc::new(NoBackend), 4, Duration::from_secs(30)));
        let mut inbox = Vec::new();
        for (i, u) in users.iter().enumerate() {
            let conn_id = i as u64 + 1;
            let token = auth::issue("s", u, auth::ROLE_USER, 3, None, 60, auth::now_unix()).unwrap().token;
            let req = BindReq { token, session_id: None, client_ver: 1, pc_mode: PcMode::TwoPc };
            registry.bind(&req, conn_id, Instant::now()).unwrap();
            let (notify, rx) = mpsc::channel(8);
            let (close, _c) = mpsc::channel(1);
            hub.conns.insert(conn_id, ConnHandle { notify, close });
            inbox.push(((*u).to_owned(), rx));
        }
        let ctx = Arc::new(EventCtx {
            hub,
            rooms: Arc::new(RoomMap::default()),
            members: Arc::new(Members::default()),
            hub_id: "hub-1".to_owned(),
        });
        Rig { ctx, inbox }
    }

    fn drain(rx: &mut Inbox) -> Vec<(u16, Value)> {
        let mut out = Vec::new();
        while let Ok((op, body)) = rx.try_recv() {
            out.push((op, serde_json::from_slice(&body).unwrap_or(Value::Null)));
        }
        out
    }

    fn note(room: &str, op: Op, body: Value, exclude: &[&str]) -> bplane::Envelope {
        bplane::Envelope {
            session_id: String::new(),
            user_id: String::new(),
            room_id: room.to_owned(),
            target: String::new(),
            exclude: exclude.iter().map(|s| (*s).to_owned()).collect(),
            wire: oxsig::frame::encode_json(&oxsig::frame::Header::msg(op.code(), 0), &body),
            pc_mode: String::new(),
            floor_priority: 0,
        }
    }

    /// ★급사 통지가 실을 값이 어디서 오는지 — `dispatch` 가 **가르기 전에** 찍는 커서다.
    /// 입장 직후 혼자인 방의 통지는 자기 제외라 아무에게도 안 가는데, 그것마저 안 찍으면
    /// 그 방 급사 통지에 실을 값이 없다. 순서를 뒤집으면 조용한 방이 빈 값으로 닫힌다.
    #[test]
    fn a_notification_delivered_to_nobody_still_sets_the_cursor() {
        let mut r = rig(&["u1"]);
        let c = r.ctx.clone();
        c.rooms.assign("r1", "sfu-1");
        c.members.join("r1", "u1");
        let body = serde_json::json!({
            "type": "joined", "room_id": "r1", "user_id": "u1",
            "version": { "epoch": "sfu-1-7f3a", "seq": 1 },
        });
        dispatch(&c, "sfu-1", note("r1", Op::ParticipantEvent, body, &["u1"]));
        assert!(drain(&mut r.inbox[0].1).is_empty(), "자기 제외 — 아무에게도 안 갔다");

        node_down(&c, "sfu-1");
        let got = drain(&mut r.inbox[0].1);
        assert_eq!(got[0].1["version"], serde_json::json!({ "epoch": "sfu-1-7f3a", "seq": 1 }));
    }

    /// 정§15-1 급사 — 그 노드의 방만 닫고, 통지의 `version` 은 ★hub 가 통과시킨 마지막 값이다.
    /// `epoch` 를 `node_id` 로 채우면 그것이 `sfu_id` 인 척한다(연§4-6) — 이 시험이 그 대입을 막는다.
    #[test]
    fn node_down_closes_only_that_nodes_rooms_and_carries_the_passed_version() {
        let mut r = rig(&["u1", "u2"]);
        let c = r.ctx.clone();
        c.rooms.assign("r1", "sfu-1");
        c.rooms.assign("r2", "sfu-2");
        c.rooms.touch("r1", Version { epoch: "sfu-1-7f3a".into(), seq: 42 });
        c.members.join("r1", "u1");
        c.members.join("r1", "u2");
        c.members.join("r2", "u1");

        node_down(&c, "sfu-1");

        for (user, rx) in &mut r.inbox {
            let got = drain(rx);
            assert_eq!(got.len(), 1, "{user} 은 방 하나만 닫혔다");
            let (op, body) = &got[0];
            assert_eq!(*op, Op::RoomEvent.code());
            assert_eq!(body["room_id"], "r1");
            assert_eq!(body["type"], "affiliation");
            assert_eq!(body["cause"], "room_closed");
            assert_eq!(body["affiliation"]["sub_rooms"], serde_json::json!([]));
            assert_eq!(body["version"]["epoch"], "sfu-1-7f3a", "지어낸 epoch 가 아니라 통과값");
            assert_eq!(body["version"]["seq"], 42);
        }
        // ★다른 노드의 방은 건드리지 않는다 — 급사는 노드 단위다.
        assert_eq!(c.rooms.node_of("r2"), Some("sfu-2".to_owned()));
        assert_eq!(c.members.members("r2"), vec!["u1".to_owned()]);
        // 닫힌 방은 배치도 명단도 남지 않는다 — 다시 들어오면 `3001`.
        assert_eq!(c.rooms.node_of("r1"), None);
        assert!(c.members.members("r1").is_empty());
    }

}
