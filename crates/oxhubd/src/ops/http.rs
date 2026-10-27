// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:RULE-TRN-ENDPOINTS · 연:RULE-TRN-HTTP-AUTH-TWO · 연:RULE-TRN-SESSION-HEADER-SCOPE · 연:RULE-TRN-CORS · 연:DEV-TRN-WS-ORIGIN · 연:RULE-TRN-WS-NO-UPGRADE-AUTH · 연:ROOM.http.CREATE_ROOM · 연:RULE-ROOM-CREATE-PROC · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-DERIVED · 정:RULE-BUS-PLACEMENT · 정:CAND-ROOM-CREATE-WAITS-TOKEN · 정:CAND-ROOM-CORS-MAX-AGE · 정:CAND-ROOM-TTL-PERMANENT-MARK · 정:IMPL-ERR-ISSUERS · 정:CAND-ERR-HTTP-MAP · 운영:RULE-AUTH-THREE · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! HTTP 표면 — `{base}` 아래 클라 경로 넷(토큰 · 방 목록 · 생성 · 상세) · WS 업그레이드(무인증 · `Origin` 보조 방어) · 프로브 둘 · 운영 `/admin`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use common::bus::env::{Call, Reply};
use oxsig::code::{Code, Failure};
use oxsig::msg::{CreateRoomReq, TokenReq, TokenRes};
use serde_json::{Value, json};

use super::HubRef;
use super::route::{self, Target};

pub const PREFLIGHT_MAX_AGE: &str = "600";
pub const CREATE_WAIT_MS: u64 = 1_000;
pub const CAPACITY_MAX: u32 = 1_000;
pub const PERMANENT_TTL: u32 = u32::MAX;

pub fn fail(code: Code, status: Option<u16>) -> Response {
    let s = StatusCode::from_u16(status.unwrap_or_else(|| code.http_status())).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (s, axum::Json(Failure::of(code))).into_response()
}

pub fn failure(f: Failure) -> Response {
    let s = StatusCode::from_u16(f.code().http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (s, axum::Json(f)).into_response()
}

fn cors(hub: &HubRef, h: &HeaderMap, r: &mut Response) {
    let allowed = &hub.cfg.policy.hub.allowed_origins;
    if allowed.is_empty() {
        return;
    }
    let origin = h.get("origin").and_then(|v| v.to_str().ok());
    let value = if allowed.iter().any(|o| o == "*") {
        Some("*".to_string())
    } else {
        origin.filter(|o| allowed.iter().any(|a| a == o)).map(str::to_string)
    };
    if let Some(v) = value.and_then(|v| HeaderValue::from_str(&v).ok()) {
        let hd = r.headers_mut();
        hd.insert("access-control-allow-origin", v);
        hd.insert("access-control-allow-headers", HeaderValue::from_static("authorization, content-type, x-oxlens-session"));
        hd.insert("access-control-allow-methods", HeaderValue::from_static("GET, POST, OPTIONS"));
        hd.insert("access-control-max-age", HeaderValue::from_static(PREFLIGHT_MAX_AGE));
    }
}

fn origin_ok(hub: &HubRef, h: &HeaderMap) -> bool {
    let Some(origin) = h.get("origin").and_then(|v| v.to_str().ok()) else { return true };
    let allowed = &hub.cfg.policy.hub.allowed_origins;
    if allowed.iter().any(|o| o == "*" || o == origin) {
        return true;
    }
    let host = h.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
    origin.split_once("://").map(|(_, rest)| rest == host).unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct Who {
    pub user_id: String,
}

pub async fn client_auth(hub: &HubRef, h: &HeaderMap) -> Result<Who, Code> {
    let bearer = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim);
    let sess = h.get("x-oxlens-session").and_then(|v| v.to_str().ok()).map(str::trim);
    let mut err = Code::TOKEN_INVALID;
    if let Some(t) = bearer {
        let secrets = crate::ses::token::secrets(&hub.cfg.system.hub.auth.api_keys, hub.cfg.system.hub.auth.jwt_secret.as_deref());
        match crate::ses::token::verify(t, &secrets) {
            Ok(id) => return Ok(Who { user_id: id.user_id }),
            Err(e) => err = e,
        }
    }
    if let Some(sid) = sess.filter(|s| !s.is_empty()) {
        if let Some(u) = hub.sessions.lock().ok().and_then(|t| t.get(sid).map(|s| s.data.user_id.clone())) {
            return Ok(Who { user_id: u });
        }
        if let Some(bus) = hub.bus() {
            let keys = bus.tokens(&format!("ox/{}/int/sess/*/*/{sid}", hub.ks.ns), common::bus::SESS_GET_TIMEOUT_MS).await;
            if let Some(t) = keys.iter().find_map(|k| hub.ks.parse_sess(k)) {
                return Ok(Who { user_id: t.user });
            }
        }
        return Err(Code::SESSION_NOT_FOUND);
    }
    Err(err)
}

pub fn router(hub: HubRef) -> Router {
    let base = hub.cfg.base_path.clone();
    let a_plane = Router::new()
        .route("/auth/token", axum::routing::post(token))
        .route("/rooms", get(list_rooms).post(create_room).options(preflight))
        .route("/rooms/:room_id", get(get_room).options(preflight))
        .route("/ws", get(ws_upgrade));
    let probes = Router::new().route("/healthz/live", get(live)).route("/healthz/ready", get(ready));
    let app = if base.is_empty() { a_plane } else { Router::new().nest(&base, a_plane) };
    app.merge(probes).merge(super::admin::router()).with_state(hub)
}

async fn live() -> Response {
    axum::Json(json!({"live": true})).into_response()
}

async fn ready(State(hub): State<HubRef>) -> Response {
    let (ok, down) = hub.sup.ready();
    let body = json!({"ready": ok, "build": hub.cfg.build, "down": down});
    let s = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (s, axum::Json(body)).into_response()
}

async fn ws_upgrade(State(hub): State<HubRef>, h: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !origin_ok(&hub, &h) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let h2 = hub.clone();
    ws.max_message_size(oxsig::frame::MAX_FRAME + 1).on_upgrade(move |s| super::ws::serve(h2, s))
}

async fn preflight(State(hub): State<HubRef>, h: HeaderMap) -> Response {
    let mut r = StatusCode::NO_CONTENT.into_response();
    cors(&hub, &h, &mut r);
    r
}

async fn token(State(hub): State<HubRef>, body: Bytes) -> Response {
    let req: TokenReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return fail(crate::ops::ws::body_code(&e), Some(400)),
    };
    let p = &hub.cfg.policy.hub;
    match crate::ses::token::issue(&hub.cfg.system.hub.auth.api_keys, &req, p.token_ttl_secs, p.metadata_max_bytes, common::time::wall_secs()) {
        Ok((token, expires_in)) => axum::Json(TokenRes { token, expires_in }).into_response(),
        Err(c) => {
            hub.drops.code(c.0);
            let status = match c {
                Code::CLAIM_NOT_ALLOWED | Code::INVALID_PAYLOAD | Code::MISSING_FIELD => Some(400),
                Code::INVALID_API_KEY => Some(401),
                _ => None,
            };
            fail(c, status)
        }
    }
}

