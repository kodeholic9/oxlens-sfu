// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-SES-RESUME · 정:IMPL-SES-ADMISSION · 정:IMPL-ROOM-JOIN · 정:IMPL-LINK-ICE-RESTART · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:IMPL-ERR-ISSUERS · 정:IMPL-AUX-MESSAGE · 연:RULE-SES-AUTH-TWO-WAYS · 연:RULE-SES-RESUME-BY-SESSION-ID · 연:RULE-SES-PC-MODE-SESSION · 연:RULE-SES-RESUME-NO-SESSION-ID · 연:RULE-SES-RESUME-RES-TWO · 연:RULE-ROOM-JOIN-NUMBER-TAGS · 연:RULE-AFF-RES-FORM · 연:RULE-AFF-PROC · 정:CAND-SES-PREBIND-2001 · 정:CAND-AFF-SPLIT-ROUTE · model: claude-opus-5-5
//! op 처리 — BIND 두 갈래(`session_id` 가 이긴다 · 다른 node 면 넘겨받기) · 축출 · RESUME 모음 · 방 귀속 op 의 node 찾기와 통과 · 입장 앞 관심 선언.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use common::bus::envelope::{Call, GoneCause, NodeQuery, Reply, ResumePart};
use common::token::{verify_user, TokenError};
use oxsig::parse::{self, BodyError};
use oxsig::{BindReq, BindRes, Code, Failure, Frame, IceRestartReq, MessageReq, Op, PcMode, ResumeRes, RoomJoinReq};
use tokio::sync::mpsc;

use super::conn::{fail_bytes, ok_bytes, response, Conn};
use super::{Ctx, Identity, Out, Session, SessionWire, Sock};

pub const SERVER_VER: u32 = 1;

fn fail(ctx: &Ctx, f: &Frame, failure: Failure) -> Vec<u8> {
    if let Some(c) = Code::from_number(failure.code) {
        super::count(ctx, c);
    }
    fail_bytes(f, &failure)
}

fn fail_code(ctx: &Ctx, f: &Frame, c: Code) -> Vec<u8> {
    fail(ctx, f, c.failure())
}

fn body_fail(ctx: &Ctx, f: &Frame, e: BodyError) -> Vec<u8> {
    fail(ctx, f, e.code().failure_with(e.message(), None))
}

fn reply_bytes(ctx: &Ctx, f: &Frame, r: Reply) -> Vec<u8> {
    match r {
        Reply::Ok(v) => ok_bytes(f, if v.is_null() { Vec::new() } else { serde_json::to_vec(&v).unwrap_or_default() }),
        Reply::Fail(failure) => fail(ctx, f, failure),
    }
}

fn call_of(ctx: &Ctx, s: &Session, op: Op, body: serde_json::Value) -> Call {
    let _ = ctx;
    Call::Op {
        op: op.as_u16(),
        body,
        user_id: s.id.user.clone(),
        session: s.fp.clone(),
        pc_mode: s.pc_mode,
        home: s.home.clone(),
        participant_type: s.id.participant_type,
        hidden: s.id.hidden,
        metadata: s.id.metadata.clone(),
        permission: s.id.permission,
    }
}

