// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:SES 정:IMPL-ROOM-JOIN ① 정:RULE-BUS-EVENTS 정:RULE-BUS-REQUEST 연:SES.op 연:RULE-SES-AUTH-TWO-WAYS 연:RULE-SES-EVICT-OLD 연:RULE-ROOM-JOIN-NUMBER-TAGS · model: claude-opus-5-5
//! 세션 — 게이트웨이가 든 클라 접속 하나. 폐기 경로는 하나(§17 순서는 미디어 서버가 돈다).

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use oxsig::bplane::{Caller, EndPath, Home, NodeReq, NodeRes, ResumePart, SessionData, UnitEvent, UnitReq, UnitRes};
use oxsig::wire::{
    Affiliation, AffiliationReq, BindReq, BindRes, IceRestartReq, MessageReq, PcMode, Permission, PublishTracksReq,
    ReadyReq, ResumeRes, RoomEventBody, RoomJoinReq, RoomLeaveReq, SubscribeLayerReq, TrackSetReq,
};
use oxsig::{Code, Frame, Op};

use crate::auth::{ALL_ALLOWED, Claims};
use crate::conn::ConnHandle;
use crate::route::{fail_frame, res_frame};
use crate::state::Hub;

pub struct Session {
    pub sid: String,
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
    pub pc_mode: PcMode,
    pub conn: Option<ConnHandle>,
    pub resumed: bool,
    pub dropped_at: Option<Instant>,
    pub rooms: HashMap<String, String>,
    pub units: BTreeSet<String>,
    pub user_sub: Option<tokio::task::JoinHandle<()>>,
    pub token: Option<zenoh::liveliness::LivelinessToken>,
}

#[derive(Default)]
pub struct Registry {
    pub by_sid: HashMap<String, Session>,
    pub by_user: HashMap<String, String>,
}

fn serde_code(e: &serde_json::Error) -> Code {
    if e.to_string().contains("missing field") { Code::MISSING_FIELD } else { Code::INVALID_PAYLOAD }
}

fn parse<T: serde::de::DeserializeOwned>(f: &Frame) -> Result<T, Frame> {
    f.parse::<T>().map_err(|e| fail_frame(f.op, f.pid, &serde_code(&e).failure_msg(e.to_string())))
}

fn fail(f: &Frame, c: Code) -> Frame {
    fail_frame(f.op, f.pid, &c.failure())
}

fn fail_msg(f: &Frame, c: Code, m: &str) -> Frame {
    fail_frame(f.op, f.pid, &c.failure_msg(m))
}

pub fn valid_room_id(r: &str) -> bool {
    common::keyspace::valid_piece(r) && r.len() <= 255
}

pub fn lookup_user(hub: &Hub, sid: &str) -> Option<String> {
    hub.sessions.lock().ok()?.by_sid.get(sid).map(|s| s.user_id.clone())
}

pub fn session_rooms(hub: &Hub, sid: &str) -> Option<Vec<String>> {
    hub.sessions.lock().ok()?.by_sid.get(sid).map(|s| s.rooms.keys().cloned().collect())
}

fn bind_reply(hub: &Hub, f: &Frame, user: &str, sid: &str, pc: PcMode) -> Frame {
    let res = BindRes {
        user_id: user.to_string(),
        server_ver: 1,
        heartbeat_interval: u64::from(hub.policy.hub.heartbeat_interval_ms),
        session_id: sid.to_string(),
        resume_window_ms: u64::from(hub.policy.hub.resume_window_ms),
        pc_mode: pc,
    };
    Frame::ok(f.op, f.pid, serde_json::to_vec(&res).unwrap_or_default())
}

