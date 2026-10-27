// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-SES-READ-AND-PROCESS · 정:CAND-AFF-SPLIT-ROUTE · 정:IMPL-SES-BIND · 정:IMPL-SES-HANDOVER · 정:IMPL-SES-RESUME · 정:IMPL-SES-SOCKET-LIFE · 정:IMPL-SES-ADMISSION · 정:IMPL-ROOM-JOIN · 정:RULE-BUS-REQUEST · 정:IMPL-AUX-MESSAGE · 정:CAND-SES-PREBIND-2001 · 연:RULE-TRN-CLOSE · 연:RULE-TRN-ONE-REPLY · 연:RULE-SES-BIND-PROC · 연:RULE-SES-LEAVE-PROC · 연:RULE-TRN-NOTIFY-ACK · model: claude-opus-5-5
//! WS 한 연결 — 읽기와 처리를 가른다(`HEARTBEAT` 는 읽는 즉시 · 방 요청은 연결 단위 차례로 미디어 서버에) · 송신은 응답 즉시 · 통지는 창으로.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use common::bus::env::{Call, Reply};
use futures_util::{SinkExt, StreamExt};
use oxsig::code::{Code, Failure};
use oxsig::frame::{Frame, FrameError, FrameKind};
use oxsig::msg::{
    AffiliationReq, BindReq, BindRes, IceRestartReq, LeaveBody, MessageReq, PublishTracksReq, ReadyReq, RoomJoinReq, RoomLeaveReq, SubscribeLayerReq,
    TaskReq, TrackSetReq,
};
use oxsig::op::Op;
use oxsig::types::PcMode;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use super::route::{self, Target};
use super::{HubRef, Out, SockTx};
use crate::ses::core::{ACK_TIMEOUT_MS, BIND_TIMEOUT_MS};
use crate::ses::{Branch, OutQueue, SessionData, bind_branch, new_session_id, token};

const UNBOUND: u8 = 0;
const BINDING: u8 = 1;
const BOUND: u8 = 2;
const CLOSED: u8 = 3;
const ROOM_ID_MAX: usize = 255;

struct Conn {
    id: u64,
    state: AtomicU8,
    resumed: AtomicU8,
    sid: Mutex<Option<String>>,
    tx: SockTx,
}

impl Conn {
    fn sid(&self) -> Option<String> {
        self.sid.lock().ok().and_then(|s| s.clone())
    }

    fn reply(&self, f: Frame) {
        let _ = self.tx.out.send(Out::Frame(f.encode()));
    }

    fn leave(&self, c: Code) {
        self.state.store(CLOSED, Ordering::Relaxed);
        let _ = self.tx.out.send(Out::Leave(c));
    }
}

pub fn room_id_ok(r: &str) -> bool {
    !r.is_empty() && r.len() <= ROOM_ID_MAX && common::bus::keys::fragment_ok(r)
}

pub fn body_code(e: &serde_json::Error) -> Code {
    if e.to_string().starts_with("missing field") { Code::MISSING_FIELD } else { Code::INVALID_PAYLOAD }
}

fn parse<T: DeserializeOwned>(f: &Frame) -> Result<T, Code> {
    f.parse::<T>().map_err(|e| body_code(&e))
}