pub async fn handle(ctx: &Arc<Ctx>, conn: &Arc<Conn>, f: &Frame, out: &mpsc::UnboundedSender<Out>) -> Option<Vec<u8>> {
    let op = Op::from_u16(f.op);
    let Some(s) = conn.session() else {
        if op == Some(Op::Bind) {
            return Some(bind(ctx, conn, f, out).await);
        }
        return Some(fail_code(ctx, f, Code::NotBound));
    };
    tracing::debug!(kind = "ev", spec = op.map(|o| o.catalog()).unwrap_or_default(), session = %s.fp, user = %s.id.user, "op in");
    let bytes = match op {
        None => fail_code(ctx, f, Code::UnknownOp),
        Some(Op::Bind) => fail(ctx, f, Code::InvalidPayload.failure_with("already bound", None)),
        Some(Op::Heartbeat) => ok_bytes(f, Vec::new()),
        Some(Op::Resume) => resume(ctx, &s, f).await,
        Some(Op::Task) => crate::aux::task(f, &s.fp),
        Some(Op::RoomJoin) => return join(ctx, &s, f, out).await,
        Some(Op::RoomLeave) => scoped(ctx, &s, f, Op::RoomLeave, true).await,
        Some(Op::Affiliation) => affiliation(ctx, &s, f).await,
        Some(Op::IceRestart) => ice_restart(ctx, &s, f).await,
        Some(Op::Message) => match parse::body::<MessageReq>(&f.body) {
            Err(e) => body_fail(ctx, f, e),
            Ok(_) => scoped(ctx, &s, f, Op::Message, false).await,
        },
        Some(o @ (Op::PublishTracks | Op::Ready | Op::SubscribeLayer | Op::TrackSet)) => scoped(ctx, &s, f, o, false).await,
        Some(_) => fail_code(ctx, f, Code::UnknownOp),
    };
    Some(bytes)
}

fn attach(conn: &Arc<Conn>, s: &Arc<Session>, out: &mpsc::UnboundedSender<Out>, resumed: bool) {
    {
        let mut st = s.lock();
        let old = st.socket.replace(Sock { tx: out.clone(), generation: conn.generation });
        if let Some(o) = old {
            let _ = o.tx.send(Out::Leave(Code::DuplicateSession));
        }
        st.dropped_at = None;
        st.resumed = resumed;
    }
    if let Ok(mut g) = conn.session.lock() {
        let _ = g.insert(s.clone());
    }
}

fn bind_res(ctx: &Ctx, s: &Session) -> Vec<u8> {
    parse::encode(&BindRes {
        user_id: s.id.user.clone(),
        server_ver: SERVER_VER,
        heartbeat_interval: u64::from(ctx.policy.hub.heartbeat_interval_ms),
        session_id: s.sid.clone(),
        resume_window_ms: u64::from(ctx.policy.hub.resume_window_ms),
        pc_mode: s.pc_mode,
    })
}

async fn bind(ctx: &Arc<Ctx>, conn: &Arc<Conn>, f: &Frame, out: &mpsc::UnboundedSender<Out>) -> Vec<u8> {
    let req: BindReq = match parse::body(&f.body) {
        Ok(r) => r,
        Err(e) => return body_fail(ctx, f, e),
    };
    if req.client_ver.unwrap_or(SERVER_VER) != SERVER_VER {
        return fail_code(ctx, f, Code::VersionMismatch);
    }
    if super::shutting(ctx) {
        return fail_code(ctx, f, Code::SfuUnavailable);
    }
    if let Some(sid) = req.session_id.as_deref().filter(|x| !x.is_empty()) {
        if let Some(old) = ctx.sessions.get(sid)
            && !old.lock().gone
        {
            attach(conn, &old, out, true);
            tracing::info!(kind = "ev", spec = "SES.S.bind_resume", session = %old.fp, user = %old.id.user, "BIND by session_id");
            return ok_bytes(f, bind_res(ctx, &old));
        }
        if let Some((_, node)) = ctx.bus.sess_by_sid(sid).await
            && node != ctx.node
            && let Some(raw) = ctx.bus.query(&node, "sess", &NodeQuery::Session { sid: sid.to_string() }, crate::bus::NODE_CALL_TIMEOUT).await
            && let Ok(w) = serde_json::from_slice::<SessionWire>(&raw)
        {
            let s = super::create(ctx, w.sid, w.id, w.pc_mode, w.servers, true).await;
            attach(conn, &s, out, true);
            tracing::info!(kind = "ev", spec = "IMPL-SES-HANDOVER", session = %s.fp, user = %s.id.user, from = %node, "session handed over");
            return ok_bytes(f, bind_res(ctx, &s));
        }
    }
    let claims = match verify_user(&req.token, &ctx.secrets.iter().map(String::as_str).collect::<Vec<_>>(), common::token::now_s()) {
        Ok(c) => c,
        Err(TokenError::Expired) => return fail_code(ctx, f, Code::TokenExpired),
        Err(_) => return fail_code(ctx, f, Code::TokenInvalid),
    };
    let Some(id) = Identity::from_claims(&claims) else { return fail_code(ctx, f, Code::TokenInvalid) };
    let max = ctx.policy.quota.max_sessions;
    if max > 0 && ctx.bus.sess_count().await >= max as usize {
        return fail_code(ctx, f, Code::QuotaExceeded);
    }
    let sid = super::new_sid();
    for old in ctx.sessions.of_user(&id.user) {
        old.send(Out::Leave(Code::DuplicateSession));
        super::count(ctx, Code::DuplicateSession);
        super::destroy(ctx, &old, GoneCause::Evict).await;
    }
    for (node, _) in ctx.bus.sess_of_user(&id.user).await {
        if node != ctx.node {
            let _ = ctx.bus.query(&node, "evict", &NodeQuery::Evict { user_id: id.user.clone(), keep_sid: sid.clone() }, crate::bus::NODE_CALL_TIMEOUT).await;
        }
    }
    let pc = req.pc_mode.unwrap_or(PcMode::V2pc);
    let s = super::create(ctx, sid, id, pc, BTreeMap::new(), false).await;
    attach(conn, &s, out, false);
    ok_bytes(f, bind_res(ctx, &s))
}

