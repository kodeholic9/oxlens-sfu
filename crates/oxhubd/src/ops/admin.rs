// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SURFACE · 정:RULE-OPS-TOKEN · 정:IMPL-OPS-CUT · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-OBSERVE · 정:RULE-BUS-REQUEST · 정:IMPL-BUS-FAULTS · 운영 규격서 §1 · 운영 규격서 §3 · 운영 규격서 §4 · 운영 규격서 §5 · 운영 규격서 §9 · model: claude-opus-5-5
//! C 평면 `/admin/*` — 자격(운영 토큰 · loopback · XFF 불신) · 읽기 평면 · 제어 평면 · 되돌릴 수 없는 셋의 확인값.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use common::bus::envelope::{Admin, Call, NodeQuery, Reply};
use common::token::{peek_issuer, verify_ops, TokenError};
use oxsig::{Code, Failure};
use serde::Deserialize;

use super::http::{fail_resp, json_resp};
use super::supervisor::{Ctl, ZENOH_UNIT};
use super::App;

type Q = Query<BTreeMap<String, String>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/admin/sfus", get(sfus))
        .route("/admin/sfus/:sfu/rooms", get(sfu_rooms))
        .route("/admin/rooms", get(rooms))
        .route("/admin/rooms/:room/snapshot", get(room_snapshot))
        .route("/admin/rooms/:room/reap/:user", post(reap))
        .route("/admin/rooms/:room/destroy", post(destroy))
        .route("/admin/users", get(users))
        .route("/admin/users/:user/media", get(user_media))
        .route("/admin/users/:user/cut", post(cut))
        .route("/admin/snapshot", get(snapshot))
        .route("/admin/drops", get(drops))
        .route("/admin/bus", get(bus))
        .route("/admin/supervisor/status", get(sup_status))
        .route("/admin/supervisor/load/:id", post(sup_load))
        .route("/admin/supervisor/stop/:id", post(sup_stop))
        .route("/admin/supervisor/kill/:id", post(sup_kill))
        .route("/admin/supervisor/shutdown", post(sup_shutdown))
        .route("/admin/log-level", post(log_level))
}

fn authorize(app: &App, peer: &SocketAddr, headers: &HeaderMap) -> Result<(), Failure> {
    if peer.ip().is_loopback() {
        return Ok(());
    }
    let tok = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim).ok_or_else(|| Code::TokenInvalid.failure())?;
    let iss = peek_issuer(tok).ok_or_else(|| Code::TokenInvalid.failure())?;
    let acct = app.boot.system.hub.auth.api_keys.iter().find(|a| a.key == iss).ok_or_else(|| Code::InvalidApiKey.failure())?;
    let claims = verify_ops(tok, &acct.secret, common::token::now_s()).map_err(|e| match e {
        TokenError::Expired => Code::TokenExpired.failure(),
        _ => Code::TokenInvalid.failure(),
    })?;
    if !claims.ops || claims.sub.is_some() || !acct.ops_allowed {
        return Err(Code::NotAuthorized.failure());
    }
    Ok(())
}

fn admin_fail(app: &App, f: &Failure) -> Response {
    if [2002, 2003, 2004, 2006].contains(&f.code) {
        app.ctx.counters.bump(Code::from_number(f.code).unwrap_or(Code::TokenInvalid));
        return json_resp(StatusCode::UNAUTHORIZED, oxsig::parse::encode(f));
    }
    fail_resp(app, f)
}

fn ok(v: serde_json::Value) -> Response {
    json_resp(StatusCode::OK, serde_json::to_vec(&v).unwrap_or_default())
}

fn not_found(what: &str) -> Response {
    json_resp(StatusCode::NOT_FOUND, serde_json::to_vec(&serde_json::json!({"error": what})).unwrap_or_default())
}

macro_rules! guard {
    ($app:expr, $peer:expr, $h:expr) => {
        if let Err(f) = authorize(&$app, &$peer, &$h) {
            return admin_fail(&$app, &f);
        }
    };
}

