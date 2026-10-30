// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-READ-SFUS · 운영:RULE-READ-ROOMS · 운영:RULE-READ-USERS · 운영:RULE-READ-SNAPSHOT · 운영:RULE-READ-DROPS · 운영:RULE-READ-TRANSIT · 운영:RULE-READ-ROOM-DETAIL · 운영:RULE-READ-NODE-ROOMS · 운영:RULE-READ-BUS · 운영:RULE-READ-BUS-ALL · 운영:RULE-READ-USER-MEDIA · 운영:RULE-READ-USER-PROBE · 운영:RULE-CTL-SUPERVISOR · 운영:RULE-CTL-CUT · 운영:RULE-CTL-ROOM · 운영:RULE-CTL-LOG-LEVEL · 운영:RULE-FMT-NO-INVENT · 정:IMPL-OPS-CUT · 정:IMPL-OPS-DESTROY-REAP · model: claude-opus-5-5
//! `/admin` — 읽기(유닛 · 방 · 세션 · 버림 · 체류 · 버스) · 제어(supervisor · 절단 · 퇴장 · 폭파 · 수준). 정본은 sfud 를 경유해 가져온다 · 못 닿은 곳은 `error`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::envelope::{CallKind, Envelope, Reply};
use common::policy::LoggingLevel;
use oxsig::{Code, Failure};
use serde_json::{Value, json};

use super::Hub;
use super::supervisor::{Role, UnitState};
use crate::ses;

pub type AdminResult = Result<Value, Failure>;

fn admin_env(method: &str, path: &str, body: Value) -> Envelope {
    let mut e = Envelope::new(CallKind::Admin);
    e.method = Some(method.to_string());
    e.path = Some(path.to_string());
    e.body = body;
    e
}

async fn sfu_admin(hub: &Hub, node: &str, method: &str, path: &str, body: Value) -> Result<Value, Failure> {
    match hub.bus.call(node, &admin_env(method, path, body)).await {
        Reply::Ok { body } => Ok(body),
        Reply::Http { status, body } if status < 300 => Ok(body),
        Reply::Http { body, .. } => Err(serde_json::from_value::<Failure>(body).unwrap_or_else(|_| Code::SfuError.failure())),
        Reply::Fail { failure } => Err(failure),
    }
}

fn unit_row(hub: &Hub, u: &super::supervisor::UnitHandle) -> Value {
    let st = u.state();
    let mut o = json!({
        "unit": u.id,
        "unit_state": st.as_str(),
        "live": st == UnitState::Running,
        "node_live": hub.bus.view.lock().map(|v| v.node_live(&hub.node)).unwrap_or(false),
    });
    if let Some(e) = u.epoch() {
        o["sfu_id"] = json!(e);
    }
    if let Some(a) = &u.addr {
        o["addr"] = json!(a);
    }
    o
}

pub fn sfus(hub: &Hub) -> Value {
    let rows: Vec<Value> = hub.sup.sfu_units().iter().map(|u| unit_row(hub, u)).collect();
    json!({ "sfus": rows, "supervising": !hub.sup.units.is_empty() })
}

pub fn rooms(hub: &Hub) -> Value {
    let routing = hub.bus.view.lock().map(|v| v.routing.clone()).unwrap_or_default();
    let sessions = hub.ses.sessions();
    let rows: Vec<Value> = routing
        .iter()
        .map(|(r, loc)| {
            let ids: Vec<String> = sessions.iter().filter(|s| s.in_room(r) && !s.claims.hidden).map(|s| s.user_id.clone()).collect();
            json!({ "room_id": r, "node": loc.node, "sfu_id": loc.inst, "members": ids.len(), "member_ids": ids })
        })
        .collect();
    json!({ "total": rows.len(), "rooms": rows })
}

pub async fn users(hub: &Hub) -> Value {
    let mut rows = Vec::new();
    for s in hub.ses.sessions() {
        let mut links = Vec::new();
        for (node, _) in s.epochs() {
            if let Ok(v) = sfu_admin(hub, &node, "GET", &format!("/admin/users/{}/links", s.user_id), Value::Null).await
                && let Some(a) = v.get("links").and_then(|x| x.as_array())
            {
                links.extend(a.iter().cloned());
            }
        }
        rows.push(json!({
            "session": s.fp,
            "user_id": s.user_id,
            "participant_type": s.claims.participant_type,
            "hidden": s.claims.hidden,
            "links": links,
        }));
    }
    json!({ "total": rows.len(), "users": rows })
}