async fn resume(ctx: &Arc<Ctx>, s: &Arc<Session>, f: &Frame) -> Vec<u8> {
    if !s.lock().resumed {
        return fail_code(ctx, f, Code::SessionNotFound);
    }
    let servers: Vec<String> = s.lock().servers.keys().cloned().collect();
    let mut res = ResumeRes { snapshot: BTreeMap::new(), publications: Vec::new(), mline_tables: Vec::new() };
    let call = Call::Resume { user_id: s.id.user.clone(), session: s.fp.clone(), home: ctx.home() };
    for node in servers {
        match ctx.bus.call(&node, &call).await {
            Reply::Ok(v) => {
                let Ok(part) = serde_json::from_value::<ResumePart>(v) else { continue };
                let rooms: Vec<String> = part.rooms.keys().cloned().collect();
                super::reconcile(ctx, s, &node, &rooms).await;
                res.snapshot.extend(part.rooms);
                res.publications.extend(part.publications);
                if let Some(t) = part.mline_table {
                    res.mline_tables.push(t);
                }
            }
            Reply::Fail(fl) => {
                tracing::warn!(kind = "diag", session = %s.fp, node = %node, code = fl.code, "resume part failed — that server's rooms are left out");
                super::reconcile(ctx, s, &node, &[]).await;
            }
        }
    }
    tracing::info!(kind = "ev", spec = "SES.S.resume_ok", session = %s.fp, user = %s.id.user, rooms = res.snapshot.len(), "RESUME snapshot");
    ok_bytes(f, parse::encode(&res))
}

fn node_for(ctx: &Ctx, s: &Session, room: &str) -> Option<String> {
    let mine = s.lock().servers.iter().find(|(_, rs)| rs.contains(room)).map(|(n, _)| n.clone());
    mine.or_else(|| ctx.bus.room_node(room))
}

fn room_id_of(v: &serde_json::Value) -> Result<String, Code> {
    match v.get("room_id") {
        None | Some(serde_json::Value::Null) => Err(Code::MissingField),
        Some(serde_json::Value::String(r)) if oxsig::room_id_ok(r) => Ok(r.clone()),
        Some(_) => Err(Code::InvalidPayload),
    }
}

async fn scoped(ctx: &Arc<Ctx>, s: &Arc<Session>, f: &Frame, op: Op, reconcile: bool) -> Vec<u8> {
    let v = match parse::json_value(&f.body) {
        Ok(v) => v,
        Err(e) => return body_fail(ctx, f, e),
    };
    let room = match room_id_of(&v) {
        Ok(r) => r,
        Err(c) => return fail(ctx, f, c.failure_with("room_id", None)),
    };
    let Some(node) = node_for(ctx, s, &room) else {
        if op == Op::RoomLeave {
            super::reconcile_drop_room(ctx, s, &room).await;
        }
        return fail_code(ctx, f, Code::RoomNotFound);
    };
    let r = ctx.bus.call(&node, &call_of(ctx, s, op, v)).await;
    if reconcile
        && let Reply::Ok(rv) = &r
        && let Some(rooms) = super::affiliation_rooms(rv)
    {
        super::reconcile(ctx, s, &node, &rooms).await;
    }
    if op == Op::RoomLeave
        && let Reply::Fail(fl) = &r
        && (fl.code == Code::RoomNotFound.number() || fl.code == Code::NotInRoom.number())
    {
        super::reconcile_drop_room(ctx, s, &room).await;
    }
    reply_bytes(ctx, f, r)
}

