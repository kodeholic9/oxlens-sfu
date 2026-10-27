// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 정:IMPL-OPS-CUT · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-OPS-DROPS · 운영:RULE-AUTH-THREE · 운영:RULE-READ-SFUS · 운영:RULE-READ-ROOMS · 운영:RULE-READ-USERS · 운영:RULE-READ-SNAPSHOT · 운영:RULE-READ-DROPS · 운영:RULE-READ-ROOM-DETAIL · 운영:RULE-READ-NODE-ROOMS · 운영:RULE-READ-BUS · 운영:RULE-READ-BUS-ALL · 운영:RULE-READ-USER-MEDIA · 운영:RULE-READ-USER-PROBE · 운영:RULE-CTL-SUPERVISOR · 운영:RULE-CTL-CUT · 운영:RULE-CTL-ROOM · 운영:RULE-CTL-LOG-LEVEL · 운영:RULE-RISK-CONFIRM · 운영:RULE-ERR-ADMIN-CODES · 운영:RULE-FMT-REPLY · 운영:RULE-FMT-NO-INVENT · model: claude-opus-5-5
//! 운영 표면 `/admin/*` — 운영 토큰 또는 loopback(XFF 불신) · 읽기 · 되돌릴 수 있음 · 확인값 필수 셋 · 모르는 것은 지어내지 않는다.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use common::bus::env::{Call, Reply};
use oxsig::code::{Code, Failure};
use oxsig::op::Op;
use serde_json::{Value, json};

use super::route::{self, Target};
use super::sup::{Cmd, UnitState, ZENOH_UNIT};
use super::{HubRef, now_ms};
use common::system::{UnitKind, UnitRole};

fn admin_fail(code: Code, status: u16) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), axum::Json(Failure::of(code))).into_response()
}

fn reply_resp(r: Reply) -> Response {
    match r {
        Reply::Ok { body } => axum::Json(body).into_response(),
        Reply::Http { status, body } => (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), axum::Json(body)).into_response(),
        Reply::Fail { failure } => {
            let s = match failure.code {
                3002 | 3001 | 2008 => 404,
                3009 => 409,
                1003 | 1002 => 400,
                c => Code(c).http_status(),
            };
            (StatusCode::from_u16(s).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), axum::Json(failure)).into_response()
        }
    }
}

fn authorized(hub: &HubRef, peer: &SocketAddr, h: &HeaderMap) -> Result<(), Box<Response>> {
    if super::http::loopback(peer) {
        return Ok(());
    }
    let tok = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim);
    let Some(t) = tok else { return Err(Box::new(admin_fail(Code::TOKEN_INVALID, 401))) };
    crate::ses::token::verify_ops(t, &hub.cfg.system.hub.auth.api_keys).map(|_| ()).map_err(|c| Box::new(admin_fail(c, 401)))
}

macro_rules! guard {
    ($hub:expr, $peer:expr, $h:expr) => {
        if let Err(r) = authorized(&$hub, &$peer, &$h) {
            return *r;
        }
    };
}

pub fn router() -> Router<HubRef> {
    Router::new()
        .route("/admin/sfus", get(sfus))
        .route("/admin/rooms", get(rooms))
        .route("/admin/users", get(users))
        .route("/admin/snapshot", get(snapshot))
        .route("/admin/drops", get(drops))
        .route("/admin/bus", get(bus))
        .route("/admin/rooms/:room_id/snapshot", get(room_snapshot))
        .route("/admin/sfus/:sfu_id/rooms", get(sfu_rooms))
        .route("/admin/users/:user_id/media", get(user_media))
        .route("/admin/users/:user_id/probe", get(user_probe))
        .route("/admin/supervisor/status", get(sup_status))
        .route("/admin/supervisor/load/:id", post(sup_load))
        .route("/admin/supervisor/stop/:id", post(sup_stop))
        .route("/admin/supervisor/kill/:id", post(sup_kill))
        .route("/admin/supervisor/shutdown", post(sup_shutdown))
        .route("/admin/users/:user_id/cut", post(user_cut))
        .route("/admin/rooms/:room_id/reap/:user_id", post(room_reap))
        .route("/admin/rooms/:room_id/destroy", post(room_destroy))
        .route("/admin/log-level", post(log_level))
}