pub fn snapshot(hub: &Hub) -> Value {
    let (ready, _) = hub.ready();
    let rows: Vec<Value> = hub.sup.sfu_units().iter().map(|u| unit_row(hub, u)).collect();
    let rooms = hub.bus.view.lock().map(|v| v.routing.len()).unwrap_or(0);
    json!({
        "hub_id": hub.node,
        "ready": ready,
        "build": hub.build,
        "supervising": !hub.sup.units.is_empty(),
        "sfus": rows,
        "rooms": rooms,
        "users": hub.ses.sessions().len(),
    })
}

pub async fn drops(hub: &Hub) -> Value {
    let c = &hub.sender.counters;
    let mut mine = json!({
        "unit": "hub",
        "sfu_id": hub.inst,
        "notice_no_socket": hub.ses.counters.notice_no_socket.load(Ordering::Relaxed),
        "bus_ev_drop": hub.bus.ev_drop.load(Ordering::Relaxed),
        "log_queue_full": c.queue_full.load(Ordering::Relaxed),
        "log_spool_full": c.spool_full.load(Ordering::Relaxed),
    });
    if let Ok(codes) = hub.ses.counters.codes.lock() {
        for (k, v) in codes.iter() {
            mine[format!("code_{k}")] = json!(v);
        }
    }
    let mut units = vec![mine];
    units.extend(per_sfu_unit(hub, "/admin/drops").await);
    json!({ "units": units })
}

async fn per_sfu_unit(hub: &Hub, path: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for u in hub.sup.sfu_units() {
        if u.state() != UnitState::Running {
            out.push(json!({ "unit": u.id, "error": Code::SfuUnavailable.num() }));
            continue;
        }
        match sfu_admin(hub, &hub.node, "GET", path, Value::Null).await {
            Ok(mut v) => {
                if let Some(o) = v.as_object_mut() {
                    o.insert("unit".into(), json!(u.id));
                    if let Some(e) = u.epoch() {
                        o.insert("sfu_id".into(), json!(e));
                    }
                }
                out.push(v);
            }
            Err(f) => out.push(json!({ "unit": u.id, "error": f.code })),
        }
    }
    out
}

pub async fn transit(hub: &Hub) -> Value {
    json!({ "units": per_sfu_unit(hub, "/admin/transit").await })
}

pub async fn room_snapshot(hub: &Hub, room: &str) -> AdminResult {
    let loc = hub.bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    sfu_admin(hub, &loc.node, "GET", &format!("/admin/rooms/{room}/snapshot"), Value::Null).await
}

pub async fn sfu_rooms(hub: &Hub, id: &str) -> AdminResult {
    let u = hub
        .sup
        .sfu_units()
        .into_iter()
        .find(|u| u.id == id || u.epoch().as_deref() == Some(id))
        .ok_or_else(|| Code::PeerNotFound.failure().with_message("no such unit"))?;
    let epoch = u.epoch().ok_or_else(|| Code::SfuUnavailable.failure())?;
    let v = sfu_admin(hub, &hub.node, "GET", "/admin/rooms", Value::Null).await?;
    let rooms = v.get("rooms").cloned().unwrap_or(json!([]));
    let total = rooms.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(json!({ "unit": u.id, "sfu_id": epoch, "rooms": rooms, "total": total }))
}

pub async fn bus_view(hub: &Hub) -> Value {
    let peers: Vec<Value> = hub.bus.peers().await.into_iter().map(|(z, w)| json!({ "zid": z, "whatami": w, "links": [] })).collect();
    let v = match hub.bus.view.lock() {
        Ok(v) => v.clone(),
        Err(_) => return json!({}),
    };
    let mut nodes: Vec<Value> = v
        .nodes
        .iter()
        .filter(|(_, n)| n.live)
        .map(|(k, n)| {
            let mut o = json!({ "node": k, "inst": n.inst });
            if k == &hub.node {
                o["self"] = json!(true);
            }
            o
        })
        .collect();
    if v.self_declared && !v.nodes.contains_key(&hub.node) {
        nodes.push(json!({ "node": hub.node, "inst": hub.inst, "self": true }));
    }
    let rooms_seen: Vec<Value> = v
        .routing
        .iter()
        .filter(|(_, l)| l.node != hub.node)
        .map(|(r, l)| json!({ "room_id": r, "node": l.node, "inst": l.inst }))
        .collect();
    let isolated = v.isolated.map(|since| json!({ "since_ms": since, "held_rooms": v.held.iter().collect::<Vec<_>>() }));
    let last = v.last_event.as_ref().map(|e| json!({ "at_ms": e.at_ms, "kind": e.kind, "key": e.key }));
    json!({
        "node": hub.node,
        "inst": hub.inst,
        "mode": hub.mode,
        "namespace": hub.namespace,
        "listen": hub.bus.cfg_listen,
        "connect": hub.bus.cfg_connect,
        "peers": peers,
        "nodes": nodes,
        "rooms_seen": rooms_seen,
        "isolated": isolated,
        "last_event": last,
    })
}