async fn list_rooms(State(hub): State<HubRef>, h: HeaderMap) -> Response {
    let mut r = match client_auth(&hub, &h).await {
        Err(c) => fail(c, Some(401)),
        Ok(_) => {
            let (rooms, errors) = room_list(&hub).await;
            for e in errors {
                tracing::warn!(peer_node = %e, "방 목록 — 답하지 않은 node(부분 병합)");
            }
            let total = rooms.len();
            axum::Json(json!({"rooms": rooms, "total": total})).into_response()
        }
    };
    cors(&hub, &h, &mut r);
    r
}

pub async fn room_list(hub: &HubRef) -> (Vec<Value>, Vec<String>) {
    let mut rooms = Vec::new();
    let mut errors = Vec::new();
    match hub.bus() {
        Some(bus) => {
            for (node, r) in bus.get_all(&hub.ks.q_all("call"), Call::RoomList.encode(), common::bus::REQUEST_TIMEOUT_MS).await {
                match r.ok().and_then(|b| Reply::decode(&b).ok()) {
                    Some(Reply::Ok { body }) => rooms.extend(body.get("rooms").and_then(|v| v.as_array()).cloned().unwrap_or_default()),
                    _ => errors.push(node),
                }
            }
        }
        None => {
            if let Reply::Ok { body } = route::local(hub, None, &Call::RoomList).await {
                rooms.extend(body.get("rooms").and_then(|v| v.as_array()).cloned().unwrap_or_default());
            }
        }
    }
    (rooms, errors)
}

async fn create_room(State(hub): State<HubRef>, h: HeaderMap, body: Bytes) -> Response {
    let mut r = match client_auth(&hub, &h).await {
        Err(c) => fail(c, Some(401)),
        Ok(_) => match create(&hub, &body).await {
            Ok(v) => (StatusCode::CREATED, axum::Json(v)).into_response(),
            Err(f) => {
                hub.drops.code(f.code);
                let s = if f.code / 1000 == 1 { 400 } else { f.code().http_status() };
                (StatusCode::from_u16(s).unwrap_or(StatusCode::BAD_REQUEST), axum::Json(f)).into_response()
            }
        },
    };
    cors(&hub, &h, &mut r);
    r
}

fn ttl(req: Option<u32>, policy: u32) -> u32 {
    match req {
        Some(v) => v,
        None if policy == 0 => PERMANENT_TTL,
        None => policy,
    }
}