fn sfu_rows(hub: &HubRef) -> Vec<Value> {
    let node_live = hub.node_live(hub.me());
    hub.sup
        .snapshot()
        .iter()
        .filter(|u| u.cfg.role == UnitRole::Sfu && u.cfg.kind != UnitKind::Inproc)
        .map(|u| {
            let mut o = json!({"unit": u.cfg.id, "live": u.state == UnitState::Running, "node_live": node_live, "unit_state": u.state.as_str()});
            if let Some(a) = &u.cfg.addr {
                o["addr"] = json!(a);
            }
            if u.state == UnitState::Running
                && let Some(e) = &u.epoch
            {
                o["sfu_id"] = json!(e);
            }
            o
        })
        .collect()
}

async fn sfus(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    axum::Json(json!({"sfus": sfu_rows(&hub), "supervising": hub.sup.supervising()})).into_response()
}

async fn rooms(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    let entries = hub.rooms.lock().map(|m| m.entries()).unwrap_or_default();
    let rows: Vec<Value> = entries
        .into_iter()
        .map(|(r, l)| {
            let ids = hub.sessions.lock().map(|t| t.room_members(&r)).unwrap_or_default();
            json!({"room_id": r, "node": l.node, "sfu_id": l.inst, "members": ids.len(), "member_ids": ids})
        })
        .collect();
    let total = rows.len();
    axum::Json(json!({"rooms": rows, "total": total})).into_response()
}

async fn users(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    let sessions: Vec<_> = hub.sessions.lock().map(|t| t.all().map(|s| s.data.clone()).collect()).unwrap_or_default();
    let mut rows = Vec::new();
    for d in sessions {
        let mut links = Vec::new();
        for (sfu, srv) in &d.servers {
            let call = Call::Admin { method: "GET".into(), path: format!("/admin/users/{}/links", d.user_id), query: BTreeMap::new(), body: None };
            if let Reply::Ok { body } | Reply::Http { body, .. } = route::call(&hub, &Target { node: srv.node.clone(), inst: Some(sfu.clone()) }, &call).await
                && let Some(a) = body.get("links").and_then(|v| v.as_array())
            {
                links.extend(a.iter().cloned());
            }
        }
        rows.push(json!({"session": d.fingerprint(), "user_id": d.user_id, "participant_type": d.participant_type, "hidden": d.hidden, "links": links}));
    }
    let total = rows.len();
    axum::Json(json!({"users": rows, "total": total})).into_response()
}

async fn snapshot(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    let (ready, _) = hub.sup.ready();
    let rooms = hub.rooms.lock().map(|m| m.count()).unwrap_or(0);
    let users = hub.sessions.lock().map(|t| t.len()).unwrap_or(0);
    axum::Json(json!({"hub_id": hub.me(), "ready": ready, "build": hub.cfg.build, "supervising": hub.sup.supervising(), "sfus": sfu_rows(&hub), "rooms": rooms, "users": users}))
        .into_response()
}

pub fn hub_drops(hub: &HubRef) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("unit".into(), json!(hub.me()));
    o.insert("notice_no_socket".into(), json!(hub.drops.notice_no_socket.load(Ordering::Relaxed)));
    o.insert("bus_ev_drop".into(), json!(hub.drops.bus_ev_drop.load(Ordering::Relaxed)));
    for (k, v) in common::log::sender::drops() {
        o.insert(k.into(), json!(v));
    }
    if let Ok(c) = hub.drops.codes.lock() {
        for (k, v) in c.iter() {
            o.insert(format!("code_{k}"), json!(v));
        }
    }
    Value::Object(o)
}

