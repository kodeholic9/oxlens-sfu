// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 정:IMPL-OPS-CUT · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-BUS-REQUEST · 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! C 평면 — `/healthz/*` · `/admin/*`(운영 규격서 §3 읽기 · §4 제어 · §5 확인 계약) · 자격(운영 토큰 · loopback).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::handle::{Call, Reply};
use oxsig::err::{ErrCode, Failure};
use serde_json::{json, Value};

use super::supervisor::{Supervisor, UState};
use crate::bus::Bus;
use crate::ses::Ses;

#[derive(Clone)]
pub struct Admin {
    pub bus: Arc<Bus>,
    pub ses: Arc<Ses>,
    pub sup: Arc<Supervisor>,
    pub build: String,
    pub node: String,
    pub shutdown: Arc<tokio::sync::Notify>,
}

type Q = Query<BTreeMap<String, String>>;

fn status(c: ErrCode) -> StatusCode {
    crate::ses::http::status_of(c)
}

fn fail(f: Failure) -> Response {
    let st = match f.err_code() {
        Some(ErrCode::MissingField) => StatusCode::BAD_REQUEST,
        Some(ErrCode::PreconditionFailed) => StatusCode::CONFLICT,
        Some(c) => status(c),
        None => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (st, Json(f)).into_response()
}

fn ok(v: Value) -> Response {
    (StatusCode::OK, Json(v)).into_response()
}

fn reply(r: Reply) -> Response {
    match r {
        Reply::Ok { body } => ok(body),
        Reply::Fail { failure } => fail(failure),
        Reply::Http { status, body } => (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(body)).into_response(),
    }
}

fn authorized(a: &Admin, peer: &SocketAddr, h: &HeaderMap) -> Result<(), Failure> {
    if peer.ip().is_loopback() {
        return Ok(());
    }
    let t = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or_else(|| Failure::new(ErrCode::TokenInvalid))?;
    crate::ses::token::verify_ops(&a.ses.sys, t.trim()).map(|_| ()).map_err(Failure::new)
}

macro_rules! guard {
    ($a:expr, $peer:expr, $h:expr) => {
        if let Err(f) = authorized(&$a, &$peer, &$h) {
            return (StatusCode::UNAUTHORIZED, Json(f)).into_response();
        }
    };
}

fn admin_call(path: &str, method: &str, query: BTreeMap<String, String>, body: Value) -> Call {
    Call::Admin { method: method.to_string(), path: path.to_string(), query, body }
}

async fn live() -> Response {
    ok(json!({"live": true}))
}

async fn ready(State(a): State<Admin>) -> Response {
    let (all, down) = a.sup.all_running();
    let ready = all && !a.bus.is_isolated();
    let st = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (st, Json(json!({"ready": ready, "build": a.build, "down": down}))).into_response()
}

fn sfus_view(a: &Admin) -> Vec<Value> {
    let live = a.bus.live_nodes();
    a.sup
        .sfu_units()
        .into_iter()
        .map(|(id, epoch, st, addr)| {
            let mut v = json!({"unit": id, "addr": addr, "unit_state": st, "live": st == UState::Running, "node_live": live.contains_key(&a.node)});
            if st == UState::Running
                && let Some(e) = epoch {
                    v["sfu_id"] = json!(e);
                }
            v
        })
        .collect()
}

async fn sfus(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    ok(json!({"sfus": sfus_view(&a), "supervising": true}))
}

async fn rooms_view(a: &Admin) -> Value {
    let mut out = Vec::new();
    let live = a.bus.live_nodes();
    let mut counts: BTreeMap<String, Value> = BTreeMap::new();
    for (node, r) in a.bus.fanout(&admin_call("/admin/rooms", "GET", BTreeMap::new(), Value::Null)).await {
        if let Reply::Ok { body } = r {
            for x in body.get("rooms").and_then(Value::as_array).cloned().unwrap_or_default() {
                if let Some(id) = x.get("room_id").and_then(Value::as_str) {
                    counts.insert(id.to_string(), x.clone());
                }
            }
        } else {
            out.push(json!({"node": node, "error": ErrCode::SfuUnavailable.code()}));
        }
    }
    for (room, p) in a.bus.rooms() {
        let c = counts.get(&room);
        out.push(json!({
            "room_id": room,
            "node": p.node,
            "sfu_id": p.inst,
            "members": c.and_then(|c| c.get("members")).cloned().unwrap_or(json!(0)),
            "member_ids": c.and_then(|c| c.get("member_ids")).cloned().unwrap_or(json!([])),
        }));
    }
    let _ = live;
    let total = out.len();
    json!({"rooms": out, "total": total})
}

async fn rooms(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    ok(rooms_view(&a).await)
}

async fn users_view(a: &Admin) -> Value {
    let mut users = Vec::new();
    for s in a.ses.list() {
        let mut links = Vec::new();
        let nodes: Vec<String> = s.nodes.lock().map(|n| n.iter().cloned().collect()).unwrap_or_default();
        for n in nodes {
            let r = a.bus.call(&n, &admin_call(&format!("/admin/links/{}", s.user_id), "GET", BTreeMap::new(), Value::Null)).await;
            if let Reply::Ok { body } = r {
                links.extend(body.get("links").and_then(Value::as_array).cloned().unwrap_or_default());
            }
        }
        users.push(json!({"session_id": s.sid, "user_id": s.user_id, "participant_type": s.v.participant_type, "hidden": s.v.hidden, "links": links}));
    }
    let total = users.len();
    json!({"users": users, "total": total})
}

async fn users(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    ok(users_view(&a).await)
}

async fn snapshot(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    let (all, _) = a.sup.all_running();
    ok(json!({
        "hub_id": a.node,
        "ready": all && !a.bus.is_isolated(),
        "build": a.build,
        "supervising": true,
        "sfus": sfus_view(&a),
        "rooms": a.bus.rooms().len(),
        "users": a.ses.count(),
    }))
}

async fn drops(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    let mut units = Vec::new();
    for (id, epoch, st, _) in a.sup.sfu_units() {
        if st != UState::Running {
            units.push(json!({"unit": id, "error": ErrCode::SfuUnavailable.code()}));
            continue;
        }
        let r = a.bus.call_local(&admin_call("/admin/drops", "GET", BTreeMap::new(), Value::Null)).await;
        let mut v = match r {
            Reply::Ok { body } => body,
            _ => json!({"error": ErrCode::SfuError.code()}),
        };
        v["unit"] = json!(id);
        v["sfu_id"] = json!(epoch);
        let lc = common::log::counts();
        v["log_dropped"] = json!(lc.queue_dropped.load(std::sync::atomic::Ordering::Relaxed) + lc.spool_dropped.load(std::sync::atomic::Ordering::Relaxed));
        units.push(v);
    }
    ok(json!({"units": units}))
}

async fn room_snapshot(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>) -> Response {
    guard!(a, p, h);
    reply(a.bus.call_room(&room, &admin_call(&format!("/admin/rooms/{room}/snapshot"), "GET", BTreeMap::new(), Value::Null)).await)
}

async fn sfu_rooms(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(sfu): Path<String>) -> Response {
    guard!(a, p, h);
    let node = a.bus.node_of_inst(&sfu).or_else(|| a.sup.sfu_units().into_iter().find(|(id, _, _, _)| id == &sfu).map(|_| a.node.clone()));
    let Some(node) = node else { return fail(Failure::new(ErrCode::SfuUnavailable)) };
    let rooms: Vec<Value> = a.bus.rooms_of_node(&node).into_iter().map(|r| json!({"room_id": r})).collect();
    let total = rooms.len();
    let inst = a.bus.live_nodes().get(&node).cloned();
    ok(json!({"unit": node, "sfu_id": inst, "rooms": rooms, "total": total}))
}

async fn bus(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Q) -> Response {
    guard!(a, p, h);
    if q.get("all").map(|v| v == "1").unwrap_or(false) {
        let mut nodes = Vec::new();
        for (n, _) in a.bus.live_nodes() {
            let v = if n == a.node { Some(a.bus.view().await) } else { a.bus.ask(&n, "bus", Vec::new()).await.and_then(|b| serde_json::from_slice(&b).ok()) };
            nodes.push(match v {
                Some(b) => json!({"node": n, "bus": b}),
                None => json!({"node": n, "error": ErrCode::SfuUnavailable.code()}),
            });
        }
        return ok(json!({"asked": a.node, "nodes": nodes}));
    }
    ok(a.bus.view().await)
}

async fn user_media(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(a, p, h);
    let mut nodes: Vec<String> = Vec::new();
    if let Some(s) = a.ses.by_user(&user) {
        nodes.extend(s.nodes.lock().map(|n| n.iter().cloned().collect::<Vec<_>>()).unwrap_or_default());
    } else {
        for (_, n, _) in a.bus.find_sess(a.bus.keys.sess_of_user(&user)).await {
            if let Some(b) = a.bus.ask(&n, "users", Vec::new()).await
                && let Ok(list) = serde_json::from_slice::<Vec<Value>>(&b) {
                    for u in list.iter().filter(|u| u.get("user_id").and_then(Value::as_str) == Some(user.as_str())) {
                        nodes.extend(u.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default().iter().filter_map(Value::as_str).map(str::to_string));
                    }
                }
        }
    }
    if nodes.is_empty() {
        nodes = a.bus.live_nodes().keys().cloned().collect();
    }
    let mut links = Vec::new();
    for n in nodes {
        match a.bus.call(&n, &admin_call(&format!("/admin/users/{user}/media"), "GET", BTreeMap::new(), Value::Null)).await {
            Reply::Ok { body } => links.extend(body.get("links").and_then(Value::as_array).cloned().unwrap_or_default()),
            Reply::Fail { failure } if failure.code == ErrCode::PeerNotFound.code() => {}
            _ => links.push(json!({"node": n, "error": ErrCode::SfuUnavailable.code()})),
        }
    }
    ok(json!({"user_id": user, "now": common::clock::wall_ms(), "links": links}))
}

async fn sup_status(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(a, p, h);
    ok(a.sup.status())
}

async fn sup_load(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(a, p, h);
    match a.sup.load(&id).await {
        Some((f, t)) => ok(json!({"id": id, "from": f, "to": t})),
        None => fail(Failure::new(ErrCode::PreconditionFailed).msg("Stopped · Blocked 만 올린다")),
    }
}

async fn sup_stop(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(a, p, h);
    match a.sup.stop(&id).await {
        Some((f, t)) => ok(json!({"id": id, "from": f, "to": t})),
        None => fail(Failure::new(ErrCode::PreconditionFailed)),
    }
}

async fn sup_kill(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>, Query(q): Q) -> Response {
    guard!(a, p, h);
    let Some(want) = q.get("if_epoch") else { return fail(Failure::new(ErrCode::MissingField).msg("if_epoch 필수")) };
    let have = a.sup.epoch_of(&id);
    if have.as_deref() != Some(want.as_str()) {
        return fail(Failure::new(ErrCode::PreconditionFailed).details(json!({"expected": want, "actual": have})));
    }
    match a.sup.kill(&id).await {
        Some((f, t)) => ok(json!({"id": id, "from": f, "to": t})),
        None => fail(Failure::new(ErrCode::PreconditionFailed)),
    }
}

async fn sup_shutdown(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Q) -> Response {
    guard!(a, p, h);
    let Some(c) = q.get("confirm") else { return fail(Failure::new(ErrCode::MissingField).msg("confirm 필수")) };
    if c != &a.node {
        return fail(Failure::new(ErrCode::PreconditionFailed).details(json!({"expected": c, "actual": a.node})));
    }
    a.shutdown.notify_one();
    ok(json!({"accepted": true, "order": ["clients", "units", "hub"]}))
}

async fn user_cut(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(a, p, h);
    let mut cut = a.ses.cut_user(&user).await;
    if cut.is_empty() {
        for (_, n, _) in a.bus.find_sess(a.bus.keys.sess_of_user(&user)).await {
            if n == a.node {
                continue;
            }
            if let Some(b) = a.bus.ask(&n, "cut", user.as_bytes().to_vec()).await {
                cut.extend(serde_json::from_slice::<Vec<Value>>(&b).unwrap_or_default());
            }
        }
    }
    if cut.is_empty() {
        return (StatusCode::NOT_FOUND, Json(json!({"user_id": user, "cut": [], "total": 0}))).into_response();
    }
    let total = cut.len();
    ok(json!({"user_id": user, "cut": cut, "total": total}))
}

async fn room_reap(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path((room, user)): Path<(String, String)>) -> Response {
    guard!(a, p, h);
    reply(a.bus.call_room(&room, &admin_call(&format!("/admin/rooms/{room}/reap/{user}"), "POST", BTreeMap::new(), Value::Null)).await)
}

async fn room_destroy(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>, Query(q): Q) -> Response {
    guard!(a, p, h);
    reply(a.bus.call_room(&room, &admin_call(&format!("/admin/rooms/{room}/destroy"), "POST", q, Value::Null)).await)
}

async fn log_level(State(a): State<Admin>, ConnectInfo(p): ConnectInfo<SocketAddr>, h: HeaderMap, body: axum::body::Bytes) -> Response {
    guard!(a, p, h);
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(level) = v.get("level").and_then(Value::as_str) else { return fail(Failure::new(ErrCode::MissingField).msg("level")) };
    let ttl = v.get("ttl_secs").and_then(Value::as_u64);
    let Some((from, to, until)) = common::log::override_level(level, ttl) else { return fail(Failure::new(ErrCode::InvalidPayload).msg("level")) };
    let mut units = Vec::new();
    for (id, _, st, _) in a.sup.sfu_units() {
        if st == UState::Running {
            let _ = a.bus.call_local(&Call::LogLevel { level: level.to_string(), ttl_secs: ttl }).await;
            units.push(id);
        }
    }
    ok(json!({"from": from, "to": to, "until": if until == 0 { Value::Null } else { json!(until) }, "units": units}))
}

pub fn router(a: Admin) -> Router {
    Router::new()
        .route("/healthz/live", get(live))
        .route("/healthz/ready", get(ready))
        .route("/admin/sfus", get(sfus))
        .route("/admin/rooms", get(rooms))
        .route("/admin/users", get(users))
        .route("/admin/snapshot", get(snapshot))
        .route("/admin/drops", get(drops))
        .route("/admin/rooms/:room_id/snapshot", get(room_snapshot))
        .route("/admin/sfus/:sfu_id/rooms", get(sfu_rooms))
        .route("/admin/bus", get(bus))
        .route("/admin/users/:user_id/media", get(user_media))
        .route("/admin/supervisor/status", get(sup_status))
        .route("/admin/supervisor/load/:id", post(sup_load))
        .route("/admin/supervisor/stop/:id", post(sup_stop))
        .route("/admin/supervisor/kill/:id", post(sup_kill))
        .route("/admin/supervisor/shutdown", post(sup_shutdown))
        .route("/admin/users/:user_id/cut", post(user_cut))
        .route("/admin/rooms/:room_id/reap/:user_id", post(room_reap))
        .route("/admin/rooms/:room_id/destroy", post(room_destroy))
        .route("/admin/log-level", post(log_level))
        .with_state(a)
}
