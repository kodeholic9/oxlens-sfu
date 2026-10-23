// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-RESYNC · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-REQUEST · 정:RULE-OPS-SURFACE · 연:ROOM.http.LIST_ROOMS · 연:ROOM.http.CREATE_ROOM · 연:ROOM.http.GET_ROOM · 연:RULE-ROOM-CREATE-PROC · 연:RULE-TRN-CORS · model: claude-opus-5-5
//! ROOM 영역(hub 쪽) — `{base}/rooms` 세 자리: 목록은 전 미디어 서버 병합 · 생성은 배치(HRW) 뒤 그 node 로 · 상세는 방 맵으로. 방 상태는 복제하지 않는다.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use common::handle::{Call, Reply};
use common::policy::Policy;
use oxsig::err::{ErrCode, Failure};
use oxsig::room::{room_id_ok, CreateRoomReq, CAPACITY_MAX};
use serde_json::{json, Value};

use crate::bus::Bus;

pub type IdentFn = Arc<dyn Fn(&HeaderMap) -> Result<(String, Option<String>), ErrCode> + Send + Sync>;

#[derive(Clone)]
pub struct Rooms {
    pub bus: Arc<Bus>,
    pub policy: Arc<Policy>,
    pub ident: IdentFn,
    pub status: Arc<dyn Fn(ErrCode) -> StatusCode + Send + Sync>,
}

fn fail(r: &Rooms, f: Failure) -> Response {
    let st = f.err_code().map(|c| (r.status)(c)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (st, Json(f)).into_response()
}

fn reply(r: &Rooms, rep: Reply, ok: StatusCode) -> Response {
    match rep {
        Reply::Ok { body } => (ok, Json(body)).into_response(),
        Reply::Fail { failure } => fail(r, failure),
        Reply::Http { status, body } => (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(body)).into_response(),
    }
}

async fn list(State(r): State<Rooms>, h: HeaderMap) -> Response {
    if let Err(c) = (r.ident)(&h) {
        return fail(&r, Failure::new(c));
    }
    let parts = r.bus.fanout(&Call::ListRooms).await;
    let mut rooms: Vec<Value> = Vec::new();
    for (node, rep) in parts {
        match rep {
            Reply::Ok { body } => rooms.extend(body.get("rooms").and_then(Value::as_array).cloned().unwrap_or_default()),
            other => tracing::warn!(kind = "diag", node = %node, reply = ?other, "room list partial — node failed"),
        }
    }
    let total = rooms.len();
    (StatusCode::OK, Json(json!({"rooms": rooms, "total": total}))).into_response()
}

async fn create(State(r): State<Rooms>, h: HeaderMap, body: axum::body::Bytes) -> Response {
    if let Err(c) = (r.ident)(&h) {
        return fail(&r, Failure::new(c));
    }
    let req: CreateRoomReq = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return fail(&r, Failure::new(ErrCode::InvalidPayload).msg(e.to_string())),
    };
    if req.name.as_deref().map(str::is_empty).unwrap_or(true) {
        return fail(&r, Failure::new(ErrCode::MissingField).msg("name 없음"));
    }
    if req.capacity.map(|c| c > CAPACITY_MAX || c == 0).unwrap_or(false) {
        return fail(&r, Failure::new(ErrCode::InvalidPayload).msg("capacity 는 1~1000"));
    }
    let room_id = match req.room_id.clone().filter(|s| !s.is_empty()) {
        Some(id) => id,
        None => common::ident::uuid4(),
    };
    if !room_id_ok(&room_id) {
        return fail(&r, Failure::new(ErrCode::InvalidPayload).msg("room_id 형식"));
    }
    let existing = r.bus.room_of(&room_id).map(|p| p.node);
    let node = match existing.or_else(|| r.bus.place(&room_id)) {
        Some(n) => n,
        None => return fail(&r, Failure::new(ErrCode::SfuUnavailable)),
    };
    if !r.bus.node_live(&node) {
        return fail(&r, Failure::new(ErrCode::SfuUnavailable).msg(format!("배치된 node {node} 가 live 가 아니다")));
    }
    let max_rooms = r.policy.quota.max_rooms;
    if max_rooms != 0 && r.bus.room_of(&room_id).is_none() && r.bus.rooms().len() as u32 >= max_rooms {
        return fail(&r, Failure::new(ErrCode::QuotaExceeded));
    }
    let mut req = req;
    req.room_id = Some(room_id.clone());
    let call = Call::CreateRoom {
        unused_ttl_secs: req.unused_ttl_secs.or(Some(r.policy.hub.room_unused_ttl_secs)),
        departure_ttl_secs: req.departure_ttl_secs.or(Some(r.policy.hub.room_departure_ttl_secs)),
        capacity: req.capacity.unwrap_or(r.policy.hub.room_default_capacity),
        req,
    };
    let rep = r.bus.call(&node, &call).await;
    if matches!(rep, Reply::Ok { .. }) {
        for _ in 0..50 {
            if r.bus.room_of(&room_id).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    tracing::info!(kind = "diag", room = %room_id, node = %node, "room create routed");
    reply(&r, rep, StatusCode::CREATED)
}

async fn detail(State(r): State<Rooms>, h: HeaderMap, Path(room_id): Path<String>, Query(q): Query<std::collections::BTreeMap<String, String>>) -> Response {
    let (user, sid) = match (r.ident)(&h) {
        Ok(v) => v,
        Err(c) => return fail(&r, Failure::new(c)),
    };
    if !room_id_ok(&room_id) {
        return fail(&r, Failure::new(ErrCode::InvalidPayload).msg("room_id 형식"));
    }
    let tracks = q.get("tracks").map(|v| v == "1").unwrap_or(false);
    let call = Call::GetRoom { room_id: room_id.clone(), tracks, session_id: sid, user_id: Some(user) };
    let rep = r.bus.call_room(&room_id, &call).await;
    reply(&r, rep, StatusCode::OK)
}

pub fn cors(policy: &Policy, h: &HeaderMap, resp: &mut Response) {
    let allowed = &policy.hub.allowed_origins;
    if allowed.is_empty() {
        return;
    }
    let origin = h.get("origin").and_then(|v| v.to_str().ok());
    let allow = if allowed.iter().any(|a| a == "*") {
        Some("*".to_string())
    } else {
        origin.filter(|o| allowed.iter().any(|a| a == o)).map(str::to_string)
    };
    let Some(a) = allow else { return };
    let hs = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&a) {
        hs.insert("access-control-allow-origin", v);
    }
    hs.insert("access-control-allow-headers", HeaderValue::from_static("authorization, content-type, x-oxlens-session"));
    hs.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, OPTIONS"));
    hs.insert("access-control-max-age", HeaderValue::from_static("600"));
}

pub fn router(r: Rooms) -> Router {
    let p = r.policy.clone();
    Router::new()
        .route("/rooms", get(list).post(create).options(|| async { StatusCode::NO_CONTENT }))
        .route("/rooms/:room_id", get(detail).options(|| async { StatusCode::NO_CONTENT }))
        .with_state(r)
        .layer(axum::middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
            let p = p.clone();
            async move {
                let h = req.headers().clone();
                let is_pre = req.method() == Method::OPTIONS;
                let mut resp = next.run(req).await;
                cors(&p, &h, &mut resp);
                if is_pre {
                    resp.headers_mut().remove("content-length");
                }
                resp
            }
        }))
}
