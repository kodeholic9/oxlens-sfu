// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 연:SES.http.TOKEN 연:ROOM.http.LIST_ROOMS 연:ROOM.http.CREATE_ROOM 연:ROOM.http.GET_ROOM 연:RULE-TRN-HTTP-AUTH-TWO 연:RULE-TRN-CORS 연:DEV-TRN-WS-ORIGIN 정:IMPL-SES-TOKEN-ISSUE 정:IMPL-ROOM-RESYNC 운영 규격서 §8-2 · model: claude-opus-5-5
//! A 평면 HTTP · WS 업그레이드 · `/healthz`.

use std::sync::Arc;

use axum::Json;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use oxsig::Code;
use oxsig::bplane::{UnitReq, UnitRes};
use oxsig::wire::{CreateRoomReq, GetRoomQuery, ListRoomsRes, TokenReq, TokenRes};

use crate::auth::{ALL_ALLOWED, Claims};
use crate::state::Hub;

pub const MAX_CAPACITY: u32 = 1000;

pub fn failure(status: StatusCode, code: Code, msg: Option<&str>) -> Response {
    let f = match msg {
        Some(m) => code.failure_msg(m),
        None => code.failure(),
    };
    (status, Json(f)).into_response()
}

fn status_of(code: Code) -> StatusCode {
    match code.0 {
        1001..=1008 => StatusCode::BAD_REQUEST,
        2001..=2010 => StatusCode::UNAUTHORIZED,
        3001 => StatusCode::NOT_FOUND,
        3009 => StatusCode::CONFLICT,
        4001..=4009 => StatusCode::TOO_MANY_REQUESTS,
        5001 => StatusCode::SERVICE_UNAVAILABLE,
        5002 => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub fn fail_code(code: Code) -> Response {
    failure(status_of(code), code, None)
}

fn cors(hub: &Hub, headers: &HeaderMap, mut res: Response) -> Response {
    let list = &hub.policy.hub.allowed_origins;
    if list.is_empty() {
        return res;
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let allow = if list.iter().any(|o| o == "*") {
        Some("*".to_string())
    } else {
        origin.filter(|o| list.iter().any(|l| l == o)).map(str::to_string)
    };
    if let Some(a) = allow.and_then(|a| HeaderValue::from_str(&a).ok()) {
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, a);
        h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
        h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, x-oxlens-session"));
    }
    res
}

pub async fn preflight(State(hub): State<Arc<Hub>>, headers: HeaderMap) -> Response {
    cors(&hub, &headers, StatusCode::NO_CONTENT.into_response())
}

enum Viewer {
    Token(Claims),
    Session { user: String, rooms: Option<Vec<String>> },
}

impl Viewer {
    fn user(&self) -> &str {
        match self {
            Viewer::Token(c) => &c.sub,
            Viewer::Session { user, .. } => user,
        }
    }
}

async fn authenticate(hub: &Arc<Hub>, headers: &HeaderMap) -> Result<Viewer, Code> {
    if let Some(sid) = headers.get("x-oxlens-session").and_then(|v| v.to_str().ok()) {
        if let Some(user) = crate::session::lookup_user(hub, sid) {
            return Ok(Viewer::Session { user, rooms: crate::session::session_rooms(hub, sid) });
        }
        if let Some(bus) = hub.bus() {
            for k in common::bus::live_keys(bus, &hub.keys.sess_by_sid(sid), crate::route::CALL_TIMEOUT).await {
                if let Some(common::keyspace::Token::Sess { user, .. }) = common::keyspace::parse_token(hub.keys.prefix(), &k) {
                    return Ok(Viewer::Session { user, rooms: None });
                }
            }
        }
        if headers.get(header::AUTHORIZATION).is_none() {
            return Err(Code::SESSION_NOT_FOUND);
        }
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(Code::TOKEN_INVALID)?;
    crate::auth::verify_user(&hub.system, bearer).map(Viewer::Token)
}

pub async fn token(State(hub): State<Arc<Hub>>, body: axum::body::Bytes) -> Response {
    let req: TokenReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let c = if e.to_string().contains("missing field") { Code::MISSING_FIELD } else { Code::INVALID_PAYLOAD };
            return failure(StatusCode::BAD_REQUEST, c, Some(&e.to_string()));
        }
    };
    let Some(acct) = hub.system.account(&req.api_key).filter(|a| a.secret == req.api_secret) else {
        return failure(StatusCode::UNAUTHORIZED, Code::INVALID_API_KEY, None);
    };
    let pt = req.participant_type.unwrap_or(0);
    if pt > 2 {
        return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some("participant_type"));
    }
    let hidden = req.hidden.unwrap_or(false);
    if !acct.participant_types.contains(&pt) || (hidden && !acct.hidden_allowed) {
        return failure(StatusCode::BAD_REQUEST, Code::CLAIM_NOT_ALLOWED, None);
    }
    if let Some(m) = &req.metadata
        && serde_json::to_vec(m).map(|v| v.len()).unwrap_or(usize::MAX) > hub.policy.hub.metadata_max_bytes as usize {
            return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some("metadata too large"));
        }
    let now = crate::auth::now_secs();
    let ttl = hub.policy.hub.token_ttl_secs;
    let claims = Claims {
        sub: req.user_id.clone(),
        participant_type: pt,
        hidden,
        metadata: req.metadata.clone(),
        permission: Some(req.permission.unwrap_or(ALL_ALLOWED)),
        iat: now,
        exp: now + u64::from(ttl),
    };
    match crate::auth::issue(&acct.secret, &claims) {
        Ok(t) => (StatusCode::OK, Json(TokenRes { token: t, expires_in: ttl })).into_response(),
        Err(c) => failure(StatusCode::INTERNAL_SERVER_ERROR, c, None),
    }
}

