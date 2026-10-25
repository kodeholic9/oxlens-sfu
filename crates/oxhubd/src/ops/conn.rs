// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-SES-READ-AND-PROCESS 정:IMPL-SES-SOCKET-LIFE 정:IMPL-SES-BIND 연:RULE-TRN-CLOSE 연:RULE-TRN-FLOW-CONTROL 연:RULE-TRN-LANES 연:RULE-TRN-WINDOW-PER-DIRECTION 연:RULE-TRN-NOTIFY-ACK 연:RULE-TRN-ONE-REPLY 연:RULE-SES-BIND-PROC 연:RULE-SES-HEARTBEAT 연:RULE-SES-LEAVE-PROC 연:RULE-SES-LEAVE-CARRIES-REASON 연:DEV-TRN-WS-ORIGIN 연:SES.S.pre_bind_other 연:SES.fact.LeaveReasons · model: claude-opus-5-5
//! 소켓 하나 — 읽기 고리(HEARTBEAT 즉답 · `BIND` 전 다른 op = `LEAVE 2001`) · 처리 고리(연결 단위 순서) · 쓰기 고리(단 넷 · 서버 송신 창 · ACK 시한 · `LEAVE` 는 큐 밖).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use oxsig::frame::{Frame, FrameError, Kind};
use oxsig::msg::LeaveBody;
use oxsig::{Code, Op};
use tokio::sync::mpsc;

use super::bind;
use super::sess::destroy;
use crate::ops::hub::Hub;
use crate::ses::{Gone, Out, Session};

static CONN_SEQ: AtomicU64 = AtomicU64::new(1);
const BACKLOG_MAX: usize = 1000;
const ACK_TIMEOUT: Duration = Duration::from_secs(30);
const BIND_TIMEOUT: Duration = Duration::from_secs(10);

pub fn origin_ok(hub: &Hub, origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(o) = origin else { return true };
    let allowed = &hub.cfg.policy.hub.allowed_origins;
    if allowed.iter().any(|a| a == "*" || a == o) {
        return true;
    }
    let authority = o.split("://").nth(1).unwrap_or(o);
    host.is_some_and(|h| h.eq_ignore_ascii_case(authority))
}

fn leave_frame(code: Code, pid: u32) -> Vec<u8> {
    let body = LeaveBody { code: code.num(), name: code.name().to_string(), message: None, details: None };
    Frame::request(Op::Leave, pid, serde_json::to_vec(&body).unwrap_or_default()).encode()
}

fn fail_body(code: Code) -> Vec<u8> {
    serde_json::to_vec(&code.failure()).unwrap_or_default()
}

enum WOut {
    Out(Out),
    Acked(u32),
}

