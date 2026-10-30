// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-TRN-ENDPOINTS · 연:RULE-TRN-HTTP-AUTH-TWO · 연:RULE-TRN-CORS · 연:DEV-TRN-WS-ORIGIN · 연:RULE-TRN-SESSION-HEADER-SCOPE · 정:IMPL-ERR-ISSUERS · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 운영:BOOT.http.HEALTH_READY · model: claude-opus-5-5
//! HTTP 표면 — `{base}/ws` · `{base}/auth/token` · `{base}/rooms` 셋(CORS) · `/healthz` · `/admin`(운영 토큰 또는 loopback) · HTTP 상태 번호는 일반 사상 하나.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use oxsig::msg::TokenReq;
use oxsig::{Code, Failure};
use serde_json::{Value, json};

use super::{Hub, admin};
use crate::ses;

pub fn status_of(code: u16) -> StatusCode {
    let s = match code {
        2005 => 400,
        1000..=1999 => 400,
        2001..=2004 | 2008 => 401,
        2006 => 403,
        2000..=2999 => 401,
        3001 => 404,
        3009 => 409,
        3000..=3999 => 409,
        4000..=4999 => 429,
        5001 => 503,
        5002 => 502,
        _ => 500,
    };
    StatusCode::from_u16(s).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn fail(f: Failure) -> Response {
    (status_of(f.code), axum::Json(f)).into_response()
}

fn fail_status(st: u16, f: Failure) -> Response {
    (StatusCode::from_u16(st).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), axum::Json(f)).into_response()
}

fn ok(v: Value) -> Response {
    (StatusCode::OK, axum::Json(v)).into_response()
}

type St = State<Arc<Hub>>;

pub fn router(hub: Arc<Hub>) -> Router {
    let base = hub.base_path.trim_end_matches('/').to_string();
    let ops = Router::new()
        .route("/healthz/live", get(live))
        .route("/healthz/ready", get(ready))
        .route("/admin/sfus", get(a_sfus))
        .route("/admin/rooms", get(a_rooms))
        .route("/admin/users", get(a_users))
        .route("/admin/snapshot", get(a_snapshot))
        .route("/admin/drops", get(a_drops))
        .route("/admin/transit", get(a_transit))
        .route("/admin/bus", get(a_bus))
        .route("/admin/rooms/:room/snapshot", get(a_room_snapshot))
        .route("/admin/sfus/:id/rooms", get(a_sfu_rooms))
        .route("/admin/users/:user/media", get(a_user_media))
        .route("/admin/users/:user/probe", get(a_user_probe))
        .route("/admin/users/:user/cut", post(a_cut))
        .route("/admin/rooms/:room/reap/:user", post(a_reap))
        .route("/admin/rooms/:room/destroy", post(a_destroy))
        .route("/admin/supervisor/status", get(a_sup_status))
        .route("/admin/supervisor/load/:id", post(a_sup_load))
        .route("/admin/supervisor/stop/:id", post(a_sup_stop))
        .route("/admin/supervisor/kill/:id", post(a_sup_kill))
        .route("/admin/supervisor/shutdown", post(a_shutdown))
        .route("/admin/log-level", post(a_log_level));
    let client = Router::new()
        .route("/ws", get(ws))
        .route("/auth/token", post(token))
        .route("/rooms", any(rooms))
        .route("/rooms/:room", any(room));
    let r = if base.is_empty() { ops.merge(client) } else { Router::new().merge(ops.clone()).nest(&base, client.merge(ops)) };
    r.with_state(hub)
}

async fn live() -> Response {
    ok(json!({ "live": true }))
}

