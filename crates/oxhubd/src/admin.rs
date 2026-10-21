// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 운영 규격서 §1 · §2 · §3 · §4 · §5 · §9 · 정:RULE-OPS-SURFACE 정:RULE-OPS-TOKEN 정:IMPL-OPS-CUT 정:IMPL-OPS-DESTROY-REAP · model: claude-opus-5-5
//! C 평면 — 읽기 다섯(+방 상세 · 노드 방 · 버스) · 제어(supervisor · cut · reap · destroy · log-level). 모르는 것은 지어내지 않는다.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use oxsig::Code;
use oxsig::bplane::{AdminReq, NodeReq, NodeRes, UnitReq, UnitRes};
use serde::Deserialize;

use crate::http::{fail_code, failure};
use crate::state::Hub;
use crate::supervisor::UnitState;

fn authorize(hub: &Hub, peer: &SocketAddr, headers: &HeaderMap) -> Result<(), Code> {
    if peer.ip().is_loopback() {
        return Ok(());
    }
    let Some(tok) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) else {
        return Err(Code::TOKEN_INVALID);
    };
    crate::auth::verify_ops(&hub.system, tok)
}

macro_rules! guard {
    ($hub:expr, $peer:expr, $headers:expr) => {
        if let Err(c) = authorize(&$hub, &$peer, &$headers) {
            return failure(StatusCode::UNAUTHORIZED, c, None);
        }
    };
}

fn sfu_rows(hub: &Hub) -> Vec<serde_json::Value> {
    let my_live = hub.node_live(&hub.node);
    hub.units
        .views()
        .into_iter()
        .filter(|u| u.role == "sfu")
        .map(|u| {
            let mut v = serde_json::json!({
                "unit": u.id, "addr": u.addr, "unit_state": u.state,
                "live": u.state == UnitState::Running, "node_live": my_live,
            });
            if u.state == UnitState::Running
                && let Some(e) = &u.epoch {
                    v["sfu_id"] = serde_json::json!(e);
                }
            v
        })
        .collect()
}

pub async fn sfus(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    Json(serde_json::json!({ "sfus": sfu_rows(&hub), "supervising": hub.units.supervising() })).into_response()
}

pub async fn rooms(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    let places: Vec<(String, String, String)> = hub
        .rooms
        .read()
        .map(|m| m.iter().filter_map(|(r, v)| v.first().map(|p| (r.clone(), p.node.clone(), p.inst.clone()))).collect())
        .unwrap_or_default();
    let mut members: std::collections::HashMap<String, (u32, Vec<String>)> = std::collections::HashMap::new();
    for (_, r) in crate::route::fanout(&hub, UnitReq::Admin(AdminReq::Rooms)).await {
        if let Ok(UnitRes::AdminRooms(v)) = r {
            for a in v {
                members.insert(a.room_id.clone(), (a.members, a.member_ids));
            }
        }
    }
    let mut rows: Vec<serde_json::Value> = places
        .into_iter()
        .map(|(room, node, inst)| {
            let mut v = serde_json::json!({ "room_id": room, "node": node, "sfu_id": inst });
            if let Some((n, ids)) = members.get(&room) {
                v["members"] = serde_json::json!(n);
                v["member_ids"] = serde_json::json!(ids);
            }
            v
        })
        .collect();
    rows.sort_by(|a, b| a["room_id"].as_str().cmp(&b["room_id"].as_str()));
    let total = rows.len();
    Json(serde_json::json!({ "rooms": rows, "total": total })).into_response()
}

