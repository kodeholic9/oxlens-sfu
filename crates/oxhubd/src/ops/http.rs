// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-SURFACE · 정:RULE-BUS-REQUEST · 정:CAND-DOC-ORCHESTRATION-ASPECT · 정:CAND-ROOM-CREATE-WAITS-TOKEN · 연:RULE-TRN-ENDPOINTS · 연:RULE-TRN-HTTP-AUTH-TWO · 연:RULE-TRN-SESSION-HEADER-SCOPE · 연:RULE-TRN-WS-NO-UPGRADE-AUTH · 연:DEV-TRN-WS-ORIGIN · 연:RULE-TRN-CORS · 연:ROOM.http.GET_ROOM · 연:ROOM.http.LIST_ROOMS · 연:RULE-ROOM-DETAIL-NO-FLOOR · model: claude-opus-5-5
//! HTTP 표면 — `{base}/auth/token` · `{base}/rooms` 셋(인증 두 갈래 · CORS) · `{base}/ws`(업그레이드는 무인증 · `Origin` 은 본다) · `/healthz` 둘.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use common::bus::envelope::{Call, Reply, RoomList};
use common::token::{verify_user, TokenError};
use oxsig::{Code, CreateRoomReq, CreateRoomRes, Failure, GetRoomQuery, ListRoomsRes, TokenReq};

use super::App;
use crate::room;

pub fn json_resp(status: StatusCode, body: Vec<u8>) -> Response {
    let mut r = (status, body).into_response();
    r.headers_mut().insert("content-type", HeaderValue::from_static("application/json"));
    r
}

pub fn fail_resp(app: &App, f: &Failure) -> Response {
    if let Some(c) = Code::from_number(f.code) {
        app.ctx.counters.bump(c);
    }
    let status = Code::from_number(f.code).map(Code::http_status).unwrap_or(500);
    json_resp(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), oxsig::parse::encode(f))
}

fn with_cors(app: &App, headers: &HeaderMap, mut r: Response) -> Response {
    let origin = headers.get("origin").and_then(|o| o.to_str().ok());
    for (k, v) in room::cors(origin, &app.ctx.policy.hub.allowed_origins) {
        if let Ok(hv) = HeaderValue::from_str(&v) {
            r.headers_mut().insert(k, hv);
        }
    }
    r
}

pub fn routes(app: Arc<App>) -> Router {
    let base = app.boot.base_path.clone();
    Router::new()
        .route(&format!("{base}/auth/token"), axum::routing::post(token))
        .route(&format!("{base}/rooms"), get(list_rooms).post(create_room).options(preflight))
        .route(&format!("{base}/rooms/:room_id"), get(get_room).options(preflight_id))
        .route(&format!("{base}/ws"), get(ws_upgrade))
        .route("/healthz/live", get(live))
        .route("/healthz/ready", get(ready))
        .merge(super::admin::routes())
        .with_state(app)
}

async fn token(State(app): State<Arc<App>>, body: Bytes) -> Response {
    let req: TokenReq = match oxsig::parse::body(&body) {
        Ok(r) => r,
        Err(e) => return fail_resp(&app, &e.code().failure_with(e.message(), None)),
    };
    let p = &app.ctx.policy.hub;
    match crate::ses::token::issue(&app.boot.system.hub.auth.api_keys, &req, p.token_ttl_secs, p.metadata_max_bytes, common::token::now_s()) {
        Ok(res) => json_resp(StatusCode::OK, oxsig::parse::encode(&res)),
        Err(f) => fail_resp(&app, &f),
    }
}

pub async fn identify(app: &App, headers: &HeaderMap) -> Result<String, Failure> {
    let sid = headers.get("x-oxlens-session").and_then(|v| v.to_str().ok()).filter(|s| !s.is_empty());
    if let Some(sid) = sid {
        if let Some(s) = app.ctx.sessions.get(sid)
            && !s.lock().gone
        {
            return Ok(s.id.user.clone());
        }
        if let Some((user, _)) = app.ctx.bus.sess_by_sid(sid).await {
            return Ok(user);
        }
    }
    let bearer = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim);
    if let Some(t) = bearer {
        let secrets: Vec<&str> = app.ctx.secrets.iter().map(String::as_str).collect();
        return match verify_user(t, &secrets, common::token::now_s()) {
            Ok(c) => c.user().map(str::to_string).ok_or_else(|| Code::TokenInvalid.failure()),
            Err(TokenError::Expired) => Err(Code::TokenExpired.failure()),
            Err(_) => Err(Code::TokenInvalid.failure()),
        };
    }
    if sid.is_some() {
        return Err(Code::SessionNotFound.failure());
    }
    Err(Code::TokenInvalid.failure_with("Authorization 또는 X-OxLens-Session 이 없다", None))
}

async fn preflight(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    with_cors(&app, &headers, StatusCode::NO_CONTENT.into_response())
}

async fn preflight_id(State(app): State<Arc<App>>, headers: HeaderMap, Path(_room): Path<String>) -> Response {
    with_cors(&app, &headers, StatusCode::NO_CONTENT.into_response())
}