async fn ready(State(hub): St) -> Response {
    let (r, down) = hub.ready();
    let body = json!({ "ready": r, "build": hub.build, "down": down });
    (if r { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, axum::Json(body)).into_response()
}

fn origin_ok(hub: &Hub, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let list = &hub.policy.hub.allowed_origins;
    if list.iter().any(|o| o == "*" || o == origin) {
        return true;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    origin.split("://").nth(1).map(|h| h == host).unwrap_or(false)
}

async fn ws(State(hub): St, headers: HeaderMap, up: WebSocketUpgrade) -> Response {
    if !origin_ok(&hub, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let ctx = hub.ses.clone();
    up.max_message_size(oxsig::frame::MAX_FRAME_BYTES * 2)
        .max_frame_size(oxsig::frame::MAX_FRAME_BYTES * 2)
        .on_upgrade(move |sock| ses::conn::serve(sock, ctx))
}

async fn token(State(hub): St, body: Bytes) -> Response {
    let req: TokenReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let missing = e.to_string().contains("missing field");
            return fail(if missing { Code::MissingField.failure() } else { Code::InvalidPayload.failure() });
        }
    };
    match hub.ses.accounts.issue(&req, hub.policy.hub.token_ttl_secs, hub.policy.hub.metadata_max_bytes) {
        Ok(t) => ok(serde_json::to_value(t).unwrap_or_default()),
        Err(f) => fail(f),
    }
}

fn cors(hub: &Hub, headers: &HeaderMap, mut r: Response) -> Response {
    let list = &hub.policy.hub.allowed_origins;
    if list.is_empty() {
        return r;
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).unwrap_or("*");
    let allow = if list.iter().any(|o| o == "*") { "*".to_string() } else if list.iter().any(|o| o == origin) { origin.to_string() } else { return r };
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&allow) {
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("authorization, content-type, x-oxlens-session"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    r
}

enum Who {
    Bearer,
    Session(Arc<ses::Session>),
}

fn who(hub: &Hub, headers: &HeaderMap) -> Result<Who, Failure> {
    if let Some(sid) = headers.get("x-oxlens-session").and_then(|v| v.to_str().ok()) {
        return hub.ses.session(sid).map(Who::Session).ok_or_else(|| Code::SessionNotFound.failure());
    }
    let tok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| Code::TokenInvalid.failure())?;
    hub.ses.accounts.verify_user(tok).map(|_| Who::Bearer).map_err(|c| c.failure())
}

async fn rooms(State(hub): St, method: Method, headers: HeaderMap, body: Bytes) -> Response {
    if method == Method::OPTIONS {
        return cors(&hub, &headers, StatusCode::NO_CONTENT.into_response());
    }
    let r = match who(&hub, &headers) {
        Err(f) => fail(f),
        Ok(_) if method == Method::GET => match crate::room::list(&hub.bus).await {
            Ok(l) => ok(serde_json::to_value(l).unwrap_or_default()),
            Err(f) => fail(f),
        },
        Ok(_) if method == Method::POST => {
            let v: Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(_) => return cors(&hub, &headers, fail(Code::InvalidPayload.failure())),
            };
            match crate::room::create(&hub.bus, &hub.policy, v).await {
                Ok(v) => (StatusCode::CREATED, axum::Json(v)).into_response(),
                Err(f) => fail(f),
            }
        }
        Ok(_) => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };
    cors(&hub, &headers, r)
}