async fn admin_call(app: &App, node: &str, what: Admin) -> Result<serde_json::Value, Failure> {
    match app.ctx.bus.call(node, &Call::Admin { what }).await {
        Reply::Ok(v) => Ok(v),
        Reply::Fail(f) => Err(f),
    }
}

fn sfu_rows(app: &App) -> Vec<serde_json::Value> {
    let node_live = app.ctx.bus.node_live(&app.boot.node_id);
    app.sup
        .of_role(common::system::UnitRole::Sfu)
        .iter()
        .map(|u| {
            let st = u.state();
            let mut v = serde_json::json!({"unit": u.cfg.id, "addr": u.cfg.addr, "unit_state": st.name(), "live": st == super::sup_core::UnitState::Running, "node_live": node_live});
            if st == super::sup_core::UnitState::Running
                && let Some(e) = u.epoch()
            {
                v["sfu_id"] = serde_json::Value::String(e);
            }
            v
        })
        .collect()
}

async fn sfus(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    ok(serde_json::json!({"sfus": sfu_rows(&app), "supervising": !app.sup.units.is_empty()}))
}

async fn rooms(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    let map: Vec<(String, String, String)> = app.ctx.bus.rooms.lock().map(|m| m.rooms.iter().filter_map(|(r, s)| s.iter().next().map(|(n, i)| (r.clone(), n.clone(), i.clone()))).collect()).unwrap_or_default();
    let mut members: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for node in app.ctx.bus.alive_nodes() {
        if let Ok(v) = admin_call(&app, &node, Admin::Rooms).await {
            for r in v["rooms"].as_array().cloned().unwrap_or_default() {
                if let Some(id) = r["room_id"].as_str() {
                    members.insert(id.to_string(), r.clone());
                }
            }
        }
    }
    let rows: Vec<serde_json::Value> = map
        .iter()
        .map(|(r, n, i)| {
            let m = members.get(r);
            serde_json::json!({"room_id": r, "node": n, "sfu_id": i, "members": m.map(|x| x["members"].clone()).unwrap_or(serde_json::Value::from(0)), "member_ids": m.map(|x| x["member_ids"].clone()).unwrap_or(serde_json::json!([]))})
        })
        .collect();
    ok(serde_json::json!({"total": rows.len(), "rooms": rows}))
}

async fn room_snapshot(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>) -> Response {
    guard!(app, peer, h);
    let Some(node) = app.ctx.bus.room_node(&room) else { return fail_resp(&app, &Code::RoomNotFound.failure()) };
    match admin_call(&app, &node, Admin::RoomSnapshot { room_id: room }).await {
        Ok(v) => ok(v),
        Err(f) => fail_resp(&app, &f),
    }
}

async fn sfu_rooms(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(sfu): Path<String>) -> Response {
    guard!(app, peer, h);
    let unit = app.sup.of_role(common::system::UnitRole::Sfu).into_iter().find(|u| u.cfg.id == sfu || u.epoch().as_deref() == Some(sfu.as_str()));
    let Some(u) = unit else { return not_found("unit") };
    match admin_call(&app, &app.boot.node_id, Admin::Rooms).await {
        Ok(v) => {
            let rooms: Vec<serde_json::Value> = v["rooms"].as_array().cloned().unwrap_or_default().into_iter().map(|r| serde_json::json!({"room_id": r["room_id"], "members": r["members"]})).collect();
            ok(serde_json::json!({"unit": u.cfg.id, "sfu_id": u.epoch(), "total": rooms.len(), "rooms": rooms}))
        }
        Err(f) => ok(serde_json::json!({"unit": u.cfg.id, "sfu_id": u.epoch(), "error": f.code})),
    }
}