async fn affiliation(ctx: &Arc<Ctx>, s: &Arc<Session>, f: &Frame) -> Vec<u8> {
    let v = match parse::json_value(&f.body) {
        Ok(v) => v,
        Err(e) => return body_fail(ctx, f, e),
    };
    let req: oxsig::AffiliationReq = match parse::typed(v.clone()) {
        Ok(r) => r,
        Err(e) => return body_fail(ctx, f, e),
    };
    let Some(room) = req.pub_select.clone().or(req.pub_deselect.clone()) else {
        return fail(ctx, f, Code::MissingField.failure_with("pub_select · pub_deselect 가 둘 다 없다", None));
    };
    if !oxsig::room_id_ok(&room) {
        return fail_code(ctx, f, Code::InvalidPayload);
    }
    let Some(node) = node_for(ctx, s, &room) else { return fail_code(ctx, f, Code::RoomNotFound) };
    let r = ctx.bus.call(&node, &call_of(ctx, s, Op::Affiliation, v)).await;
    if let Reply::Ok(rv) = &r
        && let Some(rooms) = super::affiliation_rooms(rv)
    {
        super::reconcile(ctx, s, &node, &rooms).await;
    }
    reply_bytes(ctx, f, r)
}

async fn ice_restart(ctx: &Arc<Ctx>, s: &Arc<Session>, f: &Frame) -> Vec<u8> {
    let v = match parse::json_value(&f.body) {
        Ok(v) => v,
        Err(e) => return body_fail(ctx, f, e),
    };
    let req: IceRestartReq = match parse::typed(v.clone()) {
        Ok(r) => r,
        Err(e) => return body_fail(ctx, f, e),
    };
    let node = if ctx.bus.my_inst().as_deref() == Some(req.sfu_id.as_str()) { Some(ctx.node.clone()) } else { ctx.bus.node_of_sfu(&req.sfu_id) };
    let Some(node) = node else {
        tracing::info!(kind = "ev", spec = "LINK.S.restart_3010", session = %s.fp, sfu_id = %req.sfu_id, "ICE_RESTART for a start identity that is gone");
        return fail_code(ctx, f, Code::PeerNotFound);
    };
    let r = ctx.bus.call(&node, &call_of(ctx, s, Op::IceRestart, v)).await;
    reply_bytes(ctx, f, r)
}