pub async fn users(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    let sessions: Vec<(String, String, u8, bool, Vec<String>)> = hub
        .sessions
        .lock()
        .map(|r| {
            r.by_sid
                .values()
                .map(|s| (s.sid.clone(), s.user_id.clone(), s.participant_type, s.hidden, s.units.iter().cloned().collect()))
                .collect()
        })
        .unwrap_or_default();
    let mut links: std::collections::HashMap<String, Vec<oxsig::bplane::AdminLink>> = std::collections::HashMap::new();
    let nodes: std::collections::BTreeSet<String> = sessions.iter().flat_map(|s| s.4.clone()).collect();
    for n in nodes {
        if let Ok(UnitRes::AdminUsers(v)) = crate::route::unit(&hub, &n, UnitReq::Admin(AdminReq::Users)).await {
            for u in v {
                links.entry(u.session_id.clone()).or_default().extend(u.links);
            }
        }
    }
    let rows: Vec<serde_json::Value> = sessions
        .into_iter()
        .map(|(sid, user, pt, hidden, _)| {
            serde_json::json!({ "session_id": sid, "user_id": user, "participant_type": pt, "hidden": hidden,
                                "links": links.remove(&sid).unwrap_or_default() })
        })
        .collect();
    let total = rows.len();
    Json(serde_json::json!({ "users": rows, "total": total })).into_response()
}

pub async fn snapshot(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    let (ready, _) = crate::http::ready_state(&hub);
    let rooms = hub.rooms.read().map(|m| m.len()).unwrap_or(0);
    let users = hub.sessions.lock().map(|r| r.by_sid.len()).unwrap_or(0);
    Json(serde_json::json!({
        "hub_id": hub.node, "ready": ready, "build": hub.build, "supervising": hub.units.supervising(),
        "sfus": sfu_rows(&hub), "rooms": rooms, "users": users,
    }))
    .into_response()
}

pub async fn drops(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    let mut units = Vec::new();
    for u in hub.units.views().into_iter().filter(|u| u.role == "sfu") {
        let r = hub.units.call_unit(&u.id, &UnitReq::Admin(AdminReq::Drops), crate::route::CALL_TIMEOUT).await;
        match r {
            Ok(UnitRes::AdminJson(mut v)) => {
                v["unit"] = serde_json::json!(u.id);
                if let Some(e) = u.epoch {
                    v["sfu_id"] = serde_json::json!(e);
                }
                units.push(v);
            }
            Ok(_) => units.push(serde_json::json!({ "unit": u.id, "error": 5002 })),
            Err(c) => units.push(serde_json::json!({ "unit": u.id, "error": c.0 })),
        }
    }
    let codes: serde_json::Value = hub
        .code_counts
        .lock()
        .map(|m| m.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect::<serde_json::Map<_, _>>().into())
        .unwrap_or_default();
    Json(serde_json::json!({ "units": units, "codes": codes })).into_response()
}

