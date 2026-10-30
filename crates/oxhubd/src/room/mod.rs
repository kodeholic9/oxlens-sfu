// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-REQUEST · 정:IMPL-ROOM-LIFE · 연:ROOM.http.CREATE_ROOM · 연:ROOM.http.GET_ROOM · 연:ROOM.http.LIST_ROOMS · 연:RULE-ROOM-CREATE-FORM · 정:CAND-ROOM-CREATE-WAITS-TOKEN · 정:CAND-ROOM-TTL-PERMANENT-MARK · model: claude-opus-5-5
//! hub 의 방 HTTP — 생성은 배치(HRW) 뒤 그 node 로 · 맵에 보일 때까지 응답을 미룬다 · 상세는 방 → node 맵으로 · 목록은 전 node 병합.

use std::time::Duration;

use common::envelope::{CallKind, Envelope, Reply};
use common::policy::Policy;
use oxsig::msg::{ListRoomsRes, RoomSummary};
use oxsig::{Code, Failure};

use crate::bus::Bus;

pub const CREATE_VISIBLE_WAIT: Duration = Duration::from_secs(1);
pub const PERMANENT_TTL: u32 = u32::MAX;

pub fn valid_room_id(r: &str) -> bool {
    !r.is_empty() && r.len() <= 255 && common::keyspace::valid_piece(r)
}

fn unwrap(r: Reply) -> Result<serde_json::Value, Failure> {
    match r {
        Reply::Ok { body } => Ok(body),
        Reply::Fail { failure } => Err(failure),
        Reply::Http { body, .. } => Ok(body),
    }
}

pub async fn create(bus: &Bus, policy: &Policy, mut body: serde_json::Value) -> Result<serde_json::Value, Failure> {
    let Some(obj) = body.as_object_mut() else {
        return Err(Code::InvalidPayload.failure());
    };
    match obj.get("name") {
        Some(serde_json::Value::String(n)) if !n.is_empty() => {}
        Some(serde_json::Value::String(_)) | None | Some(serde_json::Value::Null) => {
            return Err(Code::MissingField.failure().with_message("name"));
        }
        Some(_) => return Err(Code::InvalidPayload.failure().with_message("name")),
    }
    let room = match obj.get("room_id").and_then(|v| v.as_str()) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    if !valid_room_id(&room) {
        return Err(Code::InvalidPayload.failure().with_message("room_id"));
    }
    obj.insert("room_id".into(), serde_json::Value::String(room.clone()));
    let permanent = |v: u32| if v == 0 { PERMANENT_TTL } else { v };
    for (k, v) in [("unused_ttl_secs", policy.hub.room_unused_ttl_secs), ("departure_ttl_secs", policy.hub.room_departure_ttl_secs)] {
        if obj.get(k).map(|x| x.is_null()).unwrap_or(true) {
            obj.insert(k.into(), serde_json::json!(permanent(v)));
        }
    }
    if policy.quota.max_rooms > 0 {
        let n = bus.view.lock().map(|v| v.routing.len()).unwrap_or(0) as u32;
        if n >= policy.quota.max_rooms && bus.room_route(&room).is_none() {
            return Err(Code::QuotaExceeded.failure());
        }
    }
    let (node, live) = bus.place(&room).ok_or_else(|| Code::SfuUnavailable.failure())?;
    if !live {
        return Err(Code::SfuUnavailable.failure().with_details(serde_json::json!({ "node": node })));
    }
    let mut env = Envelope::new(CallKind::CreateRoom);
    env.body = body;
    let out = unwrap(bus.call(&node, &env).await)?;
    let until = tokio::time::Instant::now() + CREATE_VISIBLE_WAIT;
    while bus.room_route(&room).is_none() && tokio::time::Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(out)
}

pub async fn get(bus: &Bus, room: &str, user: Option<&str>, me_allowed: bool, tracks: bool) -> Result<serde_json::Value, Failure> {
    if !valid_room_id(room) {
        return Err(Code::InvalidPayload.failure().with_message("room_id"));
    }
    let loc = bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    let mut env = Envelope::new(CallKind::GetRoom);
    env.body = serde_json::json!({ "room_id": room, "tracks": tracks, "me": me_allowed });
    env.user_id = user.map(|u| u.to_string());
    unwrap(bus.call(&loc.node, &env).await)
}

pub async fn list(bus: &Bus) -> Result<ListRoomsRes, Failure> {
    let mut rooms: Vec<RoomSummary> = Vec::new();
    for (node, _) in bus.live_nodes() {
        let env = Envelope::new(CallKind::ListRooms);
        match unwrap(bus.call(&node, &env).await) {
            Ok(v) => {
                if let Ok(r) = serde_json::from_value::<ListRoomsRes>(v) {
                    rooms.extend(r.rooms);
                }
            }
            Err(f) => {
                tracing::warn!(kind = "diag", node = %node, code = f.code, "room list — node did not answer");
            }
        }
    }
    rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
    let total = rooms.len() as u32;
    Ok(ListRoomsRes { rooms, total })
}