async fn room(
    State(hub): St,
    method: Method,
    Path(room): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if method == Method::OPTIONS {
        return cors(&hub, &headers, StatusCode::NO_CONTENT.into_response());
    }
    if method != Method::GET {
        return cors(&hub, &headers, StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    let tracks = q.get("tracks").map(|v| v == "1").unwrap_or(false);
    let r = match who(&hub, &headers) {
        Err(f) => fail(f),
        Ok(w) => {
            let (user, me) = match &w {
                Who::Session(s) => (Some(s.user_id.clone()), s.in_room(&room)),
                Who::Bearer => (None, false),
            };
            match crate::room::get(&hub.bus, &room, user.as_deref(), me, tracks).await {
                Ok(v) => ok(v),
                Err(f) => fail(f),
            }
        }
    };
    cors(&hub, &headers, r)
}

fn admin_auth(hub: &Hub, peer: SocketAddr, headers: &HeaderMap) -> Result<(), Failure> {
    if peer.ip().is_loopback() && headers.get(header::AUTHORIZATION).is_none() {
        return Ok(());
    }
    let tok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| Code::TokenInvalid.failure())?;
    hub.ses.accounts.verify_ops(tok).map(|_| ()).map_err(|c| c.failure())
}

macro_rules! guard {
    ($hub:expr, $peer:expr, $h:expr) => {
        if let Err(f) = admin_auth(&$hub, $peer, &$h) {
            return fail_status(401, f);
        }
    };
}

fn res(r: admin::AdminResult) -> Response {
    match r {
        Ok(v) => ok(v),
        Err(f) => fail(f),
    }
}

type Peer = ConnectInfo<SocketAddr>;

async fn a_sfus(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::sfus(&hub))
}
async fn a_rooms(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::rooms(&hub))
}
async fn a_users(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::users(&hub).await)
}
async fn a_snapshot(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::snapshot(&hub))
}
async fn a_drops(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::drops(&hub).await)
}
async fn a_transit(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::transit(&hub).await)
}
async fn a_bus(State(hub): St, ConnectInfo(p): Peer, Query(q): Query<std::collections::HashMap<String, String>>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    if q.get("all").map(|v| v == "1").unwrap_or(false) {
        return ok(admin::bus_all(&hub).await);
    }
    ok(admin::bus_view(&hub).await)
}
async fn a_room_snapshot(State(hub): St, ConnectInfo(p): Peer, Path(room): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    res(admin::room_snapshot(&hub, &room).await)
}
async fn a_sfu_rooms(State(hub): St, ConnectInfo(p): Peer, Path(id): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    match admin::sfu_rooms(&hub, &id).await {
        Ok(v) => ok(v),
        Err(f) if f.code == Code::PeerNotFound.num() => fail_status(404, f),
        Err(f) => fail(f),
    }
}
async fn a_user_media(State(hub): St, ConnectInfo(p): Peer, Path(user): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    match admin::user_media(&hub, &user).await {
        Ok(v) => ok(v),
        Err(f) if f.code == Code::SessionNotFound.num() => fail_status(404, f),
        Err(f) => fail(f),
    }
}
async fn a_user_probe(State(hub): St, ConnectInfo(p): Peer, Path(user): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    match admin::probe(&hub, &user).await {
        Ok(v) => ok(v),
        Err(f) => fail_status(404, f),
    }
}
async fn a_cut(State(hub): St, ConnectInfo(p): Peer, Path(user): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    match admin::cut(&hub, &user).await {
        Ok(v) => ok(v),
        Err(f) => fail_status(404, f),
    }
}
async fn a_reap(State(hub): St, ConnectInfo(p): Peer, Path((room, user)): Path<(String, String)>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    match admin::reap(&hub, &room, &user).await {
        Ok(v) => ok(v),
        Err(f) if f.code == Code::NotInRoom.num() || f.code == Code::RoomNotFound.num() => fail_status(404, f),
        Err(f) => fail(f),
    }
}
async fn a_destroy(
    State(hub): St,
    ConnectInfo(p): Peer,
    Path(room): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    h: HeaderMap,
) -> Response {
    guard!(hub, p, h);
    res(admin::destroy(&hub, &room, q.get("if_version").map(|s| s.as_str())).await)
}
async fn a_sup_status(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    ok(admin::sup_status(&hub))
}
async fn a_sup_load(State(hub): St, ConnectInfo(p): Peer, Path(id): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    res(admin::sup_ctl(&hub, &id, "load", None).await)
}
async fn a_sup_stop(State(hub): St, ConnectInfo(p): Peer, Path(id): Path<String>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    res(admin::sup_ctl(&hub, &id, "stop", None).await)
}
async fn a_sup_kill(
    State(hub): St,
    ConnectInfo(p): Peer,
    Path(id): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    h: HeaderMap,
) -> Response {
    guard!(hub, p, h);
    res(admin::sup_ctl(&hub, &id, "kill", q.get("if_epoch").map(|s| s.as_str())).await)
}
async fn a_shutdown(State(hub): St, ConnectInfo(p): Peer, Query(q): Query<std::collections::HashMap<String, String>>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    res(admin::shutdown(&hub, q.get("confirm").map(|s| s.as_str())))
}
async fn a_log_level(State(hub): St, ConnectInfo(p): Peer, h: HeaderMap, body: Bytes) -> Response {
    guard!(hub, p, h);
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    res(admin::log_level(&hub, &v).await)
}

pub async fn serve(hub: Arc<Hub>, listen: &str, tls: Option<(String, String)>) -> Result<(), String> {
    let app = router(hub.clone());
    let lis = tokio::net::TcpListener::bind(listen).await.map_err(|e| format!("listen {listen}: {e}"))?;
    match tls {
        None => {
            tokio::spawn(async move {
                let _ = axum::serve(lis, app.into_make_service_with_connect_info::<SocketAddr>()).await;
            });
        }
        Some((cert, key)) => {
            let acceptor = tls_acceptor(&cert, &key)?;
            tokio::spawn(async move {
                loop {
                    let Ok((stream, peer)) = lis.accept().await else { continue };
                    let acceptor = acceptor.clone();
                    let app = app.clone();
                    tokio::spawn(async move {
                        let Ok(tls) = acceptor.accept(stream).await else { return };
                        let svc = hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                            req.extensions_mut().insert(ConnectInfo(peer));
                            let app = app.clone();
                            async move { tower::ServiceExt::oneshot(app, req).await }
                        });
                        let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                            .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(tls), svc)
                            .await;
                    });
                }
            });
        }
    }
    Ok(())
}

fn tls_acceptor(cert: &str, key: &str) -> Result<tokio_rustls::TlsAcceptor, String> {
    let certs: Vec<_> = rustls_pemfile::certs(&mut std::io::BufReader::new(std::fs::File::open(cert).map_err(|e| format!("tls cert: {e}"))?))
        .filter_map(|c| c.ok())
        .collect();
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(std::fs::File::open(key).map_err(|e| format!("tls key: {e}"))?))
        .map_err(|e| format!("tls key: {e}"))?
        .ok_or_else(|| "tls key: none".to_string())?;
    let cfg = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("tls: {e}"))?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(cfg)))
}