pub async fn room_snapshot(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    guard!(hub, peer, headers);
    let Some(p) = hub.room_place(&room_id) else { return fail_code(Code::ROOM_NOT_FOUND) };
    match crate::route::unit(&hub, &p.node, UnitReq::Admin(AdminReq::RoomSnapshot { room_id })).await {
        Ok(UnitRes::AdminJson(v)) => Json(v).into_response(),
        Ok(UnitRes::Fail { code, message, .. }) => failure(StatusCode::NOT_FOUND, Code(code), Some(&message)),
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

pub async fn sfu_rooms(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(sfu): Path<String>,
) -> Response {
    guard!(hub, peer, headers);
    let local = hub.units.views().into_iter().find(|u| u.role == "sfu" && (u.id == sfu || u.epoch.as_deref() == Some(sfu.as_str())));
    let (unit, epoch, res) = match local {
        Some(u) => {
            let r = hub.units.call_unit(&u.id, &UnitReq::Admin(AdminReq::Rooms), crate::route::CALL_TIMEOUT).await;
            (u.id, u.epoch.unwrap_or_default(), r)
        }
        None => {
            let node = hub.node_by_inst(&sfu).or_else(|| hub.node_live(&sfu).then(|| sfu.clone()));
            let Some(node) = node else { return failure(StatusCode::NOT_FOUND, Code::SFU_UNAVAILABLE, Some("unknown sfu")) };
            let inst = hub.nodes.read().ok().and_then(|n| n.get(&node).map(|e| e.inst.clone())).unwrap_or_default();
            let r = crate::route::unit(&hub, &node, UnitReq::Admin(AdminReq::Rooms)).await;
            (node, inst, r)
        }
    };
    match res {
        Ok(UnitRes::AdminRooms(v)) => {
            let rows: Vec<serde_json::Value> =
                v.iter().map(|a| serde_json::json!({ "room_id": a.room_id, "members": a.members })).collect();
            let total = rows.len();
            Json(serde_json::json!({ "unit": unit, "sfu_id": epoch, "rooms": rows, "total": total })).into_response()
        }
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

#[derive(Deserialize)]
pub struct AllQuery {
    all: Option<u8>,
}

pub async fn bus_view(hub: &Arc<Hub>) -> serde_json::Value {
    let z = &hub.system.zenoh;
    let nodes: Vec<serde_json::Value> = hub
        .nodes
        .read()
        .map(|n| {
            n.iter()
                .filter(|(_, e)| e.gone_at.is_none())
                .map(|(k, e)| {
                    let mut v = serde_json::json!({ "node": k, "inst": e.inst });
                    if *k == hub.node {
                        v["self"] = serde_json::json!(true);
                    }
                    v
                })
                .collect()
        })
        .unwrap_or_default();
    let rooms_seen: Vec<serde_json::Value> = hub
        .rooms
        .read()
        .map(|m| {
            m.iter()
                .flat_map(|(r, v)| v.iter().map(move |p| (r.clone(), p.clone())))
                .filter(|(_, p)| p.node != hub.node)
                .map(|(r, p)| serde_json::json!({ "room_id": r, "node": p.node, "inst": p.inst }))
                .collect()
        })
        .unwrap_or_default();
    let mut peers = Vec::new();
    if let Some(bus) = hub.bus() {
        let routers: Vec<String> = bus.info().routers_zid().await.map(|z| z.to_string()).collect();
        for r in routers {
            peers.push(serde_json::json!({ "zid": r, "whatami": "router" }));
        }
    }
    let unplaced: Vec<String> = hub
        .interest
        .lock()
        .map(|m| m.keys().filter(|r| hub.room_place(r).is_none()).cloned().collect())
        .unwrap_or_default();
    serde_json::json!({
        "node": hub.node, "inst": hub.my_inst(), "mode": z.mode, "namespace": z.namespace,
        "listen": z.listen, "connect": z.connect, "peers": peers, "nodes": nodes, "rooms_seen": rooms_seen,
        "pending": { "rooms_unplaced": unplaced, "rooms_no_view": Vec::<String>::new() },
        "last_event": hub.last_event.lock().ok().and_then(|g| g.clone()),
        "isolated": hub.isolated.load(std::sync::atomic::Ordering::Relaxed),
        "duplicates": hub.dup_rooms.load(std::sync::atomic::Ordering::Relaxed),
    })
}

pub async fn bus(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<AllQuery>,
) -> Response {
    guard!(hub, peer, headers);
    if q.all != Some(1) {
        return Json(bus_view(&hub).await).into_response();
    }
    let names: Vec<String> = hub.nodes.read().map(|n| n.keys().cloned().collect()).unwrap_or_default();
    let mut rows = Vec::new();
    for n in names {
        if n == hub.node {
            rows.push(serde_json::json!({ "node": n, "bus": bus_view(&hub).await }));
            continue;
        }
        match crate::route::node_call(&hub, &n, &NodeReq::Bus).await {
            Ok(NodeRes::Bus(v)) => rows.push(serde_json::json!({ "node": n, "bus": v })),
            Ok(_) => rows.push(serde_json::json!({ "node": n, "error": 5002 })),
            Err(c) => rows.push(serde_json::json!({ "node": n, "error": c.0 })),
        }
    }
    Json(serde_json::json!({ "asked": hub.node, "nodes": rows })).into_response()
}

pub async fn sup_status(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    guard!(hub, peer, headers);
    let units: Vec<serde_json::Value> = hub
        .units
        .views()
        .into_iter()
        .map(|u| {
            serde_json::json!({ "id": u.id, "kind": u.kind, "order": u.order, "state": u.state, "restarts": u.restarts,
                                "next_retry_ms": u.next_retry_ms, "epoch": u.epoch })
        })
        .collect();
    Json(serde_json::json!({ "units": units, "supervising": hub.units.supervising() })).into_response()
}

fn transition(id: &str, from: UnitState, to: &str) -> Response {
    Json(serde_json::json!({ "id": id, "from": from, "to": to })).into_response()
}

pub async fn sup_load(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    guard!(hub, peer, headers);
    match hub.units.load(&id) {
        Ok(from) => transition(&id, from, "Starting"),
        Err(_) => failure(StatusCode::NOT_FOUND, Code::SFU_UNAVAILABLE, Some("unknown unit")),
    }
}

pub async fn sup_stop(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    guard!(hub, peer, headers);
    match hub.units.stop(&id) {
        Ok(from) => transition(&id, from, "Stopped"),
        Err(_) => failure(StatusCode::NOT_FOUND, Code::SFU_UNAVAILABLE, Some("unknown unit")),
    }
}

#[derive(Deserialize)]
pub struct KillQuery {
    if_epoch: Option<String>,
}

pub async fn sup_kill(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<KillQuery>,
) -> Response {
    guard!(hub, peer, headers);
    let Some(e) = q.if_epoch else { return failure(StatusCode::BAD_REQUEST, Code::MISSING_FIELD, Some("if_epoch")) };
    if id == crate::supervisor::ZENOH_UNIT {
        tracing::warn!(kind = "ev", spec = "OP-PATH-ADMIN-SUPERVISOR-KILL-ID", "zenoh unit killed — node goes down");
        let hub2 = hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            crate::shutdown(&hub2, 1).await;
        });
        return transition(&id, UnitState::Running, "Down");
    }
    match hub.units.kill(&id, &e) {
        Ok(from) => transition(&id, from, "Down"),
        Err(Code::PRECONDITION_FAILED) => {
            let actual = hub.units.epoch_of(&id);
            (
                StatusCode::CONFLICT,
                Json(Code::PRECONDITION_FAILED.failure_details("if_epoch", serde_json::json!({ "expected": e, "actual": actual }))),
            )
                .into_response()
        }
        Err(_) => failure(StatusCode::NOT_FOUND, Code::SFU_UNAVAILABLE, Some("unknown unit")),
    }
}

#[derive(Deserialize)]
pub struct ConfirmQuery {
    confirm: Option<String>,
}

pub async fn sup_shutdown(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<ConfirmQuery>,
) -> Response {
    guard!(hub, peer, headers);
    let Some(c) = q.confirm else { return failure(StatusCode::BAD_REQUEST, Code::MISSING_FIELD, Some("confirm")) };
    if c != hub.node {
        return (
            StatusCode::CONFLICT,
            Json(Code::PRECONDITION_FAILED.failure_details("confirm", serde_json::json!({ "expected": c, "actual": hub.node }))),
        )
            .into_response();
    }
    let hub2 = hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        crate::shutdown(&hub2, 0).await;
    });
    Json(serde_json::json!({ "accepted": true, "order": ["clients", "units", "hub"] })).into_response()
}