pub async fn bus_all(hub: &Hub) -> Value {
    let mut rows = vec![json!({ "node": hub.node, "bus": bus_view(hub).await })];
    for (n, _) in hub.bus.live_nodes() {
        if n == hub.node {
            continue;
        }
        match hub.bus.query(&n, "bus", Vec::new()).await {
            Ok(b) => match serde_json::from_slice::<Value>(&b) {
                Ok(v) => rows.push(json!({ "node": n, "bus": v })),
                Err(_) => rows.push(json!({ "node": n, "error": Code::SfuError.num() })),
            },
            Err(c) => rows.push(json!({ "node": n, "error": c.num() })),
        }
    }
    json!({ "asked": hub.node, "nodes": rows })
}

pub async fn user_media(hub: &Hub, user: &str) -> AdminResult {
    let s = hub.ses.session_of_user(user).ok_or_else(|| Code::SessionNotFound.failure())?;
    let mut links = Vec::new();
    for (node, epoch) in s.epochs() {
        match sfu_admin(hub, &node, "GET", &format!("/admin/users/{user}/media"), Value::Null).await {
            Ok(v) => {
                if let Some(a) = v.get("links").and_then(|x| x.as_array()) {
                    links.extend(a.iter().cloned());
                }
            }
            Err(f) => links.push(json!({ "sfu_id": epoch, "error": f.code })),
        }
    }
    Ok(json!({ "user_id": user, "now": common::now_ms(), "links": links }))
}

pub async fn probe_local(hub: &Hub, user: &str) -> Vec<Value> {
    let Some(s) = hub.ses.session_of_user(user) else { return Vec::new() };
    let Some(c) = s.conn() else {
        return vec![json!({ "session": s.fp, "timeout": true })];
    };
    match crate::aux::probe(&hub.ses.probes, &c).await {
        Some(r) => vec![json!({ "session": s.fp, "result": r })],
        None => vec![json!({ "session": s.fp, "timeout": true })],
    }
}

pub async fn probe(hub: &Arc<Hub>, user: &str) -> AdminResult {
    let mut rows = probe_local(hub, user).await;
    for (node, _) in hub.bus.sessions_of_user(user).await {
        if node == hub.node {
            continue;
        }
        if let Ok(b) = hub.bus.query(&node, "probe", user.as_bytes().to_vec()).await
            && let Ok(v) = serde_json::from_slice::<Vec<Value>>(&b)
        {
            rows.extend(v);
        }
    }
    if rows.is_empty() {
        return Err(Code::SessionNotFound.failure());
    }
    Ok(json!({ "user_id": user, "sessions": rows }))
}

pub fn sup_status(hub: &Hub) -> Value {
    let units: Vec<Value> = hub
        .sup
        .units
        .iter()
        .map(|u| {
            let (restarts, next) = u.rec.lock().map(|r| (r.restarts, r.next_retry_at)).unwrap_or((0, None));
            let st = u.state();
            let mut o = json!({ "id": u.id, "kind": u.kind.as_str(), "order": u.order.max(0), "state": st.as_str(), "restarts": restarts });
            if st == UnitState::Down
                && let Some(t) = next
            {
                o["next_retry_ms"] = json!(t.saturating_duration_since(std::time::Instant::now()).as_millis() as u64);
            }
            if u.role == Role::Sfu
                && let Some(e) = u.epoch()
            {
                o["epoch"] = json!(e);
            }
            o
        })
        .collect();
    json!({ "units": units, "supervising": !hub.sup.units.is_empty() })
}