async fn drops(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    let mut units = vec![hub_drops(&hub)];
    for u in hub.sup.snapshot().iter().filter(|u| u.cfg.role == UnitRole::Sfu && u.cfg.kind != UnitKind::Inproc && u.cfg.enabled) {
        let call = Call::Admin { method: "GET".into(), path: "/admin/drops".into(), query: BTreeMap::new(), body: None };
        let r = match &u.epoch {
            Some(e) if u.state == UnitState::Running => route::local(&hub, Some(e), &call).await,
            _ => Reply::fail(Code::SFU_UNAVAILABLE),
        };
        let row = match r {
            Reply::Ok { mut body } | Reply::Http { mut body, .. } => {
                body["unit"] = json!(u.cfg.id);
                if let Some(e) = &u.epoch {
                    body["sfu_id"] = json!(e);
                }
                body
            }
            Reply::Fail { failure } => json!({"unit": u.cfg.id, "error": failure.code}),
        };
        units.push(row);
    }
    axum::Json(json!({"units": units})).into_response()
}

pub async fn bus_body(hub: &HubRef) -> Value {
    let z = &hub.cfg.system.zenoh;
    let peers = match hub.bus() {
        Some(b) => b.peers().await.into_iter().map(|p| json!({"zid": p.zid, "whatami": p.whatami, "links": p.links})).collect::<Vec<_>>(),
        None => Vec::new(),
    };
    let (nodes, seen, iso, last) = match hub.rooms.lock() {
        Ok(m) => {
            let nodes: Vec<Value> = m.nodes().into_iter().map(|(n, i)| if n == hub.me() { json!({"node": n, "inst": i, "self": true}) } else { json!({"node": n, "inst": i}) }).collect();
            let seen: Vec<Value> = m.entries().into_iter().filter(|(_, l)| l.node != hub.me()).map(|(r, l)| json!({"room_id": r, "node": l.node, "inst": l.inst})).collect();
            let iso = m.isolated().map(|(t, h)| json!({"since_ms": t, "held_rooms": h})).unwrap_or(Value::Null);
            let last = m.last_event.as_ref().map(|e| json!({"at_ms": e.at_ms, "kind": if e.put { "put" } else { "delete" }, "key": e.key})).unwrap_or(Value::Null);
            (nodes, seen, iso, last)
        }
        Err(_) => (Vec::new(), Vec::new(), Value::Null, Value::Null),
    };
    json!({"node": hub.me(), "inst": hub.epoch, "mode": z.mode, "namespace": z.namespace, "listen": z.listen, "connect": z.connect, "peers": peers, "nodes": nodes, "rooms_seen": seen, "isolated": iso, "last_event": last})
}

#[derive(serde::Deserialize, Default)]
struct AllQ {
    all: Option<u8>,
}

async fn bus(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Query<AllQ>) -> Response {
    guard!(hub, p, h);
    if q.all != Some(1) {
        return axum::Json(bus_body(&hub).await).into_response();
    }
    let mut want: BTreeSet<String> = hub.cands.lock().map(|c| c.set().into_iter().collect()).unwrap_or_default();
    want.insert(hub.me().to_string());
    let mut got: BTreeMap<String, Value> = BTreeMap::new();
    if let Some(b) = hub.bus() {
        for (node, r) in b.get_all(&hub.ks.q_all("bus"), Vec::new(), common::bus::REQUEST_TIMEOUT_MS).await {
            if let Ok(bytes) = r
                && let Ok(v) = serde_json::from_slice::<Value>(&bytes)
            {
                got.insert(node, v);
            }
        }
    }
    let nodes: Vec<Value> = want
        .iter()
        .map(|n| match got.get(n) {
            Some(v) => json!({"node": n, "bus": v}),
            None => json!({"node": n, "error": Code::SFU_UNAVAILABLE.0}),
        })
        .collect();
    axum::Json(json!({"asked": hub.me(), "nodes": nodes})).into_response()
}

