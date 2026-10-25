// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-SURFACE 정:RULE-OPS-TOKEN 정:IMPL-OPS-CUT 정:IMPL-OPS-DESTROY-REAP 정:RULE-OPS-OBSERVE 정:IMPL-AUX-TASK 운영:RULE-AUTH-THREE 운영:RULE-FMT-REPLY 운영:RULE-FMT-NO-INVENT 운영:RULE-ERR-ADMIN-CODES 운영:RULE-RISK-CONFIRM 운영:RULE-READ-SFUS 운영:RULE-READ-ROOMS 운영:RULE-READ-USERS 운영:RULE-READ-SNAPSHOT 운영:RULE-READ-DROPS 운영:RULE-READ-ROOM-DETAIL 운영:RULE-READ-NODE-ROOMS 운영:RULE-READ-BUS 운영:RULE-READ-BUS-ALL 운영:RULE-READ-USER-MEDIA 운영:RULE-READ-USER-PROBE 운영:RULE-CTL-SUPERVISOR 운영:RULE-CTL-CUT 운영:RULE-CTL-ROOM 운영:RULE-CTL-LOG-LEVEL 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 운영 표면 — `/admin/*`(운영 토큰 또는 loopback · XFF 불신) · `/healthz/*`(무인증) · 실패는 `Failure` 형 · HTTP 상태는 운영 코드 표.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use oxsig::envelope::{AdminCall, Call, Reply};
use oxsig::{Code, Failure};

use super::shape::*;
use crate::ops::hub::Hub;
use crate::ops::supervisor::{UState, ZENOH_UNIT};
use crate::ses::bind::HubQ;

fn admin_status(c: Code) -> StatusCode {
    let s = match c {
        Code::InvalidApiKey | Code::TokenInvalid | Code::TokenExpired | Code::NotAuthorized => 401,
        Code::MissingField | Code::InvalidPayload => 400,
        Code::PreconditionFailed => 409,
        Code::RoomNotFound | Code::SessionNotFound | Code::NotInRoom => 404,
        Code::SfuUnavailable => 503,
        Code::SfuError => 502,
        other => other.http_status(),
    };
    StatusCode::from_u16(s).unwrap_or(StatusCode::BAD_REQUEST)
}

pub fn afail(f: Failure) -> Response {
    let c = Code::from_num(f.code).unwrap_or(Code::InternalError);
    (admin_status(c), [(header::CONTENT_TYPE, "application/json")], serde_json::to_vec(&f).unwrap_or_default()).into_response()
}

fn afail_c(c: Code) -> Response {
    afail(c.failure())
}

fn ok<T: serde::Serialize>(v: &T) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], serde_json::to_vec(v).unwrap_or_default()).into_response()
}

fn ok_raw(b: String) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response()
}

pub async fn guard(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, h: HeaderMap, req: Request, next: Next) -> Response {
    if peer.ip().is_loopback() {
        return next.run(req).await;
    }
    let Some(t) = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")) else {
        return afail_c(Code::NotAuthorized);
    };
    match common::jwt::verify_ops(t.trim(), &hub.cfg.sys) {
        Ok(_) => next.run(req).await,
        Err(c) => afail_c(c),
    }
}

async fn sfu_rows(hub: &Hub) -> Vec<SfuRow> {
    let node_live = hub.bus.node_live(&hub.cfg.node_id);
    hub.sup
        .views()
        .await
        .into_iter()
        .filter(|u| u.role == Some(common::system::UnitRole::Sfu))
        .map(|u| SfuRow {
            unit: u.id,
            sfu_id: if u.state == UState::Running { u.epoch } else { None },
            addr: u.addr,
            unit_state: Some(u.state.name().to_string()),
            live: u.state == UState::Running,
            node_live,
        })
        .collect()
}

async fn sfus(State(hub): State<Arc<Hub>>) -> Response {
    ok(&SfusRes { sfus: sfu_rows(&hub).await, supervising: true })
}

#[derive(serde::Deserialize)]
struct NodeRooms {
    room_id: String,
    member_ids: Vec<String>,
}

async fn rooms(State(hub): State<Arc<Hub>>) -> Response {
    let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let nodes: BTreeSet<String> = hub.bus.rooms().into_iter().map(|(_, n, _)| n).collect();
    for n in nodes {
        if let Reply::Ok { body } = hub.call_node(&n, None, &Call::Admin { req: AdminCall::Rooms }).await
            && let Ok(v) = serde_json::from_str::<Vec<NodeRooms>>(&body)
        {
            for r in v {
                members.insert(r.room_id, r.member_ids);
            }
        }
    }
    let rows: Vec<RoomRow> = hub
        .bus
        .rooms()
        .into_iter()
        .map(|(r, n, i)| {
            let m = members.get(&r).cloned().unwrap_or_default();
            RoomRow { members: m.len() as u32, member_ids: m, room_id: r, node: n, sfu_id: i }
        })
        .collect();
    ok(&RoomsRes { total: rows.len() as u32, rooms: rows })
}