pub async fn run(hub: Arc<Hub>, sock: WebSocket) {
    let conn_id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (sink, mut stream) = sock.split();
    let (tx, rx) = mpsc::unbounded_channel::<Out>();
    let (wtx, wrx) = mpsc::unbounded_channel::<WOut>();
    let window = usize::from(hub.cfg.policy.hub.ws_flow_window.max(1));
    let fwd = wtx.clone();
    let mut out_rx = rx;
    tokio::spawn(async move {
        while let Some(o) = out_rx.recv().await {
            if fwd.send(WOut::Out(o)).is_err() {
                break;
            }
        }
    });
    let writer = tokio::spawn(write_loop(sink, wrx, window));
    let idle = Duration::from_millis(u64::from(hub.cfg.policy.hub.heartbeat_timeout_ms));
    let mut bound: Option<Arc<Session>> = None;
    let mut pending: Option<(u32, tokio::task::JoinHandle<Result<bind::Bound, Code>>)> = None;
    let (req_tx, req_rx) = mpsc::channel::<(u16, u32, Vec<u8>)>(BACKLOG_MAX);
    let mut req_rx = Some(req_rx);
    let opened = Instant::now();
    let mut last = Instant::now();
    let mut closed_by_leave = false;
    loop {
        let deadline = if bound.is_some() { last + idle } else { opened + BIND_TIMEOUT };
        tokio::select! {
            m = stream.next() => {
                let Some(Ok(m)) = m else { break };
                last = Instant::now();
                let b = match m {
                    Message::Binary(b) => b,
                    Message::Ping(_) | Message::Pong(_) => continue,
                    Message::Close(_) => break,
                    Message::Text(_) => {
                        let _ = tx.send(Out::Leave(Code::ProtocolError));
                        closed_by_leave = true;
                        break;
                    }
                };
                let f = match Frame::decode(&b) {
                    Ok(f) => f,
                    Err(FrameError::TooLarge) | Err(FrameError::TooShort) | Err(FrameError::BadVersion) | Err(FrameError::ReservedKind) => {
                        let _ = tx.send(Out::Leave(Code::ProtocolError));
                        closed_by_leave = true;
                        break;
                    }
                };
                if bound.is_none() {
                    if f.kind == Kind::Request && f.op == Op::Bind.as_u16() && pending.is_none() {
                        match bind::parse_bind(&f.body) {
                            Ok(req) => {
                                let h = hub.clone();
                                let t = tx.clone();
                                pending = Some((f.pid, tokio::spawn(async move { bind::bind(&h, req, conn_id, t).await })));
                            }
                            Err(c) => {
                                let _ = tx.send(Out::Reply(Frame::reply_fail(f.op, f.pid, fail_body(c))));
                                if c == Code::InvalidPayload && !f.body.is_empty() && serde_json::from_slice::<serde_json::Value>(&f.body).is_err() {
                                    let _ = tx.send(Out::Leave(Code::ProtocolError));
                                    closed_by_leave = true;
                                    break;
                                }
                            }
                        }
                        continue;
                    }
                    common::ev!(spec = "CP-CODE-NOT-BOUND", "op 0x{:04x} before BIND — LEAVE 2001", f.op);
                    let _ = tx.send(Out::Leave(Code::NotBound));
                    closed_by_leave = true;
                    break;
                }
                let Some(s) = bound.clone() else { break };
                match f.kind {
                    Kind::Ok | Kind::Fail => {
                        let _ = wtx.send(WOut::Acked(f.pid));
                    }
                    Kind::Request => {
                        let op = Op::from_u16(f.op);
                        match op {
                            Some(Op::Heartbeat) => {
                                let _ = tx.send(Out::Reply(Frame::reply_ok(f.op, f.pid, Vec::new())));
                            }
                            Some(Op::Bind) => {
                                let _ = tx.send(Out::Reply(Frame::reply_fail(f.op, f.pid, fail_body(Code::InvalidPayload))));
                            }
                            Some(Op::Leave) => {
                                common::ev!(spec = "CP-OP-LEAVE", session = s.fp.as_str(), user = s.id.user.as_str(), "client LEAVE — session discarded without window");
                                destroy(&hub, &s, Gone::Leave, None).await;
                                closed_by_leave = true;
                                break;
                            }
                            _ => {
                                if !f.body.is_empty() && serde_json::from_slice::<serde_json::Value>(&f.body).is_err() {
                                    let _ = tx.send(Out::Reply(Frame::reply_fail(f.op, f.pid, fail_body(Code::InvalidPayload))));
                                    let _ = tx.send(Out::Leave(Code::ProtocolError));
                                    closed_by_leave = true;
                                    break;
                                }
                                if req_tx.try_send((f.op, f.pid, f.body)).is_err() {
                                    let _ = tx.send(Out::Leave(Code::FlowOverflow));
                                    closed_by_leave = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            r = async { match pending.as_mut() { Some((_, h)) => h.await.ok(), None => std::future::pending().await } } => {
                let Some((pid, _)) = pending.take() else { continue };
                match r {
                    Some(Ok(b)) => {
                        let body = serde_json::to_vec(&b.res).unwrap_or_default();
                        let _ = tx.send(Out::Reply(Frame::reply_ok(Op::Bind.as_u16(), pid, body)));
                        let s = b.session.clone();
                        bound = Some(b.session);
                        if let Some(rx) = req_rx.take() {
                            tokio::spawn(process(hub.clone(), s, rx, tx.clone()));
                        }
                    }
                    Some(Err(c)) => {
                        common::ev!(spec = c.catalog_id().as_str(), "BIND failed {}", c.num());
                        let _ = tx.send(Out::Reply(Frame::reply_fail(Op::Bind.as_u16(), pid, fail_body(c))));
                    }
                    None => {
                        let _ = tx.send(Out::Reply(Frame::reply_fail(Op::Bind.as_u16(), pid, fail_body(Code::InternalError))));
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline.into()) => {
                common::ev!(spec = "CP-CODE-HEARTBEAT-TIMEOUT", session = bound.as_ref().map(|s| s.fp.as_str()).unwrap_or(""), "socket silent — LEAVE 4008");
                let _ = tx.send(Out::Leave(Code::HeartbeatTimeout));
                closed_by_leave = true;
                break;
            }
        }
    }
    if let Some(s) = &bound {
        let mut g = s.inner.lock().await;
        if g.conn.as_ref().is_some_and(|c| c.id == conn_id) {
            g.conn = None;
            g.gone_at = Some(Instant::now());
        }
    }
    if !closed_by_leave {
        tracing::debug!("socket {conn_id} closed by peer");
    }
    drop(tx);
    drop(wtx);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

async fn process(hub: Arc<Hub>, s: Arc<Session>, mut rx: mpsc::Receiver<(u16, u32, Vec<u8>)>, tx: mpsc::UnboundedSender<Out>) {
    while let Some((op, pid, body)) = rx.recv().await {
        let f = match crate::ops::route::handle(&hub, &s, op, body).await {
            Ok(b) => Frame::reply_ok(op, pid, b),
            Err(fl) => {
                hub.count_code(fl.code).await;
                Frame::reply_fail(op, pid, serde_json::to_vec(&fl).unwrap_or_default())
            }
        };
        if tx.send(Out::Reply(f)).is_err() {
            break;
        }
    }
}

async fn write_loop(mut sink: futures_util::stream::SplitSink<WebSocket, Message>, mut rx: mpsc::UnboundedReceiver<WOut>, window: usize) {
    let mut lanes: [VecDeque<(u16, Vec<u8>)>; 4] = Default::default();
    let mut inflight: HashMap<u32, Instant> = HashMap::new();
    let mut next_pid: u32 = 1;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        let mut leave: Option<Code> = None;
        tokio::select! {
            m = rx.recv() => {
                let Some(m) = m else { break };
                match m {
                    WOut::Out(Out::Reply(f)) => {
                        if sink.send(Message::Binary(f.encode())).await.is_err() {
                            break;
                        }
                    }
                    WOut::Out(Out::Notify { op, body }) => {
                        let lane = Op::from_u16(op).map(|o| o.lane().index()).unwrap_or(1);
                        lanes[lane].push_back((op, body));
                        if lanes.iter().map(VecDeque::len).sum::<usize>() > BACKLOG_MAX {
                            leave = Some(Code::FlowOverflow);
                        }
                    }
                    WOut::Out(Out::Leave(c)) => leave = Some(c),
                    WOut::Acked(pid) => {
                        inflight.remove(&pid);
                    }
                }
            }
            _ = tick.tick() => {
                if inflight.values().any(|t| t.elapsed() > ACK_TIMEOUT) {
                    leave = Some(Code::FlowTimeout);
                }
            }
        }
        if let Some(c) = leave {
            let pid = next_pid;
            let _ = sink.send(Message::Binary(leave_frame(c, pid))).await;
            let _ = sink.send(Message::Close(None)).await;
            break;
        }
        while inflight.len() < window {
            let Some((op, body)) = lanes.iter_mut().find_map(VecDeque::pop_front) else {
                break;
            };
            let pid = next_pid;
            next_pid = next_pid.wrapping_add(1);
            let f = Frame { kind: Kind::Request, reserved: 0, op, pid, body };
            if sink.send(Message::Binary(f.encode())).await.is_err() {
                return;
            }
            inflight.insert(pid, Instant::now());
        }
    }
    let _ = sink.close().await;
}