async fn users(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    let mut links: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    let nodes: std::collections::BTreeSet<String> = app.ctx.sessions.all().iter().flat_map(|s| s.lock().servers.keys().cloned().collect::<Vec<_>>()).collect();
    for node in nodes {
        if let Ok(v) = admin_call(&app, &node, Admin::Users).await {
            for u in v["users"].as_array().cloned().unwrap_or_default() {
                if let Some(id) = u["user_id"].as_str() {
                    links.entry(id.to_string()).or_default().extend(u["links"].as_array().cloned().unwrap_or_default());
                }
            }
        }
    }
    let rows: Vec<serde_json::Value> = app
        .ctx
        .sessions
        .all()
        .iter()
        .map(|s| user_row(s, links.get(&s.id.user).cloned().unwrap_or_default()))
        .collect();
    ok(serde_json::json!({"total": rows.len(), "users": rows}))
}

fn user_row(s: &crate::ses::Session, links: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({"session": s.fp, "user_id": s.id.user, "participant_type": s.id.participant_type, "hidden": s.id.hidden, "links": links})
}

async fn user_media(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(app, peer, h);
    let mut nodes: std::collections::BTreeSet<String> = app.ctx.sessions.of_user(&user).iter().flat_map(|s| s.lock().servers.keys().cloned().collect::<Vec<_>>()).collect();
    if nodes.is_empty() {
        nodes.extend(app.ctx.bus.alive_nodes());
    }
    let mut out = Vec::new();
    for node in nodes {
        match admin_call(&app, &node, Admin::UserMedia { user_id: user.clone() }).await {
            Ok(v) => out.extend(v["links"].as_array().cloned().unwrap_or_default()),
            Err(f) => out.push(serde_json::json!({"node": node, "error": f.code})),
        }
    }
    ok(serde_json::json!({"user_id": user, "now": common::time::wall_ms(), "links": out}))
}

async fn cut(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(user): Path<String>) -> Response {
    guard!(app, peer, h);
    let mut cut: Vec<String> = crate::ses::route::cut_local(&app.ctx, &user).await;
    for (node, _) in app.ctx.bus.sess_of_user(&user).await {
        if node == app.boot.node_id {
            continue;
        }
        if let Some(b) = app.ctx.bus.query(&node, "cut", &NodeQuery::Cut { user_id: user.clone() }, crate::bus::NODE_CALL_TIMEOUT).await
            && let Ok(list) = serde_json::from_slice::<Vec<String>>(&b)
        {
            cut.extend(list);
        }
    }
    if cut.is_empty() {
        return not_found("user not connected");
    }
    let rows: Vec<serde_json::Value> = cut.iter().map(|s| serde_json::json!({"session": s, "code": Code::SessionRevoked.number()})).collect();
    tracing::info!(kind = "ev", spec = "OP-PATH-ADMIN-USERS-USER-ID-CUT", user = %user, total = rows.len(), "cut");
    ok(serde_json::json!({"user_id": user, "cut": rows, "total": cut.len()}))
}

async fn reap(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path((room, user)): Path<(String, String)>) -> Response {
    guard!(app, peer, h);
    let Some(node) = app.ctx.bus.room_node(&room) else { return fail_resp(&app, &Code::RoomNotFound.failure()) };
    match admin_call(&app, &node, Admin::Reap { room_id: room, user_id: user }).await {
        Ok(v) => ok(v),
        Err(f) if f.code == Code::NotInRoom.number() => not_found("user not in room"),
        Err(f) => fail_resp(&app, &f),
    }
}

async fn destroy(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(room): Path<String>, Query(q): Q) -> Response {
    guard!(app, peer, h);
    let Some(v) = q.get("if_version").cloned() else { return fail_resp(&app, &Code::MissingField.failure_with("if_version", None)) };
    let Some(node) = app.ctx.bus.room_node(&room) else { return fail_resp(&app, &Code::RoomNotFound.failure()) };
    match admin_call(&app, &node, Admin::Destroy { room_id: room, if_version: Some(v) }).await {
        Ok(v) => ok(v),
        Err(f) => fail_resp(&app, &f),
    }
}