async fn links_of(hub: &Hub, users: Vec<String>, nodes: &BTreeSet<String>) -> BTreeMap<String, Vec<LinkRow>> {
    let mut out: BTreeMap<String, Vec<LinkRow>> = BTreeMap::new();
    for n in nodes {
        if let Reply::Ok { body } = hub.call_node(n, None, &Call::Admin { req: AdminCall::Links { users: users.clone() } }).await
            && let Ok(m) = serde_json::from_str::<BTreeMap<String, Vec<LinkRow>>>(&body)
        {
            for (u, l) in m {
                out.entry(u).or_default().extend(l);
            }
        }
    }
    out
}

pub async fn local_users(hub: &Hub) -> Vec<UserRow> {
    let ss = hub.ses.all();
    let mut nodes = BTreeSet::new();
    for s in &ss {
        nodes.extend(s.nodes().await);
    }
    let links = links_of(hub, ss.iter().map(|s| s.id.user.clone()).collect(), &nodes).await;
    ss.iter()
        .map(|s| UserRow {
            session: s.fp.clone(),
            user_id: s.id.user.clone(),
            participant_type: s.id.ptype,
            hidden: s.id.hidden,
            links: links.get(&s.id.user).cloned().unwrap_or_default(),
        })
        .collect()
}

async fn users(State(hub): State<Arc<Hub>>) -> Response {
    let u = local_users(&hub).await;
    ok(&UsersRes { total: u.len() as u32, users: u })
}

async fn snapshot(State(hub): State<Arc<Hub>>) -> Response {
    let (ready, _) = hub.sup.ready().await;
    ok(&SnapshotRes {
        hub_id: hub.cfg.node_id.clone(),
        ready,
        build: hub.cfg.build.clone(),
        supervising: true,
        sfus: sfu_rows(&hub).await,
        rooms: hub.bus.room_count() as u32,
        users: hub.ses.count() as u32,
    })
}

pub async fn hub_drops(hub: &Hub) -> DropsUnit {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for (c, n) in hub.codes.lock().await.iter() {
        counts.insert(format!("code_{c}"), *n);
    }
    let lc = hub.log.counters();
    counts.insert("log_queue_full".into(), lc.queue_full.load(Ordering::Relaxed));
    counts.insert("log_spool_full".into(), lc.spool_full.load(Ordering::Relaxed));
    counts.insert("notice_no_socket".into(), hub.ev_lost.load(Ordering::Relaxed));
    DropsUnit { unit: "hub".into(), sfu_id: None, error: None, counts }
}

async fn drops(State(hub): State<Arc<Hub>>) -> Response {
    let mut units = vec![hub_drops(&hub).await];
    for (id, epoch, client) in hub.sup.sfu_clients().await {
        let r = match client {
            Some(mut c) => common::handle::call(&mut c, &Call::Admin { req: AdminCall::Drops }).await,
            None => Reply::fail(Code::SfuUnavailable),
        };
        match r {
            Reply::Ok { body } => {
                let counts: BTreeMap<String, u64> = serde_json::from_str(&body).unwrap_or_default();
                units.push(DropsUnit { unit: id, sfu_id: epoch, error: None, counts });
            }
            Reply::Fail { failure } => units.push(DropsUnit { unit: id, sfu_id: None, error: Some(u32::from(failure.code)), counts: BTreeMap::new() }),
            Reply::Http { .. } => {}
        }
    }
    ok(&DropsRes { units })
}

fn pass(r: Reply) -> Response {
    match r {
        Reply::Ok { body } => ok_raw(body),
        Reply::Fail { failure } => afail(failure),
        Reply::Http { status, body } => {
            (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), [(header::CONTENT_TYPE, "application/json")], body).into_response()
        }
    }
}

async fn room_snapshot(State(hub): State<Arc<Hub>>, Path(room_id): Path<String>) -> Response {
    let Some((n, i)) = hub.bus.room_where(&room_id) else {
        return afail_c(Code::RoomNotFound);
    };
    pass(hub.call_node(&n, Some(&i), &Call::Admin { req: AdminCall::RoomSnapshot { room_id } }).await)
}