pub async fn bind(hub: &std::sync::Arc<Hub>, conn: &ConnHandle, f: &Frame) -> Result<String, Frame> {
    let req: BindReq = parse(f)?;
    if req.client_ver.is_some_and(|v| v != 1) {
        hub.count_code(1004);
        return Err(fail(f, Code::VERSION_MISMATCH));
    }
    if let Some(old) = req.session_id.as_deref().filter(|s| !s.is_empty()) {
        if let Some((user, pc)) = attach_local(hub, old, conn) {
            conn.reply(bind_reply(hub, f, &user, old, pc));
            tracing::info!(kind = "ev", spec = "CP-OP-BIND", user = user.as_str(), "bind resumed");
            return Ok(old.to_string());
        }
        if let Some((user, pc)) = take_remote(hub, old, conn).await {
            conn.reply(bind_reply(hub, f, &user, old, pc));
            tracing::info!(kind = "ev", spec = "CP-OP-BIND", user = user.as_str(), "bind handed over");
            return Ok(old.to_string());
        }
    }
    let claims = match crate::auth::verify_user(&hub.system, &req.token) {
        Ok(c) => c,
        Err(c) => {
            hub.count_code(c.0);
            return Err(fail(f, c));
        }
    };
    let max = hub.policy.quota.max_sessions;
    if max > 0 && hub.sessions.lock().map(|s| s.by_sid.len() as u32).unwrap_or(0) >= max {
        hub.count_code(4003);
        return Err(fail(f, Code::QUOTA_EXCEEDED));
    }
    let pc = req.pc_mode.unwrap_or(PcMode::V2pc);
    let sid = common::ident::random_hex(16);
    evict_user(hub, &claims.sub, &sid).await;
    let session = new_session(hub, &sid, &claims, pc, conn.clone()).await;
    if let Ok(mut r) = hub.sessions.lock() {
        r.by_user.insert(claims.sub.clone(), sid.clone());
        r.by_sid.insert(sid.clone(), session);
    }
    conn.reply(bind_reply(hub, f, &claims.sub, &sid, pc));
    tracing::info!(kind = "ev", spec = "CP-OP-BIND", user = claims.sub.as_str(), "bind new session");
    Ok(sid)
}

async fn new_session(hub: &std::sync::Arc<Hub>, sid: &str, c: &Claims, pc: PcMode, conn: ConnHandle) -> Session {
    let user_sub = crate::bus::subscribe_user(hub, &c.sub).await;
    let token = crate::bus::declare_sess(hub, &c.sub, sid).await;
    Session {
        sid: sid.to_string(),
        user_id: c.sub.clone(),
        participant_type: c.participant_type,
        hidden: c.hidden,
        metadata: c.metadata.clone(),
        permission: c.permission.unwrap_or(ALL_ALLOWED),
        pc_mode: pc,
        conn: Some(conn),
        resumed: false,
        dropped_at: None,
        rooms: HashMap::new(),
        units: BTreeSet::new(),
        user_sub,
        token,
    }
}

fn attach_local(hub: &Hub, sid: &str, conn: &ConnHandle) -> Option<(String, PcMode)> {
    let mut r = hub.sessions.lock().ok()?;
    let s = r.by_sid.get_mut(sid)?;
    if let Some(old) = s.conn.take()
        && old.id != conn.id && !old.is_closed() {
            old.leave(Code::DUPLICATE_SESSION);
        }
    s.conn = Some(conn.clone());
    s.resumed = true;
    s.dropped_at = None;
    Some((s.user_id.clone(), s.pc_mode))
}

async fn take_remote(hub: &std::sync::Arc<Hub>, sid: &str, conn: &ConnHandle) -> Option<(String, PcMode)> {
    let bus = hub.bus()?;
    let keys = common::bus::live_keys(bus, &hub.keys.sess_by_sid(sid), crate::route::CALL_TIMEOUT).await;
    let node = keys.iter().find_map(|k| match common::keyspace::parse_token(hub.keys.prefix(), k) {
        Some(common::keyspace::Token::Sess { node, .. }) if node != hub.node => Some(node),
        _ => None,
    })?;
    let data = match crate::route::node_call(hub, &node, &NodeReq::TakeSession { session_id: sid.to_string() }).await {
        Ok(NodeRes::Session(Some(d))) => d,
        _ => return None,
    };
    let claims = Claims {
        sub: data.user_id.clone(),
        participant_type: data.participant_type,
        hidden: data.hidden,
        metadata: data.metadata.clone(),
        permission: Some(data.permission),
        iat: 0,
        exp: 0,
    };
    let mut s = new_session(hub, sid, &claims, data.pc_mode, conn.clone()).await;
    s.resumed = true;
    s.units = data.units.iter().cloned().collect();
    for (room, n) in &data.rooms {
        interest_inc(hub, room).await;
        s.rooms.insert(room.clone(), n.clone());
    }
    let pc = s.pc_mode;
    if let Ok(mut r) = hub.sessions.lock() {
        r.by_user.insert(data.user_id.clone(), sid.to_string());
        r.by_sid.insert(sid.to_string(), s);
    }
    Some((data.user_id, pc))
}

