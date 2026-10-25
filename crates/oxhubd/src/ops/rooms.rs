// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-ROOM-LIFE 정:IMPL-ROOM-DERIVED 정:IMPL-ROOM-RESYNC 정:IMPL-SES-TOKEN-ISSUE 정:RULE-BUS-PLACEMENT 정:RULE-BUS-REQUEST 정:RULE-OPS-SURFACE 정:CAND-ROOM-CREATE-WAITS-TOKEN 정:CAND-ROOM-CORS-MAX-AGE 정:CAND-ROOM-TTL-PERMANENT-MARK 정:CAND-ERR-HTTP-MAP 연:ROOM.http.LIST_ROOMS 연:ROOM.http.CREATE_ROOM 연:ROOM.http.GET_ROOM 연:SES.http.TOKEN 연:RULE-TRN-ENDPOINTS 연:RULE-TRN-HTTP-AUTH-TWO 연:RULE-TRN-SESSION-HEADER-SCOPE 연:RULE-TRN-CORS 연:RULE-TRN-NO-NEW-SECRET 연:RULE-ROOM-CREATE-PROC 연:RULE-ROOM-MEDIA-CODECS 연:RULE-SES-TOKEN-ACCOUNT-ALLOW 연:RULE-SES-TOKEN-NO-FLOOR-POLICY 연:RULE-ROOM-DETAIL-FIELDS · model: claude-opus-5-5
//! A 평면 HTTP — `POST /auth/token` · `GET /rooms` · `POST /rooms` · `GET /rooms/{room_id}` · 인증 두 갈래(Bearer · 세션 헤더) · CORS(클라 경로 셋만).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use oxsig::envelope::{Call, Reply};
use oxsig::msg::{CreateRoomReq, CreateRoomRes, ListRoomsRes, MediaCodecs, Permission, RoomSummary, TokenReq, TokenRes};
use oxsig::{Code, Failure};

use crate::ops::hub::Hub;