pub async fn serve(hub: HubRef, socket: WebSocket) {
    let id = hub.next_sock.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Out>();
    let q = Arc::new(Mutex::new(OutQueue::new(hub.cfg.policy.hub.ws_flow_window)));
    let wake = Arc::new(Notify::new());
    let tx = SockTx { out: out_tx, q: q.clone(), wake: wake.clone() };
    if let Ok(mut s) = hub.socks.lock() {
        s.insert(id, tx.clone());
    }
    let conn = Arc::new(Conn { id, state: AtomicU8::new(UNBOUND), resumed: AtomicU8::new(0), sid: Mutex::new(None), tx });
    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            let mut leave: Option<Code> = None;
            tokio::select! {
                o = out_rx.recv() => match o {
                    Some(Out::Frame(b)) => { if sink.send(Message::Binary(b)).await.is_err() { return; } }
                    Some(Out::Leave(c)) => leave = Some(c),
                    None => return,
                },
                _ = wake.notified() => {}
                _ = tick.tick() => {
                    if q.lock().map(|x| x.overdue(super::now_ms(), ACK_TIMEOUT_MS)).unwrap_or(false) { leave = Some(Code::FLOW_TIMEOUT); }
                }
            }
            if let Some(c) = leave {
                let f = Frame::request(Op::Leave.code(), 0, &LeaveBody::of(c));
                let _ = sink.send(Message::Binary(f.encode())).await;
                let _ = sink.close().await;
                return;
            }
            loop {
                let next = q.lock().ok().and_then(|mut x| x.pop_ready(super::now_ms()));
                let Some((pid, op, body)) = next else { break };
                let f = Frame::new(FrameKind::Request, op, pid, body);
                if sink.send(Message::Binary(f.encode())).await.is_err() {
                    return;
                }
            }
        }
    });
    let (proc_tx, mut proc_rx) = mpsc::unbounded_channel::<Frame>();
    let ph = hub.clone();
    let pc = conn.clone();
    let processor = tokio::spawn(async move {
        while let Some(f) = proc_rx.recv().await {
            if pc.state.load(Ordering::Relaxed) == CLOSED {
                break;
            }
            handle(&ph, &pc, f).await;
        }
    });
    let opened = super::now_ms();
    loop {
        let st = conn.state.load(Ordering::Relaxed);
        if st == CLOSED {
            break;
        }
        let wait = if st == BOUND {
            u64::from(hub.cfg.policy.hub.heartbeat_timeout_ms)
        } else {
            (opened + BIND_TIMEOUT_MS).saturating_sub(super::now_ms()).max(1)
        };
        let msg = match tokio::time::timeout(Duration::from_millis(wait), stream.next()).await {
            Err(_) => {
                conn.leave(Code::HEARTBEAT_TIMEOUT);
                break;
            }
            Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(m))) => m,
        };
        let bytes = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Text(_) => {
                conn.leave(Code::PROTOCOL_ERROR);
                break;
            }
        };
        let f = match Frame::decode(&bytes) {
            Ok(f) => f,
            Err(FrameError::TooLarge | FrameError::Short | FrameError::BadVersion | FrameError::ReservedKind) => {
                conn.leave(Code::PROTOCOL_ERROR);
                break;
            }
        };
        if f.kind() != FrameKind::Request {
            if let Ok(mut x) = conn.tx.q.lock() {
                x.ack(f.pid);
            }
            conn.tx.wake.notify_one();
            continue;
        }
        if !f.body.is_empty() && f.json().is_err() {
            conn.reply(f.fail(&Failure::of(Code::PROTOCOL_ERROR)));
            conn.leave(Code::PROTOCOL_ERROR);
            break;
        }
        let op = Op::from_u16(f.op);
        let st = conn.state.load(Ordering::Relaxed);
        if st != BOUND {
            if st == UNBOUND && op == Some(Op::Bind) {
                conn.state.store(BINDING, Ordering::Relaxed);
                let _ = proc_tx.send(f);
                continue;
            }
            conn.leave(Code::NOT_BOUND);
            break;
        }
        match op {
            Some(Op::Heartbeat) => conn.reply(f.ok(&json!({}))),
            Some(Op::Bind) => {
                conn.reply(f.fail(&Failure::of(Code::PROTOCOL_ERROR)));
                conn.leave(Code::PROTOCOL_ERROR);
                break;
            }
            Some(Op::Leave) => {
                conn.state.store(CLOSED, Ordering::Relaxed);
                let code = f.parse::<LeaveBody>().map(|b| Code(b.code)).unwrap_or(Code(0));
                if let Some(sid) = conn.sid() {
                    hub.discard_session(&sid, code).await;
                }
                break;
            }
            _ => {
                let _ = proc_tx.send(f);
            }
        }
    }
    drop(proc_tx);
    conn.state.store(CLOSED, Ordering::Relaxed);
    processor.abort();
    if let Ok(mut s) = hub.socks.lock() {
        s.remove(&id);
    }
    if let Ok(mut t) = hub.sessions.lock() {
        t.detach(id, super::now_ms());
    }
    let _ = writer.await;
}