pub fn give_session(hub: &Hub, sid: &str) -> Option<SessionData> {
    let s = {
        let mut r = hub.sessions.lock().ok()?;
        let s = r.by_sid.remove(sid)?;
        if r.by_user.get(&s.user_id).is_some_and(|x| x == sid) {
            r.by_user.remove(&s.user_id);
        }
        s
    };
    if let Some(c) = &s.conn {
        c.leave(Code::DUPLICATE_SESSION);
    }
    if let Some(t) = &s.user_sub {
        t.abort();
    }
    let rooms: Vec<(String, String)> = s.rooms.iter().map(|(a, b)| (a.clone(), b.clone())).collect();
    for (room, _) in &rooms {
        interest_dec_sync(hub, room);
    }
    Some(SessionData {
        session_id: s.sid.clone(),
        user_id: s.user_id.clone(),
        pc_mode: s.pc_mode,
        participant_type: s.participant_type,
        hidden: s.hidden,
        metadata: s.metadata.clone(),
        permission: s.permission,
        units: s.units.iter().cloned().collect(),
        rooms,
    })
}

async fn evict_user(hub: &std::sync::Arc<Hub>, user: &str, new_sid: &str) {
    let old = hub.sessions.lock().ok().and_then(|r| r.by_user.get(user).cloned());
    if let Some(old) = old.filter(|o| o != new_sid) {
        let conn = hub.sessions.lock().ok().and_then(|r| r.by_sid.get(&old).and_then(|s| s.conn.clone()));
        if let Some(c) = conn {
            c.leave(Code::DUPLICATE_SESSION);
        }
        dispose(hub, &old, EndPath::Evict).await;
    }
    let Some(bus) = hub.bus() else { return };
    let keys = common::bus::live_keys(bus, &hub.keys.sess_by_user(user), crate::route::CALL_TIMEOUT).await;
    let nodes: BTreeSet<String> = keys
        .iter()
        .filter_map(|k| match common::keyspace::parse_token(hub.keys.prefix(), k) {
            Some(common::keyspace::Token::Sess { node, .. }) if node != hub.node => Some(node),
            _ => None,
        })
        .collect();
    for n in nodes {
        let req = NodeReq::DropUser { user_id: user.to_string(), code: 2010, except_session: Some(new_sid.to_string()) };
        let _ = crate::route::node_call(hub, &n, &req).await;
    }
}

pub async fn drop_user(hub: &std::sync::Arc<Hub>, user: &str, code: u16, except: Option<&str>) -> Vec<String> {
    let sids: Vec<String> = hub
        .sessions
        .lock()
        .map(|r| r.by_sid.values().filter(|s| s.user_id == user && Some(s.sid.as_str()) != except).map(|s| s.sid.clone()).collect())
        .unwrap_or_default();
    let path = if code == 2009 { EndPath::Cut } else { EndPath::Evict };
    for sid in &sids {
        let conn = hub.sessions.lock().ok().and_then(|r| r.by_sid.get(sid).and_then(|s| s.conn.clone()));
        if let Some(c) = conn {
            c.leave(Code(code));
        }
        dispose(hub, sid, path).await;
    }
    sids
}

pub async fn client_leave(hub: &std::sync::Arc<Hub>, sid: &str) {
    dispose(hub, sid, EndPath::Leave).await;
}