async fn list_rooms(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let r = match identify(&app, &headers).await {
        Err(f) => fail_resp(&app, &f),
        Ok(_) => {
            let mut rooms = Vec::new();
            for node in app.ctx.bus.alive_nodes() {
                match app.ctx.bus.call(&node, &Call::ListRooms).await {
                    Reply::Ok(v) => {
                        if let Ok(l) = serde_json::from_value::<RoomList>(v) {
                            rooms.extend(l.rooms);
                        }
                    }
                    Reply::Fail(f) => tracing::warn!(kind = "diag", node = %node, code = f.code, "ROOM_LIST partial — node did not answer"),
                }
            }
            rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
            let total = rooms.len() as u32;
            json_resp(StatusCode::OK, oxsig::parse::encode(&ListRoomsRes { rooms, total }))
        }
    };
    with_cors(&app, &headers, r)
}

async fn get_room(State(app): State<Arc<App>>, headers: HeaderMap, Path(room_id): Path<String>, Query(q): Query<GetRoomQuery>) -> Response {
    let r = async {
        let user = identify(&app, &headers).await?;
        if !oxsig::room_id_ok(&room_id) {
            return Err(Code::InvalidPayload.failure_with("room_id", None));
        }
        let node = app.ctx.bus.room_node(&room_id).ok_or_else(|| Code::RoomNotFound.failure())?;
        match app.ctx.bus.call(&node, &Call::GetRoom { room_id: room_id.clone(), tracks: q.tracks == Some(1), user_id: Some(user) }).await {
            Reply::Ok(v) => Ok(json_resp(StatusCode::OK, serde_json::to_vec(&v).unwrap_or_default())),
            Reply::Fail(f) => Err(f),
        }
    }
    .await
    .unwrap_or_else(|f| fail_resp(&app, &f));
    with_cors(&app, &headers, r)
}

async fn create_room(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let r = async {
        identify(&app, &headers).await?;
        let req: CreateRoomReq = oxsig::parse::body(&body).map_err(|e| e.code().failure_with(e.message(), None))?;
        let p = room::prepare_create(req, &app.ctx.policy, || uuid::Uuid::new_v4().to_string())?;
        let existing = app.ctx.bus.room_node(&p.room_id);
        let max = app.ctx.policy.quota.max_rooms;
        if existing.is_none() && max > 0 && app.ctx.bus.rooms.lock().map(|m| m.len()).unwrap_or(0) >= max as usize {
            return Err(Code::QuotaExceeded.failure());
        }
        let node = match existing {
            Some(n) => n,
            None => room::place(&p.room_id, &app.ctx.bus.candidates()).ok_or_else(|| Code::SfuUnavailable.failure())?,
        };
        if !app.ctx.bus.node_live(&node) {
            return Err(Code::SfuUnavailable.failure_with(format!("placed node {node} is not live"), None));
        }
        let call = Call::CreateRoom { req: p.req.clone(), room_id: p.room_id.clone(), unused_ttl_secs: p.unused_ttl_secs, departure_ttl_secs: p.departure_ttl_secs, capacity: p.capacity };
        let res: CreateRoomRes = match app.ctx.bus.call(&node, &call).await {
            Reply::Ok(v) => serde_json::from_value(v).map_err(|e| Code::InternalError.failure_with(e.to_string(), None))?,
            Reply::Fail(f) => return Err(f),
        };
        let deadline = common::time::wall_ms() + room::CREATE_WAIT_MS;
        while app.ctx.bus.room_node(&p.room_id).is_none() && common::time::wall_ms() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        tracing::info!(kind = "ev", spec = "IMPL-ROOM-LIFE", room = %p.room_id, node = %node, "room created");
        Ok(json_resp(StatusCode::CREATED, oxsig::parse::encode(&res)))
    }
    .await
    .unwrap_or_else(|f| fail_resp(&app, &f));
    with_cors(&app, &headers, r)
}

async fn ws_upgrade(State(app): State<Arc<App>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let origin = headers.get("origin").and_then(|o| o.to_str().ok());
    let host = headers.get("host").and_then(|o| o.to_str().ok());
    if !room::ws_origin_ok(origin, host, &app.ctx.policy.hub.allowed_origins) {
        tracing::warn!(kind = "diag", origin = origin.unwrap_or(""), "WS origin refused");
        return StatusCode::FORBIDDEN.into_response();
    }
    let ctx = app.ctx.clone();
    ws.max_message_size(oxsig::frame::MAX_FRAME * 2).max_frame_size(oxsig::frame::MAX_FRAME * 2).on_upgrade(move |s| crate::ses::conn::serve(ctx, s))
}

async fn live() -> Response {
    json_resp(StatusCode::OK, br#"{"live":true}"#.to_vec())
}

pub fn ready_body(app: &App) -> (bool, serde_json::Value) {
    let down = app.sup.down_units();
    let ready = down.is_empty() && !app.ctx.bus.isolated() && !crate::ses::shutting(&app.ctx);
    let mut v = serde_json::json!({"ready": ready, "build": app.boot.build, "down": down});
    if app.ctx.bus.isolated() {
        v["isolated"] = serde_json::Value::Bool(true);
    }
    (ready, v)
}

async fn ready(State(app): State<Arc<App>>) -> Response {
    let (ok, v) = ready_body(&app);
    json_resp(if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, serde_json::to_vec(&v).unwrap_or_default())
}