async fn handle(hub: &HubRef, c: &Arc<Conn>, f: Frame) {
    let Some(op) = Op::from_u16(f.op) else {
        hub.drops.code(Code::UNKNOWN_OP.0);
        c.reply(f.fail(&Failure::of(Code::UNKNOWN_OP)));
        return;
    };
    if op == Op::Bind {
        match bind(hub, c, &f).await {
            Ok(res) => {
                c.state.store(BOUND, Ordering::Relaxed);
                c.reply(f.ok(&res));
            }
            Err(code) => {
                hub.drops.code(code.0);
                if c.state.load(Ordering::Relaxed) != CLOSED {
                    c.state.store(UNBOUND, Ordering::Relaxed);
                }
                c.reply(f.fail(&Failure::of(code)));
            }
        }
        return;
    }
    let Some(sid) = c.sid() else {
        c.reply(f.fail(&Failure::of(Code::NOT_BOUND)));
        return;
    };
    let r = match op {
        Op::Resume => resume(hub, c, &sid).await,
        Op::Task => task(hub, &sid, &f),
        Op::RoomJoin => join(hub, &sid, &f).await,
        Op::RoomLeave => room_op::<RoomLeaveReq>(hub, &sid, &f, |r| r.room_id.clone()).await,
        Op::PublishTracks => room_op::<PublishTracksReq>(hub, &sid, &f, |r| r.room_id().to_string()).await,
        Op::Ready => room_op::<ReadyReq>(hub, &sid, &f, |r| r.room_id.clone()).await,
        Op::SubscribeLayer => room_op::<SubscribeLayerReq>(hub, &sid, &f, |r| r.room_id.clone()).await,
        Op::TrackSet => room_op::<TrackSetReq>(hub, &sid, &f, |r| r.room_id.clone()).await,
        Op::Message => room_op::<MessageReq>(hub, &sid, &f, |r| r.room_id.clone()).await,
        Op::Affiliation => affiliation(hub, &sid, &f).await,
        Op::IceRestart => ice_restart(hub, &sid, &f).await,
        _ => Err(Failure::of(Code::UNKNOWN_OP)),
    };
    match r {
        Ok(v) => c.reply(f.ok(&v)),
        Err(fl) => {
            hub.drops.code(fl.code);
            c.reply(f.fail(&fl));
        }
    }
}

fn session_ctx(hub: &HubRef, sid: &str) -> Option<(SessionData, common::bus::env::SessionCtx)> {
    hub.sessions.lock().ok().and_then(|t| t.get(sid).map(|s| (s.data.clone(), s.data.ctx())))
}

async fn forward(hub: &HubRef, sid: &str, t: &Target, f: &Frame) -> Result<Value, Failure> {
    let (_, ctx) = session_ctx(hub, sid).ok_or_else(|| Failure::of(Code::SESSION_NOT_FOUND))?;
    let body = f.json().unwrap_or(json!({}));
    let call = Call::Op { ctx, op: f.op, body };
    match route::call(hub, t, &call).await {
        Reply::Ok { body } => Ok(body),
        Reply::Fail { failure } => Err(failure),
        Reply::Http { body, .. } => Ok(body),
    }
}