async fn room_snapshot(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>) -> Response {
    guard!(hub, p, h);
    let Some(t) = route::room_target(&hub, &room) else { return admin_fail(Code::ROOM_NOT_FOUND, 404) };
    let call = Call::Admin { method: "GET".into(), path: format!("/admin/rooms/{room}/snapshot"), query: BTreeMap::new(), body: None };
    reply_resp(route::call(&hub, &t, &call).await)
}

async fn sfu_rooms(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(sfu): Path<String>) -> Response {
    guard!(hub, p, h);
    let unit = hub.sup.snapshot().into_iter().find(|u| u.cfg.id == sfu || u.epoch.as_deref() == Some(sfu.as_str()));
    let (unit_id, epoch) = match &unit {
        Some(u) => (u.cfg.id.clone(), u.epoch.clone().unwrap_or_default()),
        None => (sfu.clone(), sfu.clone()),
    };
    let Some(t) = route::sfu_target(&hub, &epoch) else { return admin_fail(Code::SFU_UNAVAILABLE, 503) };
    let rows = match route::call(&hub, &t, &Call::RoomList).await {
        Reply::Ok { body } => body.get("rooms").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
        r => return reply_resp(r),
    };
    let rooms: Vec<Value> = rows.iter().map(|r| json!({"room_id": r["room_id"], "members": r["user_count"]})).collect();
    let total = rooms.len();
    axum::Json(json!({"unit": unit_id, "sfu_id": epoch, "rooms": rooms, "total": total})).into_response()
}

async fn user_servers(hub: &HubRef, user: &str) -> Vec<(String, String)> {
    let local = hub.sessions.lock().ok().and_then(|t| t.of_user(user).map(|s| s.data.servers.iter().map(|(k, v)| (k.clone(), v.node.clone())).collect::<Vec<_>>()));
    if let Some(l) = local {
        return l;
    }
    let Some(b) = hub.bus() else { return Vec::new() };
    let keys = b.tokens(&hub.ks.sess_of_user(user), common::bus::SESS_GET_TIMEOUT_MS).await;
    let mut out = Vec::new();
    for t in keys.iter().filter_map(|k| hub.ks.parse_sess(k)) {
        if let Ok(bytes) = b.get_one(&format!("{}?user={user}", hub.ks.q_node(&t.node, "servers")), Vec::new(), common::bus::REQUEST_TIMEOUT_MS).await
            && let Ok(v) = serde_json::from_slice::<Vec<(String, String)>>(&bytes)
        {
            out.extend(v);
        }
    }
    out
}

async fn user_media(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(hub, p, h);
    let mut links = Vec::new();
    for (sfu, node) in user_servers(&hub, &user).await {
        let call = Call::Admin { method: "GET".into(), path: format!("/admin/users/{user}/media"), query: BTreeMap::new(), body: None };
        match route::call(&hub, &Target { node, inst: Some(sfu.clone()) }, &call).await {
            Reply::Ok { body } | Reply::Http { body, .. } => links.push(body),
            Reply::Fail { failure } => links.push(json!({"sfu_id": sfu, "error": failure.code})),
        }
    }
    axum::Json(json!({"user_id": user, "now": common::time::wall_ms(), "links": links})).into_response()
}

pub async fn probe_local(hub: &HubRef, user: &str) -> Vec<Value> {
    let target = hub.sessions.lock().ok().and_then(|t| t.of_user(user).map(|s| (s.data.sid.clone(), s.data.fingerprint(), s.sock)));
    let Some((sid, fp, sock)) = target else { return Vec::new() };
    let Some(sock) = sock else { return vec![json!({"session": fp, "timeout": true})] };
    let (req_id, rx) = hub.probes.open(&sid);
    hub.push_frame(sock, Op::Task.code(), &json!({"phase": "request", "req_id": req_id, "type": "probe", "params": {}}));
    match tokio::time::timeout(std::time::Duration::from_millis(crate::aux::PROBE_TIMEOUT_MS), rx).await {
        Ok(Ok(r)) => vec![json!({"session": fp, "result": r})],
        _ => {
            hub.probes.close(&sid, req_id);
            vec![json!({"session": fp, "timeout": true})]
        }
    }
}