async fn join(ctx: &Arc<Ctx>, s: &Arc<Session>, f: &Frame, out: &mpsc::UnboundedSender<Out>) -> Option<Vec<u8>> {
    let v = match parse::json_value(&f.body) {
        Ok(v) => v,
        Err(e) => return Some(body_fail(ctx, f, e)),
    };
    let req: RoomJoinReq = match parse::typed(v.clone()) {
        Ok(r) => r,
        Err(e) => return Some(body_fail(ctx, f, e)),
    };
    if !oxsig::room_id_ok(&req.room_id) {
        return Some(fail(ctx, f, Code::InvalidPayload.failure_with("room_id", None)));
    }
    match s.pc_mode {
        PcMode::V1pc if req.extmap.is_none() || req.codecs.is_none() => {
            return Some(fail(ctx, f, Code::MissingField.failure_with("1pc 번호표(extmap · codecs)가 없다", None)));
        }
        PcMode::V2pc if req.extmap.is_some() || req.codecs.is_some() => {
            return Some(fail(ctx, f, Code::InvalidPayload.failure_with("2pc 에 번호표", None)));
        }
        _ => {}
    }
    if let Err(retry) = ctx.admission.admit(common::time::wall_ms()) {
        return Some(fail(ctx, f, Code::Overloaded.failure_with("join admission", Some(serde_json::json!({"retry_after_ms": retry})))));
    }
    let Some(node) = ctx.bus.room_node(&req.room_id) else { return Some(fail_code(ctx, f, Code::RoomNotFound)) };
    ctx.bus.interest_room(&req.room_id).await;
    s.lock().pending.insert(req.room_id.clone(), Vec::new());
    let r = ctx.bus.call(&node, &call_of(ctx, s, Op::RoomJoin, v)).await;
    match r {
        Reply::Ok(rv) => {
            if let Some(rooms) = super::affiliation_rooms(&rv) {
                super::reconcile(ctx, s, &node, &rooms).await;
            }
            ctx.bus.release_room(&req.room_id).await;
            let bytes = ok_bytes(f, serde_json::to_vec(&rv).unwrap_or_default());
            let buffered = s.lock().pending.remove(&req.room_id).unwrap_or_default();
            let _ = out.send(response(f.op, bytes));
            for item in buffered {
                let _ = out.send(Out::Item(item));
            }
            tracing::info!(kind = "ev", spec = "CP-OP-ROOM-JOIN", session = %s.fp, user = %s.id.user, room = %req.room_id, node = %node, "joined");
            None
        }
        Reply::Fail(fl) => {
            s.lock().pending.remove(&req.room_id);
            ctx.bus.release_room(&req.room_id).await;
            Some(fail(ctx, f, fl))
        }
    }
}

pub async fn evict_local(ctx: &Arc<Ctx>, user: &str, keep_sid: &str) -> usize {
    let mut n = 0;
    for old in ctx.sessions.of_user(user) {
        if old.sid == keep_sid {
            continue;
        }
        old.send(Out::Leave(Code::DuplicateSession));
        super::destroy(ctx, &old, GoneCause::Evict).await;
        n += 1;
    }
    n
}

pub async fn hand_over(ctx: &Arc<Ctx>, sid: &str) -> Option<SessionWire> {
    let s = ctx.sessions.get(sid)?;
    let servers: BTreeMap<String, BTreeSet<String>> = s.lock().servers.clone();
    s.send(Out::Leave(Code::DuplicateSession));
    super::detach(ctx, &s).await;
    tracing::info!(kind = "ev", spec = "IMPL-SES-HANDOVER", session = %s.fp, user = %s.id.user, "session handed to another node");
    Some(SessionWire { sid: s.sid.clone(), id: s.id.clone(), pc_mode: s.pc_mode, servers })
}

pub async fn cut_local(ctx: &Arc<Ctx>, user: &str) -> Vec<String> {
    let mut cut = Vec::new();
    for s in ctx.sessions.of_user(user) {
        s.send(Out::Leave(Code::SessionRevoked));
        super::count(ctx, Code::SessionRevoked);
        super::destroy(ctx, &s, GoneCause::Cut).await;
        cut.push(s.fp.clone());
    }
    cut
}

pub async fn on_room_event(ctx: &Arc<Ctx>, user: &str, n: &common::bus::envelope::Notice) {
    if n.op != Op::RoomEvent.as_u16() || n.body.get("type").and_then(|t| t.as_str()) != Some("affiliation") {
        return;
    }
    let Some(rooms) = super::affiliation_rooms(&n.body) else { return };
    let room = n.body.get("room_id").and_then(|r| r.as_str()).unwrap_or_default().to_string();
    let node = n.sfu_id.as_deref().and_then(|sid| if ctx.bus.my_inst().as_deref() == Some(sid) { Some(ctx.node.clone()) } else { ctx.bus.node_of_sfu(sid) });
    for s in ctx.sessions.of_user(user) {
        let node = node.clone().or_else(|| s.lock().servers.iter().find(|(_, rs)| rs.contains(&room)).map(|(k, _)| k.clone()));
        if let Some(node) = node {
            super::reconcile(ctx, &s, &node, &rooms).await;
        }
    }
}