async fn sfu_rooms(State(hub): State<Arc<Hub>>, Path(sfu): Path<String>) -> Response {
    for (id, epoch, client) in hub.sup.sfu_clients().await {
        if id == sfu || epoch.as_deref() == Some(sfu.as_str()) {
            let Some(mut c) = client else {
                return afail_c(Code::SfuUnavailable);
            };
            return pass(common::handle::call(&mut c, &Call::Admin { req: AdminCall::SfuRooms }).await);
        }
    }
    afail(Code::RoomNotFound.failure_msg("그런 유닛 없음"))
}

pub async fn bus_body(hub: &Hub) -> BusBody {
    let me = hub.cfg.node_id.clone();
    let nodes = hub
        .bus
        .nodes()
        .into_iter()
        .map(|(n, i)| {
            let mut m = serde_json::Map::new();
            m.insert("node".into(), n.clone().into());
            m.insert("inst".into(), i.into());
            if n == me {
                m.insert("self".into(), true.into());
            }
            m
        })
        .collect();
    let rooms_seen = hub.bus.rooms().into_iter().filter(|(_, n, _)| *n != me).map(|(r, n, i)| SeenRow { room_id: r, node: n, inst: i }).collect();
    BusBody {
        node: me,
        inst: hub.inst.clone(),
        mode: hub.bus.mode.clone(),
        namespace: hub.bus.keys.ns.clone(),
        listen: hub.bus.listen.clone(),
        connect: hub.bus.connect.clone(),
        peers: hub.bus.peers().await.into_iter().map(|(z, w)| Peer { zid: z, whatami: w.into(), links: Vec::new() }).collect(),
        nodes,
        rooms_seen,
        pending: Pending { rooms_unplaced: Vec::new(), rooms_no_view: Vec::new() },
        last_event: hub.bus.last_event().map(|e| LastEv { at_ms: e.at_ms, kind: e.kind.into(), key: e.key }),
    }
}

#[derive(serde::Deserialize)]
struct BusQ {
    all: Option<String>,
}

async fn bus(State(hub): State<Arc<Hub>>, Query(q): Query<BusQ>) -> Response {
    if q.all.as_deref() != Some("1") {
        return ok(&bus_body(&hub).await);
    }
    let mut rows = vec![BusAllRow { node: hub.cfg.node_id.clone(), bus: Some(bus_body(&hub).await), error: None }];
    let others: BTreeSet<String> = hub.bus.nodes().into_iter().map(|(n, _)| n).filter(|n| *n != hub.cfg.node_id).collect();
    for n in others {
        let body = serde_json::to_vec(&HubQ::Bus).unwrap_or_default();
        match hub.bus.ask(&n, "hub", body, common::bus::NODE_CALL_TIMEOUT).await {
            Ok(b) => match serde_json::from_slice::<BusBody>(&b) {
                Ok(bb) => rows.push(BusAllRow { node: n, bus: Some(bb), error: None }),
                Err(_) => rows.push(BusAllRow { node: n, bus: None, error: Some(u32::from(Code::SfuError.num())) }),
            },
            Err(c) => rows.push(BusAllRow { node: n, bus: None, error: Some(u32::from(c.num())) }),
        }
    }
    ok(&BusAllRes { asked: hub.cfg.node_id.clone(), nodes: rows })
}

async fn user_nodes(hub: &Hub, user: &str) -> BTreeSet<String> {
    let mut nodes = BTreeSet::new();
    if let Some(s) = hub.ses.of_user(user) {
        nodes.extend(s.nodes().await);
    }
    if nodes.is_empty() {
        nodes.extend(hub.bus.live_nodes());
    }
    nodes
}

async fn user_media(State(hub): State<Arc<Hub>>, Path(user): Path<String>) -> Response {
    let mut links = Vec::new();
    for n in user_nodes(&hub, &user).await {
        match hub.call_node(&n, None, &Call::Admin { req: AdminCall::UserMedia { user_id: user.clone() } }).await {
            Reply::Ok { body } => {
                if let Ok(serde_json::Value::Array(a)) = serde_json::from_str::<serde_json::Value>(&body) {
                    links.extend(a);
                }
            }
            Reply::Fail { failure } => {
                if failure.code != Code::PeerNotFound.num() {
                    let mut m = serde_json::Map::new();
                    m.insert("sfu_id".into(), hub.bus.rooms().into_iter().find(|(_, nn, _)| *nn == n).map(|(_, _, i)| i).unwrap_or(n.clone()).into());
                    m.insert("error".into(), failure.code.into());
                    links.push(serde_json::Value::Object(m));
                }
            }
            Reply::Http { .. } => {}
        }
    }
    ok(&UserMediaRes { user_id: user, now: common::log::now_ms(), links })
}