async fn user_probe(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(hub, p, h);
    let mut sessions = probe_local(&hub, &user).await;
    if let Some(b) = hub.bus() {
        let keys = b.tokens(&hub.ks.sess_of_user(&user), common::bus::SESS_GET_TIMEOUT_MS).await;
        for t in keys.iter().filter_map(|k| hub.ks.parse_sess(k)).filter(|t| t.node != hub.me()) {
            if let Ok(bytes) = b.get_one(&format!("{}?user={user}", hub.ks.q_node(&t.node, "probe")), Vec::new(), common::bus::REQUEST_TIMEOUT_MS).await
                && let Ok(v) = serde_json::from_slice::<Vec<Value>>(&bytes)
            {
                sessions.extend(v);
            }
        }
    }
    if sessions.is_empty() {
        return admin_fail(Code::SESSION_NOT_FOUND, 404);
    }
    axum::Json(json!({"user_id": user, "sessions": sessions})).into_response()
}

async fn sup_status(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(hub, p, h);
    axum::Json(super::sup::status_json(&hub)).into_response()
}

fn transition(id: &str, r: Result<(UnitState, UnitState), Code>) -> Response {
    match r {
        Ok((from, to)) => axum::Json(json!({"id": id, "from": from.as_str(), "to": to.as_str()})).into_response(),
        Err(Code::ROOM_NOT_FOUND) => admin_fail(Code::ROOM_NOT_FOUND, 404),
        Err(c) => admin_fail(c, if c == Code::PRECONDITION_FAILED { 409 } else { c.http_status() }),
    }
}

async fn sup_load(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(hub, p, h);
    transition(&id, hub.sup.command(&id, Cmd::Load).await)
}

async fn sup_stop(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(hub, p, h);
    transition(&id, hub.sup.command(&id, Cmd::Stop).await)
}

#[derive(serde::Deserialize, Default)]
struct KillQ {
    if_epoch: Option<String>,
}

async fn sup_kill(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>, Query(q): Query<KillQ>) -> Response {
    guard!(hub, p, h);
    let Some(e) = q.if_epoch else { return admin_fail(Code::MISSING_FIELD, 400) };
    let actual = if id == ZENOH_UNIT { Some(hub.epoch.clone()) } else { hub.sup.snapshot().into_iter().find(|u| u.cfg.id == id).and_then(|u| u.epoch) };
    if actual.as_deref() != Some(e.as_str()) {
        return (StatusCode::CONFLICT, axum::Json(Failure::of(Code::PRECONDITION_FAILED).details(json!({"expected": e, "actual": actual})))).into_response();
    }
    transition(&id, hub.sup.command(&id, |r| Cmd::Kill(e.clone(), r)).await)
}

#[derive(serde::Deserialize, Default)]
struct ConfirmQ {
    confirm: Option<String>,
}

async fn sup_shutdown(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Query<ConfirmQ>) -> Response {
    guard!(hub, p, h);
    let Some(c) = q.confirm else { return admin_fail(Code::MISSING_FIELD, 400) };
    if c != hub.me() {
        return (StatusCode::CONFLICT, axum::Json(Failure::of(Code::PRECONDITION_FAILED).details(json!({"expected": c, "actual": hub.me()})))).into_response();
    }
    let h2 = hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        super::begin_shutdown(&h2, "admin shutdown").await;
    });
    axum::Json(json!({"accepted": true, "order": ["clients", "units", "hub"]})).into_response()
}

pub async fn cut_local(hub: &HubRef, user: &str) -> Vec<Value> {
    let sid = hub.sessions.lock().ok().and_then(|t| t.of_user(user).map(|s| s.data.sid.clone()));
    let Some(sid) = sid else { return Vec::new() };
    let fp = common::log::fingerprint(&sid);
    hub.discard_session(&sid, Code::SESSION_REVOKED, super::GoneBy::Server).await;
    vec![json!({"session": fp, "code": Code::SESSION_REVOKED.0})]
}

