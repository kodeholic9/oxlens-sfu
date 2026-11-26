// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-STATUS · 정:RULE-HOOK-PERMISSION · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-REQUEST · 정:RULE-FLR-SNAPSHOT · model: claude-opus-5-5
//! 현황 조회 · 제어의 답 — 응답자는 hub: 방 목록(엄격 — 못 닿은 node 가 있으면 그 Call 의 실패 코드 그대로) · 방 상세(방을 맡은 node 로 방 귀속 요청 · 발언권을 같이 — 봉투 `floor` · 답은 `GET_ROOM` 몸 + `floor{speaker, queue, start}` 로 읽는다 · 못 읽으면 미디어 서버 오류) · 권한 제어(방을 맡은 node 로 방 귀속 요청 `Permission` · 대상 사용자는 `target_user` · 성공은 적용된 넷) · zenoh 입구의 get 하나에 답 하나(`answer` — 성공은 몸 JSON · 실패는 `Failure` JSON · 제어 payload `{"user_id", "permission"}` 을 못 읽으면 INVALID_PAYLOAD).

use common::envelope::{CallKind, Envelope, PermissionCall, Reply, RoomDetail};
use oxsig::msg::{ListRoomsRes, Permission};
use oxsig::{Code, Failure};

use serde::{Deserialize, Serialize};

use crate::bus::Bus;
use crate::hook::zgate::judge::Ask;

fn unwrap(r: Reply) -> Result<serde_json::Value, Failure> {
    match r {
        Reply::Ok { body } | Reply::Http { body, .. } => Ok(body),
        Reply::Fail { failure } => Err(failure),
    }
}

pub async fn rooms(bus: &Bus) -> Result<ListRoomsRes, Failure> {
    crate::room::list_strict(bus).await
}

pub fn detail_env(room: &str) -> Envelope {
    let mut env = Envelope::new(CallKind::GetRoom);
    env.body = serde_json::json!({ "room_id": room, "tracks": false, "me": false });
    env.floor = true;
    env
}

pub async fn detail(bus: &Bus, room: &str) -> Result<RoomDetail, Failure> {
    if !crate::room::valid_room_id(room) {
        return Err(Code::InvalidPayload.failure().with_message("room_id"));
    }
    let loc = bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    let body = unwrap(bus.call(&loc.node, &detail_env(room)).await)?;
    serde_json::from_value::<RoomDetail>(body).map_err(|_| Code::SfuError.failure())
}

pub fn permission_env(room: &str, user: &str, permission: Permission) -> Envelope {
    let mut env = Envelope::new(CallKind::Permission);
    env.body = serde_json::to_value(PermissionCall { room_id: room.to_string(), permission }).unwrap_or_default();
    env.target_user = Some(user.to_string());
    env
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CtlBody {
    pub user_id: String,
    pub permission: Permission,
}

fn json<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_default()
}

pub async fn answer(bus: &Bus, ask: &Ask, payload: &[u8]) -> Result<Vec<u8>, Vec<u8>> {
    let r: Result<Vec<u8>, Failure> = match ask {
        Ask::List => rooms(bus).await.map(|v| json(&v)),
        Ask::Detail(room) => detail(bus, room).await.map(|v| json(&v)),
        Ask::Ctl(room) => match serde_json::from_slice::<CtlBody>(payload) {
            Ok(b) => permission(bus, room, &b.user_id, b.permission).await.map(|v| json(&v)),
            Err(_) => Err(Code::InvalidPayload.failure()),
        },
        Ask::Outside => Err(Code::UnknownOp.failure()),
    };
    r.map_err(|f| json(&f))
}

pub async fn permission(bus: &Bus, room: &str, user: &str, permission: Permission) -> Result<Permission, Failure> {
    if !crate::room::valid_room_id(room) {
        return Err(Code::InvalidPayload.failure().with_message("room_id"));
    }
    let loc = bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    let body = unwrap(bus.call(&loc.node, &permission_env(room, user, permission)).await)?;
    serde_json::from_value::<Permission>(body).map_err(|_| Code::SfuError.failure())
}