async fn snapshot(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    let (ready, _) = super::http::ready_body(&app);
    let rooms = app.ctx.bus.rooms.lock().map(|m| m.len()).unwrap_or(0);
    ok(serde_json::json!({"hub_id": app.boot.node_id, "ready": ready, "build": app.boot.build, "supervising": !app.sup.units.is_empty(), "sfus": sfu_rows(&app), "rooms": rooms, "users": app.ctx.sessions.len()}))
}

async fn drops(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    let mut units = Vec::new();
    let mut hub = serde_json::json!({"unit": "hub", "node": app.boot.node_id});
    for (k, v) in app.ctx.counters.snapshot() {
        hub[k] = serde_json::Value::from(v);
    }
    units.push(hub);
    for u in app.sup.of_role(common::system::UnitRole::Sfu) {
        match admin_call(&app, &app.boot.node_id, Admin::Drops).await {
            Ok(mut v) => {
                v["unit"] = serde_json::Value::String(u.cfg.id.clone());
                if let Some(e) = u.epoch() {
                    v["sfu_id"] = serde_json::Value::String(e);
                }
                units.push(v);
            }
            Err(f) => units.push(serde_json::json!({"unit": u.cfg.id, "error": f.code})),
        }
    }
    ok(serde_json::json!({"units": units}))
}

async fn bus(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Q) -> Response {
    guard!(app, peer, h);
    if q.get("all").map(|v| v == "1").unwrap_or(false) {
        let mut nodes = vec![serde_json::json!({"node": app.boot.node_id, "bus": app.ctx.bus.info().await})];
        let known: Vec<String> = app.ctx.bus.nodes.lock().map(|n| n.nodes.keys().cloned().collect()).unwrap_or_default();
        for node in known {
            if node == app.boot.node_id {
                continue;
            }
            if !app.ctx.bus.node_live(&node) {
                nodes.push(serde_json::json!({"node": node, "error": Code::SfuUnavailable.number()}));
                continue;
            }
            match app.ctx.bus.query(&node, "bus", &NodeQuery::Bus, crate::bus::NODE_CALL_TIMEOUT).await.and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
                Some(v) => nodes.push(serde_json::json!({"node": node, "bus": v})),
                None => nodes.push(serde_json::json!({"node": node, "error": Code::SfuError.number()})),
            }
        }
        return ok(serde_json::json!({"asked": app.boot.node_id, "nodes": nodes}));
    }
    ok(app.ctx.bus.info().await)
}

async fn sup_status(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap) -> Response {
    guard!(app, peer, h);
    ok(app.sup.status())
}

async fn sup_load(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(app, peer, h);
    let Some(u) = app.sup.find(&id) else { return not_found("unit") };
    let (tx, rx) = tokio::sync::oneshot::channel();
    u.send(Ctl::Load(tx));
    match rx.await {
        Ok(Ok((from, to))) => ok(serde_json::json!({"id": id, "from": from, "to": to})),
        Ok(Err(why)) => json_resp(StatusCode::CONFLICT, serde_json::to_vec(&serde_json::json!({"id": id, "from": u.state().name(), "to": u.state().name(), "refused": why})).unwrap_or_default()),
        Err(_) => fail_resp(&app, &Code::InternalError.failure()),
    }
}

async fn sup_stop(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>) -> Response {
    guard!(app, peer, h);
    if id == ZENOH_UNIT {
        super::begin_shutdown(app.clone(), "zenoh unit stopped — the node goes down");
        return ok(serde_json::json!({"id": id, "from": "Running", "to": "Stopped"}));
    }
    let Some(u) = app.sup.find(&id) else { return not_found("unit") };
    let (tx, rx) = tokio::sync::oneshot::channel();
    u.send(Ctl::Stop(tx));
    match rx.await {
        Ok((from, to)) => ok(serde_json::json!({"id": id, "from": from, "to": to})),
        Err(_) => fail_resp(&app, &Code::InternalError.failure()),
    }
}

