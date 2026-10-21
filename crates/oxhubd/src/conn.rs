// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 연:TRN.frame.WS 연:RULE-TRN-CLOSE 연:RULE-TRN-FLOW-CONTROL 연:RULE-TRN-LANES 연:RULE-TRN-WINDOW-PER-DIRECTION 연:RULE-SES-LEAVE-PROC 정:RULE-SES-READ-AND-PROCESS 정:IMPL-SES-SOCKET-LIFE · model: claude-opus-5-5
//! WS 연결 하나 — 읽기와 처리를 가른다. 응답은 곧바로, 서버 통지는 창 안에서 단 순서로, `LEAVE` 는 큐 밖으로.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use oxsig::frame::PidCounter;
use oxsig::wire::LeaveBody;
use oxsig::{Code, Frame, FrameError, Kind, Lane, Op};
use tokio::sync::mpsc;

use crate::state::Hub;

const BACKLOG_MAX: usize = 1_000;
const ACK_TIMEOUT: Duration = Duration::from_millis(30_000);
const BIND_TIMEOUT: Duration = Duration::from_millis(10_000);

static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

enum Out {
    Reply(Frame),
    Notify(Op, Vec<u8>),
    Ack(u32),
    Leave(LeaveBody),
    Close,
}

#[derive(Clone)]
pub struct ConnHandle {
    pub id: u64,
    tx: mpsc::UnboundedSender<Out>,
    backlog: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl ConnHandle {
    fn push(&self, o: Out) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        if matches!(o, Out::Reply(_) | Out::Notify(..)) {
            self.backlog.fetch_add(1, Ordering::Relaxed);
        }
        let _ = self.tx.send(o);
    }

    pub fn reply(&self, f: Frame) {
        self.push(Out::Reply(f));
    }

    pub fn notify(&self, op: Op, body: Vec<u8>) {
        self.push(Out::Notify(op, body));
    }

    pub fn leave(&self, code: Code) {
        self.push(Out::Leave(code.leave()));
    }

    pub fn close(&self) {
        self.push(Out::Close);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn ack(&self, pid: u32) {
        let _ = self.tx.send(Out::Ack(pid));
    }
}

fn lane_idx(l: Lane) -> usize {
    match l {
        Lane::Recovery => 0,
        Lane::Session => 1,
        Lane::Data => 2,
        Lane::Diagnostic => 3,
    }
}

async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, f: &Frame) -> bool {
    sink.send(Message::Binary(f.encode())).await.is_ok()
}

async fn writer(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::UnboundedReceiver<Out>,
    backlog: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
    window: usize,
) {
    let mut pid = PidCounter::default();
    let mut lanes: [VecDeque<(Op, Vec<u8>)>; 4] = Default::default();
    let mut inflight: HashMap<u32, Instant> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(1_000));
    let mut leave_with: Option<LeaveBody> = None;
    'outer: loop {
        tokio::select! {
            o = rx.recv() => match o {
                None | Some(Out::Close) => break,
                Some(Out::Reply(f)) => {
                    backlog.fetch_sub(1, Ordering::Relaxed);
                    if !send(&mut sink, &f).await { break }
                }
                Some(Out::Notify(op, body)) => {
                    backlog.fetch_sub(1, Ordering::Relaxed);
                    lanes[lane_idx(op.lane())].push_back((op, body));
                }
                Some(Out::Ack(p)) => { inflight.remove(&p); }
                Some(Out::Leave(b)) => { leave_with = Some(b); break }
            },
            _ = tick.tick() => {
                if inflight.values().any(|t| t.elapsed() > ACK_TIMEOUT) {
                    leave_with = Some(Code::FLOW_TIMEOUT.leave());
                    break;
                }
            }
        }
        let queued: usize = lanes.iter().map(VecDeque::len).sum::<usize>() + backlog.load(Ordering::Relaxed);
        if queued > BACKLOG_MAX {
            leave_with = Some(Code::FLOW_OVERFLOW.leave());
            break;
        }
        while inflight.len() < window {
            let Some(q) = lanes.iter_mut().find(|q| !q.is_empty()) else { break };
            let Some((op, body)) = q.pop_front() else { break };
            let p = pid.take();
            inflight.insert(p, Instant::now());
            if !send(&mut sink, &Frame::request(op.as_u16(), p, body)).await {
                break 'outer;
            }
        }
    }
    closed.store(true, Ordering::Relaxed);
    if let Some(b) = leave_with {
        let body = serde_json::to_vec(&b).unwrap_or_default();
        let p = pid.take();
        let _ = send(&mut sink, &Frame::request(Op::Leave.as_u16(), p, body)).await;
    }
    let _ = sink.send(Message::Close(None)).await;
    let _ = sink.close().await;
}

pub async fn run(hub: Arc<Hub>, socket: WebSocket) {
    let (sink, mut stream) = socket.split();
    let (tx, rx) = mpsc::unbounded_channel();
    let backlog = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    let conn = ConnHandle { id: CONN_SEQ.fetch_add(1, Ordering::Relaxed), tx, backlog: backlog.clone(), closed: closed.clone() };
    let window = usize::from(hub.policy.hub.ws_flow_window.max(1));
    tokio::spawn(writer(sink, rx, backlog, closed, window));
    let (work_tx, mut work_rx) = mpsc::unbounded_channel::<(String, Frame)>();
    {
        let hub = hub.clone();
        let conn = conn.clone();
        tokio::spawn(async move {
            while let Some((sid, f)) = work_rx.recv().await {
                let r = crate::session::handle_op(&hub, &sid, &f).await;
                conn.reply(r);
            }
        });
    }
    let idle = Duration::from_millis(u64::from(hub.policy.hub.heartbeat_timeout_ms));
    let mut sid: Option<String> = None;
    loop {
        let limit = if sid.is_some() { idle } else { BIND_TIMEOUT };
        let msg = match tokio::time::timeout(limit, stream.next()).await {
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
        if f.kind != Kind::Request {
            conn.ack(f.pid);
            continue;
        }
        let Some(op) = Op::from_u16(f.op) else {
            hub.count_code(1001);
            conn.reply(crate::route::fail_frame(f.op, f.pid, &Code::UNKNOWN_OP.failure()));
            continue;
        };
        if serde_json::from_slice::<serde::de::IgnoredAny>(f.body_json()).is_err() {
            conn.reply(crate::route::fail_frame(f.op, f.pid, &Code::PROTOCOL_ERROR.failure()));
            conn.leave(Code::PROTOCOL_ERROR);
            break;
        }
        match (&sid, op) {
            (None, Op::Bind) => match crate::session::bind(&hub, &conn, &f).await {
                Ok(s) => sid = Some(s),
                Err(fr) => conn.reply(fr),
            },
            (None, _) => {
                hub.count_code(2001);
                conn.reply(crate::route::fail_frame(f.op, f.pid, &Code::NOT_BOUND.failure()));
            }
            (Some(_), Op::Heartbeat) => conn.reply(Frame::ok(f.op, f.pid, vec![])),
            (Some(s), Op::Leave) => {
                crate::session::client_leave(&hub, s).await;
                conn.close();
                sid = None;
                break;
            }
            (Some(_), Op::Bind) => {
                conn.reply(crate::route::fail_frame(f.op, f.pid, &Code::INVALID_PAYLOAD.failure_msg("already bound")))
            }
            (Some(s), _) => {
                let _ = work_tx.send((s.clone(), f));
            }
        }
    }
    if let Some(s) = sid {
        crate::session::conn_closed(&hub, &s, conn.id).await;
    }
    conn.close();
}