fn sub_rooms(v: &Value) -> BTreeSet<String> {
    v.pointer("/affiliation/sub_rooms").or_else(|| v.get("sub_rooms")).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

async fn room_op<T: DeserializeOwned>(hub: &HubRef, sid: &str, f: &Frame, room_of: impl Fn(&T) -> String) -> Result<Value, Failure> {
    let req: T = parse(f).map_err(Failure::of)?;
    let room = room_of(&req);
    if !room_id_ok(&room) {
        return Err(Failure::of(Code::INVALID_PAYLOAD));
    }
    let t = route::room_target(hub, &room).ok_or_else(|| Failure::of(Code::ROOM_NOT_FOUND))?;
    let v = forward(hub, sid, &t, f).await?;
    if f.op == Op::RoomLeave.code()
        && let Some(inst) = &t.inst
    {
        hub.set_server_rooms(sid, inst, &t.node, sub_rooms(&v));
    }
    Ok(v)
}

async fn join(hub: &HubRef, sid: &str, f: &Frame) -> Result<Value, Failure> {
    let req: RoomJoinReq = parse(f).map_err(Failure::of)?;
    if !room_id_ok(&req.room_id) {
        return Err(Failure::of(Code::INVALID_PAYLOAD));
    }
    let pc = session_ctx(hub, sid).map(|x| x.0.pc_mode).unwrap_or(PcMode::V2pc);
    let has_tags = req.extmap.is_some() || req.codecs.is_some();
    match pc {
        PcMode::V1pc if req.extmap.is_none() || req.codecs.is_none() => return Err(Failure::of(Code::MISSING_FIELD)),
        PcMode::V2pc if has_tags => return Err(Failure::of(Code::INVALID_PAYLOAD)),
        _ => {}
    }
    if let Err(retry) = hub.admit_join(super::now_ms()) {
        common::ev!(spec = "CP-CODE-OVERLOADED", room = %req.room_id, "입장 수용 한도");
        return Err(Failure::of(Code::OVERLOADED).details(json!({"retry_after_ms": retry})));
    }
    let t = route::room_target(hub, &req.room_id).ok_or_else(|| Failure::of(Code::ROOM_NOT_FOUND))?;
    if let Ok(mut j) = hub.joining.lock() {
        j.entry(sid.to_string()).or_default().insert(req.room_id.clone());
    }
    hub.interest_inc(&req.room_id).await;
    let r = forward(hub, sid, &t, f).await;
    if let Ok(v) = &r {
        let sfu = v.pointer("/server_config/sfu_id").and_then(|x| x.as_str()).map(str::to_string).or(t.inst.clone()).unwrap_or_default();
        hub.set_server_rooms(sid, &sfu, &t.node, sub_rooms(v));
    }
    if let Ok(mut j) = hub.joining.lock()
        && let Some(s) = j.get_mut(sid)
    {
        s.remove(&req.room_id);
        if s.is_empty() {
            j.remove(sid);
        }
    }
    hub.interest_dec(&req.room_id).await;
    r
}

async fn affiliation(hub: &HubRef, sid: &str, f: &Frame) -> Result<Value, Failure> {
    let req: AffiliationReq = parse(f).map_err(Failure::of)?;
    let room = req.pub_select.clone().or(req.pub_deselect.clone()).ok_or_else(|| Failure::of(Code::MISSING_FIELD))?;
    if !room_id_ok(&room) {
        return Err(Failure::of(Code::INVALID_PAYLOAD));
    }
    let t = route::room_target(hub, &room).ok_or_else(|| Failure::of(Code::ROOM_NOT_FOUND))?;
    let v = forward(hub, sid, &t, f).await?;
    if let Some(inst) = &t.inst {
        hub.set_server_rooms(sid, inst, &t.node, sub_rooms(&v));
    }
    Ok(v)
}

async fn ice_restart(hub: &HubRef, sid: &str, f: &Frame) -> Result<Value, Failure> {
    let req: IceRestartReq = parse(f).map_err(Failure::of)?;
    let t = route::sfu_target(hub, &req.sfu_id).ok_or_else(|| Failure::of(Code::PEER_NOT_FOUND))?;
    forward(hub, sid, &t, f).await
}

fn task(hub: &HubRef, sid: &str, f: &Frame) -> Result<Value, Failure> {
    let req: TaskReq = parse(f).map_err(Failure::of)?;
    if let TaskReq::Report { req_id, result, .. } = req {
        hub.probes.report(sid, req_id, result);
    }
    Ok(json!({}))
}

async fn resume(hub: &HubRef, c: &Arc<Conn>, sid: &str) -> Result<Value, Failure> {
    if c.resumed.load(Ordering::Relaxed) == 0 {
        return Err(Failure::of(Code::SESSION_NOT_FOUND));
    }
    let (data, ctx) = session_ctx(hub, sid).ok_or_else(|| Failure::of(Code::SESSION_NOT_FOUND))?;
    let mut snapshot = serde_json::Map::new();
    let mut publications: Vec<Value> = Vec::new();
    let mut tables: Vec<Value> = Vec::new();
    for (sfu, srv) in &data.servers {
        let t = Target { node: srv.node.clone(), inst: Some(sfu.clone()) };
        let part = match route::call(hub, &t, &Call::Resume { ctx: ctx.clone() }).await {
            Reply::Ok { body } => body,
            _ => return Err(Failure::of(Code::SFU_ERROR)),
        };
        let mut rooms = BTreeSet::new();
        if let Some(m) = part.get("snapshot").and_then(|v| v.as_object()) {
            for (k, v) in m {
                rooms.insert(k.clone());
                snapshot.insert(k.clone(), v.clone());
            }
        }
        if let Some(p) = part.get("publications").and_then(|v| v.as_array()) {
            publications.extend(p.iter().cloned());
        }
        if let Some(tb) = part.get("mline_table").filter(|v| !v.is_null()) {
            tables.push(tb.clone());
        }
        hub.set_server_rooms(sid, sfu, &srv.node, rooms);
    }
    common::ev!(spec = "CP-OP-RESUME", session = %data.fingerprint(), user = %data.user_id, "이어받기 스냅샷");
    Ok(json!({"snapshot": snapshot, "publications": publications, "mline_tables": tables}))
}

async fn bind(hub: &HubRef, c: &Arc<Conn>, f: &Frame) -> Result<BindRes, Code> {
    let req: BindReq = parse(f)?;
    if req.client_ver.unwrap_or(1) != oxsig::WIRE_VERSION {
        return Err(Code::VERSION_MISMATCH);
    }
    let hb = u64::from(hub.cfg.policy.hub.heartbeat_interval_ms);
    let win = u64::from(hub.cfg.policy.hub.resume_window_ms);
    let local_live = req.session_id.as_deref().is_some_and(|s| hub.sessions.lock().map(|t| t.get(s).is_some()).unwrap_or(false));
    match bind_branch(req.session_id.as_deref(), local_live) {
        Branch::Resume(sid) => {
            let prev = hub.sessions.lock().ok().and_then(|mut t| t.attach(&sid, c.id));
            if let Some(p) = prev
                && p != c.id
            {
                hub.send_leave(p, Code::DUPLICATE_SESSION);
            }
            let data = hub.sessions.lock().ok().and_then(|t| t.get(&sid).map(|s| s.data.clone())).ok_or(Code::SESSION_NOT_FOUND)?;
            c.sid.lock().map_err(|_| Code::INTERNAL_ERROR)?.replace(sid.clone());
            c.resumed.store(1, Ordering::Relaxed);
            common::ev!(spec = "SES.S.bind_resume", area = "SES", key = %data.fingerprint(), from = "DETACHED", to = "BOUND", cause = "BIND", session = %data.fingerprint(), user = %data.user_id);
            return Ok(BindRes { user_id: data.user_id, server_ver: oxsig::WIRE_VERSION, heartbeat_interval: hb, session_id: sid, resume_window_ms: win, pc_mode: data.pc_mode });
        }
        Branch::Elsewhere(sid) => {
            if let Some(data) = handover(hub, &sid).await {
                install(hub, c, data.clone(), true).await;
                common::ev!(spec = "SES.S.bind_resume", area = "SES", key = %data.fingerprint(), from = "ELSEWHERE", to = "BOUND", cause = "BIND", session = %data.fingerprint(), user = %data.user_id);
                return Ok(BindRes { user_id: data.user_id, server_ver: oxsig::WIRE_VERSION, heartbeat_interval: hb, session_id: sid, resume_window_ms: win, pc_mode: data.pc_mode });
            }
        }
        Branch::Token => {}
    }
    let secrets = token::secrets(&hub.cfg.system.hub.auth.api_keys, hub.cfg.system.hub.auth.jwt_secret.as_deref());
    let id = token::verify(&req.token, &secrets)?;
    let max = hub.cfg.policy.quota.max_sessions;
    if max != 0 {
        let n = match hub.bus() {
            Some(b) => b.tokens(&format!("ox/{}/int/sess/**", hub.ks.ns), common::bus::SESS_GET_TIMEOUT_MS).await.len(),
            None => hub.sessions.lock().map(|t| t.len()).unwrap_or(0),
        };
        if n >= max as usize {
            return Err(Code::QUOTA_EXCEEDED);
        }
    }
    let sid = new_session_id();
    let data = SessionData {
        sid: sid.clone(),
        user_id: id.user_id.clone(),
        participant_type: id.participant_type,
        hidden: id.hidden,
        metadata: id.metadata,
        permission: id.permission,
        pc_mode: req.pc_mode.unwrap_or(PcMode::V2pc),
        home_node: hub.me().to_string(),
        home_inst: hub.epoch.clone(),
        servers: Default::default(),
    };
    evict_elsewhere(hub, &id.user_id).await;
    install(hub, c, data.clone(), false).await;
    common::ev!(spec = "SES.S.bind_new", area = "SES", key = %data.fingerprint(), from = "NONE", to = "BOUND", cause = "BIND", session = %data.fingerprint(), user = %data.user_id);
    Ok(BindRes { user_id: data.user_id, server_ver: oxsig::WIRE_VERSION, heartbeat_interval: hb, session_id: sid, resume_window_ms: win, pc_mode: data.pc_mode })
}

async fn install(hub: &HubRef, c: &Arc<Conn>, data: SessionData, resumed: bool) {
    hub.user_interest(&data.user_id).await;
    let old = hub.sessions.lock().ok().and_then(|mut t| t.insert(data.clone(), c.id));
    if let Some(old) = old {
        if let Some(sock) = old.sock {
            hub.send_leave(sock, Code::DUPLICATE_SESSION);
        }
        let h = hub.clone();
        let old_sid = old.data.sid.clone();
        let old_user = old.data.user_id.clone();
        let fp = old.data.fingerprint();
        let servers = old.data.servers.clone();
        let rooms = old.data.all_rooms();
        tokio::spawn(async move {
            for r in rooms {
                h.interest_dec(&r).await;
            }
            h.sess_tokens.lock().await.remove(&old_sid);
            h.user_interest_drop(&old_user).await;
            for (sfu, srv) in servers {
                let call = Call::SessionGone { user_id: old_user.clone(), session: fp.clone(), code: Code::DUPLICATE_SESSION.0 };
                let _ = route::call(&h, &Target { node: srv.node, inst: Some(sfu) }, &call).await;
            }
        });
    }
    for r in data.all_rooms() {
        hub.interest_inc(&r).await;
    }
    if let Some(b) = hub.bus()
        && let Ok(t) = b.declare_sess(&data.user_id, &data.sid).await
    {
        hub.sess_tokens.lock().await.insert(data.sid.clone(), t);
    }
    if let Ok(mut s) = c.sid.lock() {
        s.replace(data.sid.clone());
    }
    c.resumed.store(u8::from(resumed), Ordering::Relaxed);
}

async fn handover(hub: &HubRef, sid: &str) -> Option<SessionData> {
    let bus = hub.bus()?;
    let keys = bus.tokens(&format!("ox/{}/int/sess/*/*/{sid}", hub.ks.ns), common::bus::SESS_GET_TIMEOUT_MS).await;
    let tok = keys.iter().find_map(|k| hub.ks.parse_sess(k))?;
    if tok.node == hub.me() {
        return None;
    }
    let key = format!("{}?sid={sid}", hub.ks.q_node(&tok.node, "sess"));
    let bytes = bus.get_one(&key, Vec::new(), common::bus::SESS_GET_TIMEOUT_MS).await.ok()?;
    let mut data: SessionData = serde_json::from_slice(&bytes).ok()?;
    data.home_node = hub.me().to_string();
    data.home_inst = hub.epoch.clone();
    Some(data)
}

async fn evict_elsewhere(hub: &HubRef, user: &str) {
    let Some(bus) = hub.bus() else { return };
    let keys = bus.tokens(&hub.ks.sess_of_user(user), common::bus::SESS_GET_TIMEOUT_MS).await;
    let nodes: BTreeSet<String> = keys.iter().filter_map(|k| hub.ks.parse_sess(k)).map(|t| t.node).filter(|n| n != hub.me()).collect();
    for n in nodes {
        let _ = bus.put(&hub.ks.ev_node(&n), serde_json::to_vec(&json!({"evict_user": user})).unwrap_or_default()).await;
    }
}