pub async fn user_cut(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Response {
    guard!(hub, peer, headers);
    let mut cut = crate::session::drop_user(&hub, &user, 2009, None).await;
    if let Some(bus) = hub.bus() {
        let keys = common::bus::live_keys(bus, &hub.keys.sess_by_user(&user), crate::route::CALL_TIMEOUT).await;
        let nodes: std::collections::BTreeSet<String> = keys
            .iter()
            .filter_map(|k| match common::keyspace::parse_token(hub.keys.prefix(), k) {
                Some(common::keyspace::Token::Sess { node, .. }) if node != hub.node => Some(node),
                _ => None,
            })
            .collect();
        for n in nodes {
            if let Ok(NodeRes::Cut(v)) =
                crate::route::node_call(&hub, &n, &NodeReq::DropUser { user_id: user.clone(), code: 2009, except_session: None }).await
            {
                cut.extend(v);
            }
        }
    }
    if cut.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    tracing::info!(kind = "ev", spec = "CP-CODE-SESSION-REVOKED", user = user.as_str(), "operator cut");
    let rows: Vec<serde_json::Value> = cut.iter().map(|s| serde_json::json!({ "session_id": s, "code": 2009 })).collect();
    let total = rows.len();
    Json(serde_json::json!({ "user_id": user, "cut": rows, "total": total })).into_response()
}

pub async fn reap(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((room_id, user_id)): Path<(String, String)>,
) -> Response {
    guard!(hub, peer, headers);
    let Some(p) = hub.room_place(&room_id) else { return fail_code(Code::ROOM_NOT_FOUND) };
    match crate::route::unit(&hub, &p.node, UnitReq::Admin(AdminReq::Reap { room_id, user_id })).await {
        Ok(UnitRes::Reaped(r)) => Json(r).into_response(),
        Ok(UnitRes::Fail { code: 3002, .. }) => StatusCode::NOT_FOUND.into_response(),
        Ok(UnitRes::Fail { code, message, .. }) => failure(StatusCode::NOT_FOUND, Code(code), Some(&message)),
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

#[derive(Deserialize)]
pub struct VersionQuery {
    if_version: Option<String>,
}

pub async fn destroy(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
    Query(q): Query<VersionQuery>,
) -> Response {
    guard!(hub, peer, headers);
    let Some(v) = q.if_version else { return failure(StatusCode::BAD_REQUEST, Code::MISSING_FIELD, Some("if_version")) };
    let Some(p) = hub.room_place(&room_id) else { return fail_code(Code::ROOM_NOT_FOUND) };
    match crate::route::unit(&hub, &p.node, UnitReq::Admin(AdminReq::Destroy { room_id, if_version: Some(v) })).await {
        Ok(UnitRes::Destroyed(r)) => Json(r).into_response(),
        Ok(UnitRes::Fail { code: 3009, message, details }) => (
            StatusCode::CONFLICT,
            Json(match details {
                Some(d) => Code::PRECONDITION_FAILED.failure_details(message, d),
                None => Code::PRECONDITION_FAILED.failure(),
            }),
        )
            .into_response(),
        Ok(UnitRes::Fail { code, message, .. }) => failure(StatusCode::NOT_FOUND, Code(code), Some(&message)),
        Ok(_) => fail_code(Code::SFU_ERROR),
        Err(c) => fail_code(c),
    }
}

#[derive(Deserialize)]
pub struct LogLevelReq {
    level: String,
    ttl_secs: Option<u64>,
}

pub async fn log_level(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LogLevelReq>,
) -> Response {
    guard!(hub, peer, headers);
    let (from, to) = match common::log::set_level(&req.level, req.ttl_secs) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, Code::INVALID_PAYLOAD, Some(&e)),
    };
    let mut ids = Vec::new();
    for u in hub.units.views().into_iter().filter(|u| u.role == "sfu" && u.state == UnitState::Running) {
        let r = hub
            .units
            .call_unit(&u.id, &UnitReq::Admin(AdminReq::LogLevel { level: req.level.clone(), ttl_secs: req.ttl_secs }), crate::route::CALL_TIMEOUT)
            .await;
        if r.is_ok() {
            ids.push(u.id);
        }
    }
    let until = req.ttl_secs.map(|t| common::ident::now_ms() + t * 1000);
    Json(serde_json::json!({ "from": from, "to": to, "until": until, "units": ids })).into_response()
}