pub async fn list_rooms(State(hub): State<Arc<Hub>>, headers: HeaderMap) -> Response {
    let res = match authenticate(&hub, &headers).await {
        Err(c) => fail_code(c),
        Ok(_) => {
            let mut rooms = Vec::new();
            for (node, r) in crate::route::fanout(&hub, UnitReq::ListRooms).await {
                match r {
                    Ok(UnitRes::Rooms(v)) => rooms.extend(v),
                    other => tracing::warn!(kind = "diag", node = node.as_str(), "room list part failed: {other:?}"),
                }
            }
            rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
            let total = rooms.len() as u32;
            Json(ListRoomsRes { rooms, total }).into_response()
        }
    };
    cors(&hub, &headers, res)
}

pub async fn create_room(State(hub): State<Arc<Hub>>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let res = create_room_inner(&hub, &headers, &body).await;
    cors(&hub, &headers, res)
}

async fn create_room_inner(hub: &Arc<Hub>, headers: &HeaderMap, body: &[u8]) -> Response {
    if let Err(c) = authenticate(hub, headers).await {
        return fail_code(c);
    }
    let mut req: CreateRoomReq = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(e) => {
            let c = if e.to_string().contains("missing field") { Code::MISSING_FIELD } else { Code::INVALID_PAYLOAD };
            return failure(StatusCode::BAD_REQUEST, c, Some(&e.to_string()));
        }
    };
    let capacity = req.capacity.unwrap_or(hub.policy.hub.room_default_capacity.min(MAX_CAPACITY));
    if capacity > MAX_CAPACITY || capacity == 0 {
        return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some("capacity"));
    }
    let room_id = match req.room_id.as_deref() {
        None | Some("") => uuid::Uuid::new_v4().to_string(),
        Some(r) => r.to_string(),
    };
    if !crate::session::valid_room_id(&room_id) {
        return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some("room_id"));
    }
    let max = hub.policy.quota.max_rooms;
    if max > 0 && hub.rooms.read().map(|m| m.len() as u32).unwrap_or(0) >= max {
        return failure(StatusCode::TOO_MANY_REQUESTS, Code::QUOTA_EXCEEDED, None);
    }
    let Some(node) = hub.placement(&room_id) else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, Code::SFU_UNAVAILABLE, Some("no node"));
    };
    if !hub.node_live(&node) {
        return failure(StatusCode::SERVICE_UNAVAILABLE, Code::SFU_UNAVAILABLE, Some("placed node not live"));
    }
    req.room_id = Some(room_id);
    let unit_req = UnitReq::CreateRoom {
        req,
        capacity,
        unused_ttl_secs: hub.policy.hub.room_unused_ttl_secs,
        departure_ttl_secs: hub.policy.hub.room_departure_ttl_secs,
    };
    match crate::route::unit(hub, &node, unit_req).await {
        Ok(UnitRes::Created(c)) => {
            if let Ok(mut m) = hub.rooms.write() {
                let v = m.entry(c.room_id.clone()).or_default();
                if !v.iter().any(|p| p.node == node) {
                    v.push(crate::state::Place { node: node.clone(), inst: String::new() });
                }
            }
            (StatusCode::CREATED, Json(c)).into_response()
        }
        Ok(UnitRes::Fail { code, message, .. }) => failure(status_of(Code(code)), Code(code), Some(&message)),
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

pub async fn get_room(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
    Query(q): Query<GetRoomQuery>,
) -> Response {
    let res = get_room_inner(&hub, &headers, &room_id, q).await;
    cors(&hub, &headers, res)
}

async fn get_room_inner(hub: &Arc<Hub>, headers: &HeaderMap, room_id: &str, q: GetRoomQuery) -> Response {
    let viewer = match authenticate(hub, headers).await {
        Ok(v) => v,
        Err(c) => return fail_code(c),
    };
    if !crate::session::valid_room_id(room_id) {
        return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some("room_id"));
    }
    let Some(place) = hub.room_place(room_id) else { return fail_code(Code::ROOM_NOT_FOUND) };
    let user = match &viewer {
        Viewer::Session { rooms: Some(rooms), .. } if !rooms.iter().any(|r| r == room_id) => None,
        v => Some(v.user().to_string()),
    };
    let req = UnitReq::GetRoom { room_id: room_id.to_string(), viewer: user, tracks: q.tracks == Some(1) };
    match crate::route::unit(hub, &place.node, req).await {
        Ok(UnitRes::Room(r)) => Json(*r).into_response(),
        Ok(UnitRes::Fail { code, message, .. }) => failure(status_of(Code(code)), Code(code), Some(&message)),
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

fn origin_allowed(hub: &Hub, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let list = &hub.policy.hub.allowed_origins;
    if list.iter().any(|o| o == "*" || o == origin) {
        return true;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")) == Some(host)
}

pub async fn ws(State(hub): State<Arc<Hub>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !origin_allowed(&hub, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.max_message_size(oxsig::frame::MAX_FRAME + 1)
        .on_upgrade(move |socket| crate::conn::run(hub, socket))
        .into_response()
}

pub async fn live() -> Response {
    Json(serde_json::json!({ "live": true })).into_response()
}

pub fn ready_state(hub: &Hub) -> (bool, Vec<String>) {
    let down = hub.units.down_units();
    let ok = down.is_empty() && !hub.isolated.load(std::sync::atomic::Ordering::Relaxed) && !hub.is_shutting_down();
    (ok, down)
}

pub async fn ready(State(hub): State<Arc<Hub>>) -> Response {
    let (ok, down) = ready_state(&hub);
    let body = serde_json::json!({ "ready": ok, "build": hub.build, "down": down });
    let st = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (st, Json(body)).into_response()
}