async fn user_cut(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(hub, p, h);
    let mut cut = cut_local(&hub, &user).await;
    if let Some(b) = hub.bus() {
        let keys = b.tokens(&hub.ks.sess_of_user(&user), common::bus::SESS_GET_TIMEOUT_MS).await;
        for t in keys.iter().filter_map(|k| hub.ks.parse_sess(k)).filter(|t| t.node != hub.me()) {
            if let Ok(bytes) = b.get_one(&format!("{}?user={user}", hub.ks.q_node(&t.node, "cut")), Vec::new(), common::bus::REQUEST_TIMEOUT_MS).await
                && let Ok(v) = serde_json::from_slice::<Vec<Value>>(&bytes)
            {
                cut.extend(v);
            }
        }
    }
    if cut.is_empty() {
        return admin_fail(Code::SESSION_NOT_FOUND, 404);
    }
    let total = cut.len();
    axum::Json(json!({"user_id": user, "cut": cut, "total": total})).into_response()
}

async fn room_reap(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path((room, user)): Path<(String, String)>) -> Response {
    guard!(hub, p, h);
    let Some(t) = route::room_target(&hub, &room) else { return admin_fail(Code::ROOM_NOT_FOUND, 404) };
    let call = Call::Admin { method: "POST".into(), path: format!("/admin/rooms/{room}/reap/{user}"), query: BTreeMap::new(), body: None };
    reply_resp(route::call(&hub, &t, &call).await)
}

#[derive(serde::Deserialize, Default)]
struct VersionQ {
    if_version: Option<String>,
}

async fn room_destroy(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>, Query(q): Query<VersionQ>) -> Response {
    guard!(hub, p, h);
    let Some(t) = route::room_target(&hub, &room) else { return admin_fail(Code::ROOM_NOT_FOUND, 404) };
    let Some(v) = q.if_version else { return admin_fail(Code::MISSING_FIELD, 400) };
    let mut query = BTreeMap::new();
    query.insert("if_version".to_string(), v);
    let call = Call::Admin { method: "POST".into(), path: format!("/admin/rooms/{room}/destroy"), query, body: None };
    let r = route::call(&hub, &t, &call).await;
    if matches!(&r, Reply::Ok { .. }) || matches!(&r, Reply::Http { status, .. } if *status < 300) {
        let until = super::now_ms() + common::bus::SETTLE_MS + super::http::CREATE_WAIT_MS;
        while super::now_ms() < until && route::room_target(&hub, &room).is_some() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    reply_resp(r)
}

#[derive(serde::Deserialize)]
struct LevelReq {
    level: String,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

async fn log_level(State(hub): State<HubRef>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, body: Bytes) -> Response {
    guard!(hub, p, h);
    let Ok(req) = serde_json::from_slice::<LevelReq>(&body) else { return admin_fail(Code::INVALID_PAYLOAD, 400) };
    let Some(to) = common::policy::LoggingLevel::parse(&req.level) else { return admin_fail(Code::INVALID_PAYLOAD, 400) };
    let from = common::log::level::current_at(now_ms());
    let ttl = req.ttl_secs.unwrap_or(u64::MAX / 4_000);
    common::log::level::set_override(to, ttl, now_ms());
    let until = req.ttl_secs.map(|t| common::time::wall_ms() + t * 1000);
    let mut units = Vec::new();
    for (id, _, mut c) in hub.sup.clients_running().await {
        let call = Call::LogLevel { level: req.level.clone(), ttl_secs: req.ttl_secs.unwrap_or(0) };
        if c.call(common::grpc::CallReq { envelope: call.encode() }).await.is_ok() {
            units.push(id);
        }
    }
    common::ev!(spec = "OP-PATH-ADMIN-LOG-LEVEL", from = from.as_str(), to = to.as_str(), "로그 수준 일시 덮기");
    axum::Json(json!({"from": from.as_str(), "to": to.as_str(), "until": until, "units": units})).into_response()
}