pub async fn sup_ctl(hub: &Hub, id: &str, which: &str, if_epoch: Option<&str>) -> AdminResult {
    let u = hub.sup.unit(id).ok_or_else(|| Code::PeerNotFound.failure().with_message("no such unit"))?;
    if which == "kill" && u.role == Role::Sfu {
        let want = if_epoch.ok_or_else(|| Code::MissingField.failure().with_message("if_epoch"))?;
        let have = u.epoch().unwrap_or_default();
        if want != have {
            return Err(Code::PreconditionFailed.failure().with_details(json!({ "expected": want, "actual": have })));
        }
    }
    let (from, to) = hub.sup.request(id, which).await.map_err(|e| Code::PreconditionFailed.failure().with_message(e))?;
    Ok(json!({ "id": id, "from": from.as_str(), "to": to.as_str() }))
}

pub fn shutdown(hub: &Hub, confirm: Option<&str>) -> AdminResult {
    let c = confirm.ok_or_else(|| Code::MissingField.failure().with_message("confirm"))?;
    if c != hub.node {
        return Err(Code::PreconditionFailed.failure().with_details(json!({ "expected": c, "actual": hub.node })));
    }
    let _ = hub.shutdown.send("admin shutdown".into());
    Ok(json!({ "accepted": true, "order": ["clients", "units", "hub"] }))
}

pub async fn cut(hub: &Arc<Hub>, user: &str) -> AdminResult {
    let mut cut = ses::cut(&hub.ses, user).await;
    for (node, _) in hub.bus.sessions_of_user(user).await {
        if node == hub.node {
            continue;
        }
        if let Ok(b) = hub.bus.query(&node, "cut", user.as_bytes().to_vec()).await
            && let Ok(v) = serde_json::from_slice::<Vec<String>>(&b)
        {
            cut.extend(v);
        }
    }
    if cut.is_empty() {
        return Err(Code::SessionNotFound.failure());
    }
    let rows: Vec<Value> = cut.iter().map(|s| json!({ "session": s, "code": Code::SessionRevoked.num() })).collect();
    Ok(json!({ "user_id": user, "cut": rows, "total": rows.len() }))
}

pub async fn reap(hub: &Hub, room: &str, user: &str) -> AdminResult {
    let loc = hub.bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    let mut e = admin_env("POST", &format!("/admin/rooms/{room}/reap/{user}"), Value::Null);
    e.target_user = Some(user.to_string());
    match hub.bus.call(&loc.node, &e).await {
        Reply::Ok { body } => Ok(body),
        Reply::Http { status, body } if status < 300 => Ok(body),
        Reply::Http { body, .. } => Err(serde_json::from_value(body).unwrap_or_else(|_| Code::SfuError.failure())),
        Reply::Fail { failure } => Err(failure),
    }
}

pub async fn destroy(hub: &Hub, room: &str, if_version: Option<&str>) -> AdminResult {
    let iv = if_version.ok_or_else(|| Code::MissingField.failure().with_message("if_version"))?;
    let loc = hub.bus.room_route(room).ok_or_else(|| Code::RoomNotFound.failure())?;
    let v = sfu_admin(hub, &loc.node, "POST", &format!("/admin/rooms/{room}/destroy"), json!({ "if_version": iv })).await?;
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while hub.bus.room_route(room).is_some() && tokio::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(v)
}

pub async fn log_level(hub: &Hub, body: &Value) -> AdminResult {
    let lvl = body
        .get("level")
        .and_then(|v| v.as_str())
        .and_then(LoggingLevel::parse)
        .ok_or_else(|| Code::InvalidPayload.failure().with_message("level"))?;
    let ttl = body.get("ttl_secs").and_then(|v| v.as_u64());
    let from = hub.sender.level(common::now_ms());
    let until = ttl.map(|t| common::now_ms() + t * 1000);
    hub.sender.set_level(lvl, until);
    let mut units = vec!["hub".to_string()];
    for u in hub.sup.sfu_units() {
        if u.state() != UnitState::Running {
            continue;
        }
        let mut e = Envelope::new(CallKind::LogLevel);
        e.body = json!({ "level": lvl.as_str(), "until": until });
        if matches!(hub.bus.call_local(&e).await, Reply::Ok { .. }) {
            units.push(u.id.clone());
        }
    }
    Ok(json!({ "from": from.as_str(), "to": lvl.as_str(), "until": until, "units": units }))
}