pub fn fail(code: Code, f: Failure) -> Response {
    let st = StatusCode::from_u16(code.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    (st, [(header::CONTENT_TYPE, "application/json")], serde_json::to_vec(&f).unwrap_or_default()).into_response()
}

pub fn fail_c(code: Code) -> Response {
    fail(code, code.failure())
}

fn fail_from(f: Failure) -> Response {
    let code = Code::from_num(f.code).unwrap_or(Code::InternalError);
    fail(code, f)
}

pub fn ok_json(st: StatusCode, body: Vec<u8>) -> Response {
    (st, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn cors(hub: &Hub, h: &HeaderMap, mut r: Response) -> Response {
    let allowed = &hub.cfg.policy.hub.allowed_origins;
    if allowed.is_empty() {
        return r;
    }
    let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return r;
    };
    let value = if allowed.iter().any(|a| a == "*") {
        "*".to_string()
    } else if allowed.iter().any(|a| a == o) {
        o.to_string()
    } else {
        return r;
    };
    if let Ok(v) = HeaderValue::from_str(&value) {
        r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    r.headers_mut().insert(header::VARY, HeaderValue::from_static("origin"));
    r
}

async fn preflight(State(hub): State<Arc<Hub>>, h: HeaderMap) -> Response {
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, x-oxlens-session"));
    r.headers_mut().insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    cors(&hub, &h, r)
}

pub struct Viewer {
    pub user: String,
}

pub async fn auth(hub: &Arc<Hub>, h: &HeaderMap) -> Result<Viewer, Code> {
    let mut err = Code::TokenInvalid;
    if let Some(t) = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) {
        match common::jwt::verify_user(t.trim(), &hub.cfg.sys) {
            Ok(c) => {
                if let Some(u) = c.user() {
                    return Ok(Viewer { user: u.to_string() });
                }
            }
            Err(c) => err = c,
        }
    }
    if let Some(sid) = h.get("x-oxlens-session").and_then(|v| v.to_str().ok()) {
        if let Some(s) = hub.ses.get(sid) {
            return Ok(Viewer { user: s.id.user.clone() });
        }
        if let Some((user, _node)) = hub.bus.sess_by_sid(sid).await {
            return Ok(Viewer { user });
        }
        err = Code::SessionNotFound;
    }
    Err(err)
}

async fn token(State(hub): State<Arc<Hub>>, body: Bytes) -> Response {
    let req: TokenReq = match serde_json::from_slice::<serde_json::Value>(&body) {
        Err(_) => return fail_c(Code::InvalidPayload),
        Ok(v) => {
            for k in ["api_key", "api_secret", "user_id"] {
                if v.get(k).and_then(|x| x.as_str()).is_none() {
                    return fail(Code::MissingField, Code::MissingField.failure_msg(format!("{k} 없음")));
                }
            }
            match serde_json::from_value(v) {
                Ok(r) => r,
                Err(_) => return fail_c(Code::InvalidPayload),
            }
        }
    };
    let Some(acct) = hub.cfg.sys.account(&req.api_key).filter(|a| a.secret == req.api_secret) else {
        return fail_c(Code::InvalidApiKey);
    };
    let ptype = req.participant_type.unwrap_or(0);
    if ptype > 2 {
        return fail(Code::InvalidPayload, Code::InvalidPayload.failure_msg("participant_type"));
    }
    let hidden = req.hidden.unwrap_or(false);
    if !acct.participant_types.contains(&ptype) || (hidden && !acct.hidden_allowed) {
        return fail_c(Code::ClaimNotAllowed);
    }
    let max = hub.cfg.policy.hub.metadata_max_bytes;
    if let Some(m) = &req.metadata
        && max > 0
        && serde_json::to_vec(m).map(|b| b.len()).unwrap_or(0) as u32 > max
    {
        return fail(Code::InvalidPayload, Code::InvalidPayload.failure_msg("metadata 상한"));
    }
    if req.user_id.is_empty() {
        return fail(Code::MissingField, Code::MissingField.failure_msg("user_id 없음"));
    }
    let now = common::jwt::now_secs();
    let ttl = hub.cfg.policy.hub.token_ttl_secs;
    let claims = common::jwt::UserClaims {
        sub: Some(req.user_id.clone()),
        user_id: None,
        participant_type: ptype,
        hidden,
        permission: Some(req.permission.clone().unwrap_or(Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true })),
        metadata: req.metadata.clone(),
        iat: now,
        exp: now + u64::from(ttl),
    };
    match common::jwt::sign(&claims, &acct.secret) {
        Ok(t) => {
            common::ev!(
                spec = "CP-OP-BIND",
                user = req.user_id.as_str(),
                "token issued account={} participant_type={ptype} hidden={hidden}",
                acct.name.as_deref().unwrap_or(&acct.key)
            );
            ok_json(StatusCode::OK, serde_json::to_vec(&TokenRes { token: t, expires_in: ttl }).unwrap_or_default())
        }
        Err(c) => fail_c(c),
    }
}

async fn list(State(hub): State<Arc<Hub>>, h: HeaderMap) -> Response {
    let r = match auth(&hub, &h).await {
        Err(c) => fail_c(c),
        Ok(_) => {
            let mut rooms: Vec<RoomSummary> = Vec::new();
            for node in hub.bus.live_nodes() {
                match hub.call_node(&node, None, &Call::ListRooms).await {
                    Reply::Ok { body } => match serde_json::from_str::<ListRoomsRes>(&body) {
                        Ok(l) => rooms.extend(l.rooms),
                        Err(e) => tracing::warn!("list rooms: node {node} body {e}"),
                    },
                    other => {
                        tracing::warn!("list rooms: node {node} failed {other:?} — partial merge")
                    }
                }
            }
            rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
            let total = rooms.len() as u32;
            ok_json(StatusCode::OK, serde_json::to_vec(&ListRoomsRes { rooms, total }).unwrap_or_default())
        }
    };
    cors(&hub, &h, r)
}

fn resolve_ttl(req: Option<u32>, policy: u32) -> u32 {
    match req {
        Some(v) => v,
        None if policy == 0 => u32::MAX,
        None => policy,
    }
}

async fn create(State(hub): State<Arc<Hub>>, h: HeaderMap, body: Bytes) -> Response {
    let r = match auth(&hub, &h).await {
        Err(c) => fail_c(c),
        Ok(_) => create_inner(&hub, &body).await,
    };
    cors(&hub, &h, r)
}

async fn create_inner(hub: &Arc<Hub>, body: &[u8]) -> Response {
    let v: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return fail_c(Code::InvalidPayload),
    };
    if v.get("name").and_then(|n| n.as_str()).is_none() {
        return fail(Code::MissingField, Code::MissingField.failure_msg("name 없음"));
    }
    let mut req: CreateRoomReq = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(_) => return fail_c(Code::InvalidPayload),
    };
    let cap_max = hub.cfg.policy.hub.room_default_capacity;
    if req.capacity.is_some_and(|c| c > cap_max || c == 0) {
        return fail(Code::InvalidPayload, Code::InvalidPayload.failure_msg("capacity"));
    }
    req.capacity = Some(req.capacity.unwrap_or(cap_max));
    let room_id = match req.room_id.as_deref() {
        None | Some("") => common::id::uuid4(),
        Some(r) => {
            if r.len() > 255 || !common::bus::room_id_ok(r) {
                return fail(Code::InvalidPayload, Code::InvalidPayload.failure_msg("room_id 형식"));
            }
            r.to_string()
        }
    };
    req.room_id = Some(room_id.clone());
    req.unused_ttl_secs = Some(resolve_ttl(req.unused_ttl_secs, hub.cfg.policy.hub.room_unused_ttl_secs));
    req.departure_ttl_secs = Some(resolve_ttl(req.departure_ttl_secs, hub.cfg.policy.hub.room_departure_ttl_secs));
    let mc = req.media_codecs.clone().unwrap_or(MediaCodecs { audio: None, video: None });
    req.media_codecs = Some(MediaCodecs {
        audio: Some(mc.audio.filter(|a| !a.is_empty()).unwrap_or_else(|| "opus".to_string())),
        video: Some(mc.video.filter(|a| !a.is_empty()).unwrap_or_else(|| "H264".to_string())),
    });
    let existing = hub.bus.room_where(&room_id);
    let node = match &existing {
        Some((n, _)) => n.clone(),
        None => {
            let max = hub.cfg.policy.quota.max_rooms;
            if max > 0 && hub.bus.room_count() as u32 >= max {
                return fail_c(Code::QuotaExceeded);
            }
            let cands = hub.bus.candidates();
            let Some(n) = common::bus::place(cands.iter().map(String::as_str), &room_id).map(str::to_string) else {
                return fail_c(Code::SfuUnavailable);
            };
            n
        }
    };
    if !hub.bus.node_live(&node) {
        return fail_c(Code::SfuUnavailable);
    }
    let call = Call::CreateRoom { room_id: room_id.clone(), req };
    match hub.call_node(&node, existing.as_ref().map(|e| e.1.as_str()), &call).await {
        Reply::Ok { body } => {
            let t0 = Instant::now();
            while hub.bus.room_where(&room_id).is_none() && t0.elapsed() < Duration::from_secs(1) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let res: CreateRoomRes = match serde_json::from_str(&body) {
                Ok(r) => r,
                Err(e) => return fail(Code::SfuError, Code::SfuError.failure_msg(e.to_string())),
            };
            common::ev!(spec = "SV-STATE-UNIT-STATE", room = room_id.as_str(), "room create placed node={node}");
            ok_json(StatusCode::CREATED, serde_json::to_vec(&res).unwrap_or_default())
        }
        Reply::Fail { failure } => fail_from(failure),
        Reply::Http { status, body } => ok_json(StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body.into_bytes()),
    }
}