pub async fn create(hub: &HubRef, body: &[u8]) -> Result<Value, Failure> {
    let req: CreateRoomReq = serde_json::from_slice(body).map_err(|e| Failure::of(crate::ops::ws::body_code(&e)))?;
    if req.name.is_empty() {
        return Err(Failure::of(Code::MISSING_FIELD));
    }
    let room_id = match req.room_id.as_deref() {
        None | Some("") => uuid::Uuid::new_v4().to_string(),
        Some(r) if crate::ops::ws::room_id_ok(r) => r.to_string(),
        Some(_) => return Err(Failure::of(Code::INVALID_PAYLOAD)),
    };
    let p = &hub.cfg.policy.hub;
    let capacity = req.capacity.unwrap_or(p.room_default_capacity);
    if capacity == 0 || capacity > CAPACITY_MAX {
        return Err(Failure::of(Code::INVALID_PAYLOAD));
    }
    let existing = route::room_target(hub, &room_id);
    let max = hub.cfg.policy.quota.max_rooms;
    if existing.is_none() && max != 0 && hub.rooms.lock().map(|m| m.count()).unwrap_or(0) >= max as usize {
        return Err(Failure::of(Code::QUOTA_EXCEEDED));
    }
    let target = match existing {
        Some(t) => t,
        None => {
            let node = hub.cands.lock().ok().and_then(|c| c.place(&room_id)).ok_or_else(|| Failure::of(Code::SFU_UNAVAILABLE))?;
            if !hub.node_live(&node) {
                return Err(Failure::of(Code::SFU_UNAVAILABLE));
            }
            Target { node, inst: None }
        }
    };
    let call = Call::RoomCreate {
        room_id: room_id.clone(),
        name: req.name.clone(),
        capacity,
        unused_ttl_secs: ttl(req.unused_ttl_secs, p.room_unused_ttl_secs),
        departure_ttl_secs: ttl(req.departure_ttl_secs, p.room_departure_ttl_secs),
        media_codecs: req.media_codecs.clone(),
    };
    let v = match route::call(hub, &target, &call).await {
        Reply::Ok { body } => body,
        Reply::Fail { failure } => return Err(failure),
        Reply::Http { body, .. } => body,
    };
    let until = super::now_ms() + CREATE_WAIT_MS;
    while super::now_ms() < until && route::room_target(hub, &room_id).is_none() {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(v)
}

#[derive(serde::Deserialize, Default)]
pub struct DetailQuery {
    #[serde(default)]
    pub tracks: Option<u8>,
}

async fn get_room(State(hub): State<HubRef>, h: HeaderMap, Path(room_id): Path<String>, Query(q): Query<DetailQuery>) -> Response {
    let mut r = match client_auth(&hub, &h).await {
        Err(c) => fail(c, Some(401)),
        Ok(who) => {
            if !crate::ops::ws::room_id_ok(&room_id) {
                fail(Code::INVALID_PAYLOAD, Some(400))
            } else {
                match route::room_target(&hub, &room_id) {
                    None => fail(Code::ROOM_NOT_FOUND, Some(404)),
                    Some(t) => match route::call(&hub, &t, &Call::RoomGet { room_id, tracks: q.tracks == Some(1), me: Some(who.user_id) }).await {
                        Reply::Ok { body } => axum::Json(body).into_response(),
                        Reply::Http { status, body } => (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), axum::Json(body)).into_response(),
                        Reply::Fail { failure } => failure_resp(failure),
                    },
                }
            }
        }
    };
    cors(&hub, &h, &mut r);
    r
}

fn failure_resp(f: Failure) -> Response {
    failure(f)
}

pub fn loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

pub async fn serve(hub: HubRef) -> Result<(), String> {
    let app = router(hub.clone());
    let listener = tokio::net::TcpListener::bind(&hub.cfg.listen).await.map_err(|e| format!("listen {}: {e}", hub.cfg.listen))?;
    match &hub.cfg.system.hub.tls {
        None => axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.map_err(|e| e.to_string()),
        Some(tls) => serve_tls(listener, app, &tls.cert, &tls.key).await,
    }
}

async fn serve_tls(listener: tokio::net::TcpListener, app: Router, cert: &str, key: &str) -> Result<(), String> {
    use tokio_rustls::rustls;
    let certs: Vec<_> = rustls_pemfile::certs(&mut std::io::BufReader::new(std::fs::File::open(cert).map_err(|e| format!("tls cert {cert}: {e}"))?))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("tls cert: {e}"))?;
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(std::fs::File::open(key).map_err(|e| format!("tls key {key}: {e}"))?))
        .map_err(|e| format!("tls key: {e}"))?
        .ok_or("tls key 가 없다")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("tls: {e}"))?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    loop {
        let Ok((tcp, peer)) = listener.accept().await else { continue };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else { return };
            let svc = hyper_util::service::TowerToHyperService::new(app.layer(axum::Extension(ConnectInfo(peer))));
            let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(tls), svc).with_upgrades().await;
        });
    }
}