pub async fn conn_closed(hub: &std::sync::Arc<Hub>, sid: &str, conn_id: u64) {
    let at = {
        let Ok(mut r) = hub.sessions.lock() else { return };
        let Some(s) = r.by_sid.get_mut(sid) else { return };
        if s.conn.as_ref().is_some_and(|c| c.id == conn_id) {
            s.conn = None;
            let now = Instant::now();
            s.dropped_at = Some(now);
            now
        } else {
            return;
        }
    };
    let hub = hub.clone();
    let sid = sid.to_string();
    let window = std::time::Duration::from_millis(u64::from(hub.policy.hub.resume_window_ms));
    tokio::spawn(async move {
        tokio::time::sleep(window).await;
        let expired = hub
            .sessions
            .lock()
            .map(|r| r.by_sid.get(&sid).is_some_and(|s| s.conn.is_none() && s.dropped_at == Some(at)))
            .unwrap_or(false);
        if expired {
            dispose(&hub, &sid, EndPath::Window).await;
        }
    });
}

pub async fn dispose(hub: &std::sync::Arc<Hub>, sid: &str, path: EndPath) {
    let s = {
        let Ok(mut r) = hub.sessions.lock() else { return };
        let Some(s) = r.by_sid.remove(sid) else { return };
        if r.by_user.get(&s.user_id).is_some_and(|x| x == sid) {
            r.by_user.remove(&s.user_id);
        }
        s
    };
    if let Some(t) = &s.user_sub {
        t.abort();
    }
    drop(s.token);
    for room in s.rooms.keys() {
        interest_dec_sync(hub, room);
    }
    tracing::info!(kind = "ev", spec = "SV-STATE-SESSION-END", user = s.user_id.as_str(), path = format!("{path:?}"), "session end");
    let units: Vec<String> = s.units.iter().cloned().collect();
    let user = s.user_id.clone();
    let sid = s.sid.clone();
    let hub = hub.clone();
    tokio::spawn(async move {
        for n in units {
            let req = UnitReq::SessionEnd { user_id: user.clone(), session_id: sid.clone(), path };
            let _ = crate::route::unit(&hub, &n, req).await;
        }
    });
}