#[derive(serde::Deserialize)]
struct GetQ {
    tracks: Option<u8>,
}

async fn detail(State(hub): State<Arc<Hub>>, h: HeaderMap, Path(room_id): Path<String>, Query(q): Query<GetQ>) -> Response {
    let r = match auth(&hub, &h).await {
        Err(c) => fail_c(c),
        Ok(v) => match hub.bus.room_where(&room_id) {
            None => fail_c(Code::RoomNotFound),
            Some((node, inst)) => {
                let call = Call::GetRoom { room_id: room_id.clone(), tracks: q.tracks == Some(1), viewer: Some(v.user) };
                match hub.call_node(&node, Some(&inst), &call).await {
                    Reply::Ok { body } => ok_json(StatusCode::OK, body.into_bytes()),
                    Reply::Fail { failure } => fail_from(failure),
                    Reply::Http { status, body } => ok_json(StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body.into_bytes()),
                }
            }
        },
    };
    cors(&hub, &h, r)
}

pub fn routes(base_path: &str) -> Router<Arc<Hub>> {
    let base = base_path.trim_end_matches('/');
    Router::new()
        .route(&format!("{base}/auth/token"), post(token))
        .route(&format!("{base}/rooms"), get(list).post(create).options(preflight))
        .route(&format!("{base}/rooms/:room_id"), get(detail).options(preflight))
}