pub async fn local_probe(hub: &Hub, user: &str) -> Vec<ProbeRow> {
    let mut out = Vec::new();
    if let Some(s) = hub.ses.of_user(user) {
        let r = hub.aux.ask(&s).await;
        out.push(ProbeRow { session: s.fp.clone(), timeout: if r.is_none() { Some(true) } else { None }, result: r });
    }
    out
}

async fn user_probe(State(hub): State<Arc<Hub>>, Path(user): Path<String>) -> Response {
    let mut rows = local_probe(&hub, &user).await;
    for (n, _) in hub.bus.sess_of_user(&user).await {
        if hub.is_local(&n) {
            continue;
        }
        let body = serde_json::to_vec(&HubQ::Probe { user: user.clone() }).unwrap_or_default();
        if let Ok(b) = hub.bus.ask(&n, "hub", body, common::bus::NODE_CALL_TIMEOUT).await
            && let Ok(v) = serde_json::from_slice::<Vec<ProbeRow>>(&b)
        {
            rows.extend(v);
        }
    }
    if rows.is_empty() {
        return afail_c(Code::SessionNotFound);
    }
    ok(&ProbeRes { user_id: user, sessions: rows })
}

async fn sup_status(State(hub): State<Arc<Hub>>) -> Response {
    let units = hub
        .sup
        .views()
        .await
        .into_iter()
        .map(|u| SupRow {
            id: u.id,
            kind: u.kind.as_str().into(),
            order: u.order,
            state: u.state.name().into(),
            restarts: u.restarts,
            next_retry_ms: u.next_retry_ms,
            epoch: u.epoch,
        })
        .collect();
    ok(&SupStatusRes { units, supervising: true })
}

async fn sup_load(State(hub): State<Arc<Hub>>, Path(id): Path<String>) -> Response {
    match hub.sup.load(&id).await {
        Ok(from) => ok(&SupMoveRes { id, from: from.name().into(), to: UState::Starting.name().into() }),
        Err(c) => afail(c.failure_msg(format!("unit {id}"))),
    }
}

async fn sup_stop(State(hub): State<Arc<Hub>>, Path(id): Path<String>) -> Response {
    match hub.sup.stop(&id).await {
        Ok((from, to)) => {
            if id == ZENOH_UNIT {
                hub.shutting_down.store(true, Ordering::Relaxed);
            }
            ok(&SupMoveRes { id, from: from.name().into(), to: to.name().into() })
        }
        Err(c) => afail(c.failure_msg(format!("unit {id}"))),
    }
}

#[derive(serde::Deserialize)]
struct KillQ {
    if_epoch: Option<String>,
}

async fn sup_kill(State(hub): State<Arc<Hub>>, Path(id): Path<String>, Query(q): Query<KillQ>) -> Response {
    let Some(e) = q.if_epoch else {
        return afail(Code::MissingField.failure_msg("if_epoch 없음"));
    };
    match hub.sup.kill(&id, &e).await {
        Ok(from) => ok(&SupMoveRes { id, from: from.name().into(), to: UState::Down.name().into() }),
        Err((Code::PreconditionFailed, actual)) => {
            afail(Code::PreconditionFailed.failure().with_details(serde_json::json!({ "expected": e, "actual": actual })))
        }
        Err((c, _)) => afail(c.failure_msg(format!("unit {id}"))),
    }
}

#[derive(serde::Deserialize)]
struct ConfirmQ {
    confirm: Option<String>,
}

async fn sup_shutdown(State(hub): State<Arc<Hub>>, Query(q): Query<ConfirmQ>) -> Response {
    let Some(c) = q.confirm else {
        return afail(Code::MissingField.failure_msg("confirm 없음"));
    };
    if c != hub.cfg.node_id {
        return afail(Code::PreconditionFailed.failure().with_details(serde_json::json!({ "expected": c, "actual": hub.cfg.node_id })));
    }
    let h = hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        super::begin_shutdown(&h);
    });
    ok(&ShutdownRes { accepted: true, order: vec!["clients", "units", "hub"] })
}

pub async fn local_cut(hub: &Arc<Hub>, user: &str) -> Vec<CutRow> {
    let mut out = Vec::new();
    if let Some(s) = hub.ses.of_user(user) {
        out.push(CutRow { session: s.fp.clone(), code: Code::SessionRevoked.num() });
        common::ev!(spec = "CP-CODE-SESSION-REVOKED", session = s.fp.as_str(), user = user, "operator cut");
        crate::ses::destroy(hub, &s, crate::ses::Gone::Cut, Some(Code::SessionRevoked)).await;
    }
    out
}