async fn sup_kill(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Path(id): Path<String>, Query(q): Q) -> Response {
    guard!(app, peer, h);
    let Some(want) = q.get("if_epoch").cloned() else { return fail_resp(&app, &Code::MissingField.failure_with("if_epoch", None)) };
    if id == ZENOH_UNIT {
        super::begin_shutdown(app.clone(), "zenoh unit killed — Blocked, the node goes down");
        return ok(serde_json::json!({"id": id, "from": "Running", "to": "Blocked"}));
    }
    let Some(u) = app.sup.find(&id) else { return not_found("unit") };
    let actual = u.epoch().unwrap_or_default();
    if actual != want {
        return fail_resp(&app, &Code::PreconditionFailed.failure_with("if_epoch mismatch", Some(serde_json::json!({"expected": actual, "actual": want}))));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    u.send(Ctl::Kill(tx));
    match rx.await {
        Ok((from, _)) => ok(serde_json::json!({"id": id, "from": from, "to": "Down"})),
        Err(_) => fail_resp(&app, &Code::InternalError.failure()),
    }
}

async fn sup_shutdown(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, Query(q): Q) -> Response {
    guard!(app, peer, h);
    let Some(c) = q.get("confirm").cloned() else { return fail_resp(&app, &Code::MissingField.failure_with("confirm", None)) };
    if c != app.boot.node_id {
        return fail_resp(&app, &Code::PreconditionFailed.failure_with("confirm mismatch", Some(serde_json::json!({"expected": app.boot.node_id, "actual": c}))));
    }
    super::begin_shutdown(app.clone(), "admin shutdown");
    ok(serde_json::json!({"accepted": true, "order": ["clients", "units", "hub"]}))
}

#[derive(Deserialize)]
struct LevelReq {
    level: String,
    #[serde(default)]
    ttl_secs: Option<u64>,
}

async fn log_level(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, body: Bytes) -> Response {
    guard!(app, peer, h);
    let req: LevelReq = match oxsig::parse::body(&body) {
        Ok(r) => r,
        Err(e) => return fail_resp(&app, &e.code().failure_with(e.message(), None)),
    };
    let Some(to) = common::policy::LoggingLevel::parse(&req.level) else { return fail_resp(&app, &Code::InvalidPayload.failure_with("level", None)) };
    let (from, until) = common::log::override_level(to, req.ttl_secs);
    let mut units = vec!["hub".to_string()];
    for u in app.sup.of_role(common::system::UnitRole::Sfu) {
        if let Reply::Ok(_) = app.ctx.bus.call_local(&Call::LogLevel { level: req.level.clone(), ttl_secs: req.ttl_secs }).await {
            units.push(u.cfg.id.clone());
        }
    }
    tracing::info!(kind = "ev", spec = "OP-PATH-ADMIN-LOG-LEVEL", from = from.as_str(), to = to.as_str(), "log level overridden");
    ok(serde_json::json!({"from": from.as_str(), "to": to.as_str(), "until": until, "units": units}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_rows_carry_the_fingerprint_not_the_session_id() {
        let sid = "0123456789abcdef0123456789abcdef";
        let id = crate::ses::Identity { user: "u".into(), participant_type: 0, hidden: false, metadata: None, permission: None };
        let s = crate::ses::Session::new(sid.into(), id, oxsig::PcMode::V2pc, common::bus::envelope::Home { node: "n".into(), inst: "i".into() }, crate::ses::State::default());
        let row = user_row(&s, Vec::new());
        assert_eq!(row["session"], common::identity::fingerprint(sid), "연:DEV-TRN-SESSION-SECRET /admin/users 의 session 은 지문");
        assert!(!row.to_string().contains(sid), "원값이 어디에도 실리지 않는다");
    }
}