pub async fn interest_inc(hub: &std::sync::Arc<Hub>, room: &str) {
    let exists = hub
        .interest
        .lock()
        .map(|mut m| {
            if let Some(i) = m.get_mut(room) {
                i.count += 1;
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if exists {
        return;
    }
    let task = crate::bus::subscribe_room(hub, room).await;
    if let Ok(mut m) = hub.interest.lock() {
        match m.get_mut(room) {
            Some(i) => {
                i.count += 1;
                task.abort();
            }
            None => {
                m.insert(room.to_string(), crate::state::Interest { count: 1, task });
            }
        }
    }
}

pub fn interest_dec_sync(hub: &Hub, room: &str) {
    if let Ok(mut m) = hub.interest.lock() {
        let gone = match m.get_mut(room) {
            Some(i) => {
                i.count = i.count.saturating_sub(1);
                i.count == 0
            }
            None => false,
        };
        if gone
            && let Some(i) = m.remove(room) {
                i.task.abort();
            }
    }
}

fn caller(hub: &Hub, sid: &str) -> Option<Caller> {
    let r = hub.sessions.lock().ok()?;
    let s = r.by_sid.get(sid)?;
    Some(Caller {
        user_id: s.user_id.clone(),
        session_id: s.sid.clone(),
        pc_mode: s.pc_mode,
        participant_type: s.participant_type,
        hidden: s.hidden,
        metadata: s.metadata.clone(),
        permission: s.permission,
        home: Home { node: hub.node.clone(), inst: hub.my_inst() },
    })
}

fn reconcile(hub: &Hub, sid: &str, node: &str, aff: &Affiliation) -> Vec<String> {
    let mut dropped = Vec::new();
    if let Ok(mut r) = hub.sessions.lock()
        && let Some(s) = r.by_sid.get_mut(sid) {
            let gone: Vec<String> =
                s.rooms.iter().filter(|(room, n)| n.as_str() == node && !aff.sub_rooms.contains(room)).map(|(k, _)| k.clone()).collect();
            for g in gone {
                s.rooms.remove(&g);
                dropped.push(g);
            }
        }
    for g in &dropped {
        interest_dec_sync(hub, g);
    }
    dropped
}

async fn forward(hub: &std::sync::Arc<Hub>, sid: &str, node: &str, f: &Frame) -> Result<UnitRes, Code> {
    let c = caller(hub, sid).ok_or(Code::NOT_BOUND)?;
    let body = String::from_utf8_lossy(f.body_json()).to_string();
    let r = crate::route::unit(hub, node, UnitReq::RoomOp { caller: c, op: f.op, body }).await;
    if matches!(r, Ok(UnitRes::Frame { .. }))
        && let Ok(mut reg) = hub.sessions.lock()
            && let Some(s) = reg.by_sid.get_mut(sid) {
                s.units.insert(node.to_string());
            }
    r
}

fn room_node(hub: &Hub, sid: &str, room: &str) -> Option<String> {
    hub.sessions
        .lock()
        .ok()
        .and_then(|r| r.by_sid.get(sid).and_then(|s| s.rooms.get(room).cloned()))
        .or_else(|| hub.room_place(room).map(|p| p.node))
}

fn ok_affiliation(res: &Result<UnitRes, Code>) -> Option<Affiliation> {
    #[derive(serde::Deserialize)]
    struct WithAff {
        affiliation: Affiliation,
    }
    match res {
        Ok(UnitRes::Frame { ok: true, body }) => serde_json::from_str::<WithAff>(body).ok().map(|w| w.affiliation),
        _ => None,
    }
}

fn ok_sub_rooms(res: &Result<UnitRes, Code>) -> Option<Affiliation> {
    match res {
        Ok(UnitRes::Frame { ok: true, body }) => serde_json::from_str::<Affiliation>(body).ok(),
        _ => None,
    }
}

fn admit_join(hub: &Hub) -> Option<u64> {
    let limit = hub.policy.hub.join_admit_per_sec;
    if limit == 0 {
        return None;
    }
    let sec = common::ident::now_ms() / 1000;
    let mut g = hub.join_window.lock().ok()?;
    if g.0 != sec {
        g.0 = sec;
        g.1 = 0;
    }
    g.1 += 1;
    if g.1 <= limit {
        return None;
    }
    let base = u64::from(hub.policy.hub.join_retry_after_ms);
    let r = common::ident::random_bytes(2);
    let jitter = u64::from(u16::from_le_bytes([r[0], r[1]])) % base.max(1);
    Some(base + jitter)
}

pub async fn handle_op(hub: &std::sync::Arc<Hub>, sid: &str, f: &Frame) -> Frame {
    let Some(op) = Op::from_u16(f.op) else { return fail(f, Code::UNKNOWN_OP) };
    let r = match op {
        Op::Resume => return resume(hub, sid, f).await,
        Op::RoomJoin => return room_join(hub, sid, f).await,
        Op::RoomLeave => {
            let req: RoomLeaveReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            if !valid_room_id(&req.room_id) {
                return fail(f, Code::INVALID_PAYLOAD);
            }
            let Some(node) = room_node(hub, sid, &req.room_id) else { return fail(f, Code::NOT_IN_ROOM) };
            let res = forward(hub, sid, &node, f).await;
            if let Some(a) = ok_affiliation(&res) {
                reconcile(hub, sid, &node, &a);
            }
            res
        }
        Op::PublishTracks => {
            let req: PublishTracksReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            let room = match &req {
                PublishTracksReq::Add { room_id, .. } | PublishTracksReq::Remove { room_id, .. } => room_id.clone(),
            };
            return routed(hub, sid, f, &room).await;
        }
        Op::Ready => {
            let req: ReadyReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            return routed(hub, sid, f, &req.room_id).await;
        }
        Op::SubscribeLayer => {
            let req: SubscribeLayerReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            return routed(hub, sid, f, &req.room_id).await;
        }
        Op::TrackSet => {
            let req: TrackSetReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            return routed(hub, sid, f, &req.room_id).await;
        }
        Op::Message => {
            let req: MessageReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            return routed(hub, sid, f, &req.room_id).await;
        }
        Op::Affiliation => {
            let req: AffiliationReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            let Some(room) = req.pub_select.clone().or(req.pub_deselect.clone()) else {
                return fail_msg(f, Code::MISSING_FIELD, "pub_select or pub_deselect");
            };
            if !valid_room_id(&room) {
                return fail(f, Code::INVALID_PAYLOAD);
            }
            let Some(node) = room_node(hub, sid, &room) else { return fail(f, Code::ROOM_NOT_FOUND) };
            let res = forward(hub, sid, &node, f).await;
            if let Some(a) = ok_sub_rooms(&res) {
                reconcile(hub, sid, &node, &a);
            }
            res
        }
        Op::IceRestart => {
            let req: IceRestartReq = match parse(f) {
                Ok(r) => r,
                Err(e) => return e,
            };
            let Some(node) = hub.node_by_inst(&req.sfu_id) else {
                hub.count_code(3010);
                return fail(f, Code::PEER_NOT_FOUND);
            };
            forward(hub, sid, &node, f).await
        }
        Op::Task => return Frame::ok(f.op, f.pid, vec![]),
        _ => return fail(f, Code::UNKNOWN_OP),
    };
    count_reply(hub, &r);
    res_frame(f.op, f.pid, r)
}

fn count_reply(hub: &Hub, r: &Result<UnitRes, Code>) {
    match r {
        Ok(UnitRes::Frame { ok: false, body }) => {
            if let Ok(fl) = serde_json::from_str::<oxsig::wire::Failure>(body) {
                hub.count_code(fl.code);
            }
        }
        Err(c) => hub.count_code(c.0),
        _ => {}
    }
}

async fn routed(hub: &std::sync::Arc<Hub>, sid: &str, f: &Frame, room: &str) -> Frame {
    if !valid_room_id(room) {
        return fail(f, Code::INVALID_PAYLOAD);
    }
    let Some(node) = room_node(hub, sid, room) else {
        hub.count_code(3001);
        return fail(f, Code::ROOM_NOT_FOUND);
    };
    let r = forward(hub, sid, &node, f).await;
    count_reply(hub, &r);
    res_frame(f.op, f.pid, r)
}

async fn room_join(hub: &std::sync::Arc<Hub>, sid: &str, f: &Frame) -> Frame {
    let req: RoomJoinReq = match parse(f) {
        Ok(r) => r,
        Err(e) => return e,
    };
    if !valid_room_id(&req.room_id) {
        hub.count_code(1002);
        return fail_msg(f, Code::INVALID_PAYLOAD, "room_id");
    }
    let pc = hub.sessions.lock().ok().and_then(|r| r.by_sid.get(sid).map(|s| s.pc_mode)).unwrap_or(PcMode::V2pc);
    let has_tags = req.extmap.is_some() || req.codecs.is_some();
    match pc {
        PcMode::V1pc if req.extmap.is_none() || req.codecs.is_none() => {
            hub.count_code(1003);
            return fail_msg(f, Code::MISSING_FIELD, "1pc ROOM_JOIN needs extmap and codecs");
        }
        PcMode::V2pc if has_tags => {
            hub.count_code(1002);
            return fail_msg(f, Code::INVALID_PAYLOAD, "2pc ROOM_JOIN carries no extmap/codecs");
        }
        _ => {}
    }
    let Some(place) = hub.room_place(&req.room_id) else {
        hub.count_code(3001);
        return fail(f, Code::ROOM_NOT_FOUND);
    };
    if let Some(retry) = admit_join(hub) {
        hub.count_code(4009);
        return fail_frame(
            f.op,
            f.pid,
            &Code::OVERLOADED.failure_details("overloaded", serde_json::json!({ "retry_after_ms": retry })),
        );
    }
    let was_member = hub
        .sessions
        .lock()
        .map(|r| r.by_sid.get(sid).is_some_and(|s| s.rooms.contains_key(&req.room_id)))
        .unwrap_or(false);
    if !was_member {
        interest_inc(hub, &req.room_id).await;
        if let Ok(mut r) = hub.sessions.lock()
            && let Some(s) = r.by_sid.get_mut(sid) {
                s.rooms.insert(req.room_id.clone(), place.node.clone());
            }
    }
    let res = forward(hub, sid, &place.node, f).await;
    match ok_affiliation(&res) {
        Some(a) => {
            reconcile(hub, sid, &place.node, &a);
        }
        None => {
            if !was_member {
                if let Ok(mut r) = hub.sessions.lock()
                    && let Some(s) = r.by_sid.get_mut(sid) {
                        s.rooms.remove(&req.room_id);
                    }
                interest_dec_sync(hub, &req.room_id);
            }
        }
    }
    count_reply(hub, &res);
    res_frame(f.op, f.pid, res)
}

async fn resume(hub: &std::sync::Arc<Hub>, sid: &str, f: &Frame) -> Frame {
    let (resumed, units, user) = match hub.sessions.lock() {
        Ok(r) => match r.by_sid.get(sid) {
            Some(s) => (s.resumed, s.units.iter().cloned().collect::<Vec<_>>(), s.user_id.clone()),
            None => (false, vec![], String::new()),
        },
        Err(_) => (false, vec![], String::new()),
    };
    if !resumed {
        hub.count_code(2008);
        return fail(f, Code::SESSION_NOT_FOUND);
    }
    let mut out = ResumeRes { snapshot: Default::default(), publications: vec![], mline_tables: vec![] };
    for n in units {
        match crate::route::unit(hub, &n, UnitReq::Resume { user_id: user.clone(), session_id: sid.to_string() }).await {
            Ok(UnitRes::Resume(ResumePart { snapshot, publications, mline_tables })) => {
                out.snapshot.extend(snapshot);
                out.publications.extend(publications);
                out.mline_tables.extend(mline_tables);
            }
            other => tracing::warn!(kind = "diag", node = n.as_str(), "resume part missing: {other:?}"),
        }
    }
    Frame::ok(f.op, f.pid, serde_json::to_vec(&out).unwrap_or_default())
}

pub fn deliver_user(hub: &Hub, user: &str, ev: &UnitEvent) {
    let (conn, sid) = match hub.sessions.lock() {
        Ok(r) => match r.by_user.get(user).and_then(|sid| r.by_sid.get(sid)) {
            Some(s) => (s.conn.clone(), s.sid.clone()),
            None => return,
        },
        Err(_) => return,
    };
    if ev.op == Op::RoomEvent.as_u16()
        && let Ok(RoomEventBody::Affiliation { room_id, affiliation, .. }) = serde_json::from_str::<RoomEventBody>(&ev.body) {
            let node = hub
                .sessions
                .lock()
                .ok()
                .and_then(|r| r.by_sid.get(&sid).and_then(|s| s.rooms.get(&room_id).cloned()))
                .or_else(|| hub.room_place(&room_id).map(|p| p.node));
            if let Some(n) = node {
                reconcile(hub, &sid, &n, &affiliation);
            }
        }
    if let (Some(c), Some(op)) = (conn, Op::from_u16(ev.op)) {
        c.notify(op, ev.body.as_bytes().to_vec());
    }
}

pub fn deliver_room(hub: &Hub, room: &str, ev: &UnitEvent) {
    let Some(op) = Op::from_u16(ev.op) else { return };
    let targets: Vec<ConnHandle> = match hub.sessions.lock() {
        Ok(r) => r
            .by_sid
            .values()
            .filter(|s| s.rooms.contains_key(room) && ev.exclude.as_deref() != Some(s.user_id.as_str()))
            .filter_map(|s| s.conn.clone())
            .collect(),
        Err(_) => return,
    };
    for c in targets {
        c.notify(op, ev.body.as_bytes().to_vec());
    }
}

pub fn forget_room(hub: &Hub, room: &str) {
    let mut hit = 0usize;
    if let Ok(mut r) = hub.sessions.lock() {
        for s in r.by_sid.values_mut() {
            if s.rooms.remove(room).is_some() {
                hit += 1;
            }
        }
    }
    if hit > 0
        && let Ok(mut m) = hub.interest.lock()
            && let Some(i) = m.remove(room) {
                i.task.abort();
            }
}

pub async fn shutdown_all(hub: &std::sync::Arc<Hub>) {
    let sids: Vec<(String, Option<ConnHandle>)> =
        hub.sessions.lock().map(|r| r.by_sid.values().map(|s| (s.sid.clone(), s.conn.clone())).collect()).unwrap_or_default();
    for (sid, c) in sids {
        if let Some(c) = c {
            c.leave(Code::SERVER_SHUTDOWN);
        }
        dispose(hub, &sid, EndPath::Shutdown).await;
    }
}