async fn user_cut(State(hub): State<Arc<Hub>>, Path(user): Path<String>) -> Response {
    let mut rows = local_cut(&hub, &user).await;
    for (n, _) in hub.bus.sess_of_user(&user).await {
        if hub.is_local(&n) {
            continue;
        }
        let body = serde_json::to_vec(&HubQ::Cut { user: user.clone() }).unwrap_or_default();
        if let Ok(b) = hub.bus.ask(&n, "hub", body, common::bus::NODE_CALL_TIMEOUT).await
            && let Ok(v) = serde_json::from_slice::<Vec<CutRow>>(&b)
        {
            rows.extend(v);
        }
    }
    if rows.is_empty() {
        return afail_c(Code::SessionNotFound);
    }
    ok(&CutRes { user_id: user, total: rows.len() as u32, cut: rows })
}

async fn room_reap(State(hub): State<Arc<Hub>>, Path((room_id, user_id)): Path<(String, String)>) -> Response {
    let Some((n, i)) = hub.bus.room_where(&room_id) else {
        return afail_c(Code::RoomNotFound);
    };
    pass(hub.call_node(&n, Some(&i), &Call::Admin { req: AdminCall::Reap { room_id, user_id } }).await)
}

#[derive(serde::Deserialize)]
struct VerQ {
    if_version: Option<String>,
}

async fn room_destroy(State(hub): State<Arc<Hub>>, Path(room_id): Path<String>, Query(q): Query<VerQ>) -> Response {
    let Some(v) = q.if_version else {
        return afail(Code::MissingField.failure_msg("if_version 없음"));
    };
    let Some((n, i)) = hub.bus.room_where(&room_id) else {
        return afail_c(Code::RoomNotFound);
    };
    pass(hub.call_node(&n, Some(&i), &Call::Admin { req: AdminCall::Destroy { room_id, if_version: Some(v) } }).await)
}

#[derive(serde::Deserialize)]
struct LevelReq {
    level: Option<String>,
    ttl_secs: Option<u32>,
}

async fn log_level(State(hub): State<Arc<Hub>>, body: Bytes) -> Response {
    let Ok(r) = serde_json::from_slice::<LevelReq>(&body) else {
        return afail_c(Code::InvalidPayload);
    };
    let Some(lv) = r.level else {
        return afail(Code::MissingField.failure_msg("level 없음"));
    };
    let Some(to) = common::log::parse_level(&lv) else {
        return afail(Code::InvalidPayload.failure_msg(format!("level {lv}")));
    };
    let ch = hub.log.override_level(to, r.ttl_secs);
    let mut units = Vec::new();
    for (id, _, client) in hub.sup.sfu_clients().await {
        if let Some(mut c) = client
            && let Reply::Ok { .. } = common::handle::call(&mut c, &Call::LogLevel { level: lv.clone(), ttl_secs: r.ttl_secs }).await
        {
            units.push(id);
        }
    }
    common::ev!(spec = "OP-PATH-ADMIN-LOG-LEVEL", "log level {} → {} ttl={:?}", common::log::level_name(ch.from), lv, r.ttl_secs);
    ok(&LogLevelRes { from: common::log::level_name(ch.from).into(), to: lv, until: ch.until, units })
}

async fn live() -> Response {
    ok(&LiveRes { live: true })
}

async fn ready(State(hub): State<Arc<Hub>>) -> Response {
    let (r, down) = hub.sup.ready().await;
    let ready = r && !hub.stopping();
    let body = serde_json::to_vec(&ReadyRes { ready, build: hub.cfg.build.clone(), down }).unwrap_or_default();
    (if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

pub fn routes(hub: Arc<Hub>) -> Router<Arc<Hub>> {
    let admin = Router::new()
        .route("/admin/sfus", get(sfus))
        .route("/admin/rooms", get(rooms))
        .route("/admin/users", get(users))
        .route("/admin/snapshot", get(snapshot))
        .route("/admin/drops", get(drops))
        .route("/admin/rooms/:room_id/snapshot", get(room_snapshot))
        .route("/admin/sfus/:sfu_id/rooms", get(sfu_rooms))
        .route("/admin/bus", get(bus))
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
        .route_layer(axum::middleware::from_fn_with_state(hub, guard));
    Router::new().route("/healthz/live", get(live)).route("/healthz/ready", get(ready)).merge(admin)
}
