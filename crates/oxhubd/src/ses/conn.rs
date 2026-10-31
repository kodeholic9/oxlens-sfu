// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SES-READ-AND-PROCESS · 정:IMPL-SES-SOCKET-LIFE · 연:RULE-TRN-CLOSE · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-WINDOW-PER-DIRECTION · 연:RULE-TRN-LANES · 연:RULE-SES-BIND-PROC · 연:RULE-SES-HEARTBEAT · 연:RULE-SES-LEAVE-CARRIES-REASON · 정:CAND-SES-PREBIND-2001 · model: claude-opus-5-5
//! WS 연결 하나 — 읽기와 처리를 가른다 · `HEARTBEAT` 는 읽는 즉시 · 방 요청은 연결 순서대로 · 서버 통지는 4단 큐 + ACK 창 · 끊을 때는 `LEAVE` 먼저.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use oxsig::frame::{self, FrameKind};
use oxsig::{Code, Op};
use tokio::sync::mpsc;

use super::{Ctx, Session};

pub const T_BIND: Duration = Duration::from_secs(10);
pub const T_IDLE: Duration = Duration::from_secs(30);
pub const T_ACK: Duration = Duration::from_secs(30);
pub const QUEUE_MAX: usize = 1_000;

static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

pub enum Out {
    Reply { flags: u8, op: u16, pid: u32, body: Vec<u8> },
    Notice { op: Op, body: Vec<u8> },
    Ack(u32),
    Leave { code: Code, details: Option<serde_json::Value> },
    Close,
}

#[derive(Clone)]
pub struct Conn {
    pub id: u64,
    pub tx: mpsc::UnboundedSender<Out>,
}

impl Conn {
    pub fn send(&self, o: Out) -> bool {
        self.tx.send(o).is_ok()
    }

    pub fn reply_ok(&self, op: u16, pid: u32, req_flags: u8, body: Vec<u8>) {
        let flags = (req_flags & !frame::KIND_MASK) | FrameKind::Ok.bits();
        let _ = self.tx.send(Out::Reply { flags, op, pid, body });
    }

    pub fn reply_fail(&self, op: u16, pid: u32, req_flags: u8, f: &oxsig::Failure) {
        let flags = (req_flags & !frame::KIND_MASK) | FrameKind::Fail.bits();
        let _ = self.tx.send(Out::Reply { flags, op, pid, body: serde_json::to_vec(f).unwrap_or_default() });
    }

    pub fn leave(&self, code: Code) {
        let _ = self.tx.send(Out::Leave { code, details: None });
    }
}

pub struct Request {
    pub op: Op,
    pub op_num: u16,
    pub pid: u32,
    pub flags: u8,
    pub body: Vec<u8>,
}

struct Writer {
    lanes: [VecDeque<(Op, Vec<u8>)>; 4],
    inflight: HashMap<u32, Instant>,
    next_pid: u32,
    window: usize,
}

impl Writer {
    fn queued(&self) -> usize {
        self.lanes.iter().map(|l| l.len()).sum()
    }

    fn next(&mut self) -> Option<(Op, Vec<u8>)> {
        if self.inflight.len() >= self.window {
            return None;
        }
        self.lanes.iter_mut().find_map(|l| l.pop_front())
    }

    fn pid(&mut self) -> u32 {
        let p = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);
        p
    }
}

pub async fn serve(ws: WebSocket, ctx: Arc<Ctx>) {
    let id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let conn = Conn { id, tx: tx.clone() };
    let window = ctx.policy.hub.ws_flow_window.clamp(1, 10) as usize;
    let (req_tx, mut req_rx) = mpsc::unbounded_channel::<(Arc<Session>, Request)>();
    let leave_code = Arc::new(std::sync::Mutex::new(None::<Code>));

    let lc = leave_code.clone();
    let writer = tokio::spawn(async move {
        let mut w = Writer { lanes: Default::default(), inflight: HashMap::new(), next_pid: 1, window };
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            let ev = tokio::select! {
                o = rx.recv() => o,
                _ = tick.tick() => {
                    if w.inflight.values().any(|t| t.elapsed() > T_ACK) {
                        Some(Out::Leave { code: Code::FlowTimeout, details: None })
                    } else {
                        continue;
                    }
                }
            };
            let Some(o) = ev else { break };
            match o {
                Out::Reply { flags, op, pid, body } => {
                    if sink.send(Message::Binary(frame::encode_flags(flags, op, pid, &body))).await.is_err() {
                        break;
                    }
                }
                Out::Notice { op, body } => {
                    w.lanes[op.lane() as usize].push_back((op, body));
                    if w.queued() > QUEUE_MAX {
                        let _ = send_leave(&mut sink, &mut w, Code::FlowOverflow, None).await;
                        if let Ok(mut g) = lc.lock() {
                            g.replace(Code::FlowOverflow);
                        }
                        break;
                    }
                }
                Out::Ack(pid) => {
                    w.inflight.remove(&pid);
                }
                Out::Leave { code, details } => {
                    let _ = send_leave(&mut sink, &mut w, code, details).await;
                    if let Ok(mut g) = lc.lock() {
                        g.get_or_insert(code);
                    }
                    break;
                }
                Out::Close => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
            while let Some((op, body)) = w.next() {
                let pid = w.pid();
                w.inflight.insert(pid, Instant::now());
                if sink.send(Message::Binary(frame::encode(FrameKind::Request, op.as_u16(), pid, &body))).await.is_err() {
                    return;
                }
            }
        }
        let _ = sink.close().await;
    });

    let pctx = ctx.clone();
    let pconn = conn.clone();
    let processor = tokio::spawn(async move {
        while let Some((s, r)) = req_rx.recv().await {
            super::handle_request(&pctx, &s, &pconn, r).await;
        }
    });

    let mut session: Option<Arc<Session>> = None;
    loop {
        let limit = if session.is_none() { T_BIND } else { T_IDLE };
        let msg = match tokio::time::timeout(limit, stream.next()).await {
            Err(_) => {
                conn.leave(Code::HeartbeatTimeout);
                break;
            }
            Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(m))) => m,
        };
        let data = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            Message::Text(_) => {
                conn.leave(Code::ProtocolError);
                break;
            }
            _ => continue,
        };
        let f = match frame::decode(&data) {
            Ok(f) => f,
            Err(_) => {
                conn.leave(Code::ProtocolError);
                break;
            }
        };
        let h = f.header;
        match h.kind() {
            FrameKind::Ok | FrameKind::Fail => {
                let _ = conn.tx.send(Out::Ack(h.pid));
                continue;
            }
            FrameKind::Request => {}
        }
        if !f.body.is_empty() && serde_json::from_slice::<serde_json::Value>(f.body).is_err() {
            conn.reply_fail(h.op, h.pid, h.flags, &Code::ProtocolError.failure());
            conn.leave(Code::ProtocolError);
            break;
        }
        let op = Op::from_u16(h.op);
        let Some(s) = session.clone() else {
            if op == Some(Op::Bind) {
                let judge = super::bind(&ctx, &conn, h.op, h.pid, h.flags, f.body);
                tokio::pin!(judge);
                let mut early = false;
                let mut gone = false;
                let got = loop {
                    tokio::select! {
                        biased;
                        r = &mut judge => break r,
                        m = stream.next(), if !early && !gone => match m {
                            Some(Ok(Message::Binary(b))) => {
                                super::pre_bind_other(&ctx, frame::decode(&b).ok().and_then(|x| Op::from_u16(x.header.op)));
                                conn.leave(Code::NotBound);
                                early = true;
                            }
                            Some(Ok(Message::Text(_))) => {
                                super::pre_bind_other(&ctx, None);
                                conn.leave(Code::NotBound);
                                early = true;
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => gone = true,
                            Some(Ok(_)) => {}
                        },
                    }
                };
                session = got;
                if early || gone {
                    break;
                }
                continue;
            }
            super::pre_bind_other(&ctx, op);
            conn.leave(Code::NotBound);
            break;
        };
        match op {
            Some(Op::Heartbeat) => conn.reply_ok(h.op, h.pid, h.flags, Vec::new()),
            Some(Op::Bind) => {
                super::bind_bound(&ctx, &s);
                conn.reply_fail(h.op, h.pid, h.flags, &Code::ProtocolError.failure());
                conn.leave(Code::ProtocolError);
                break;
            }
            Some(Op::Leave) => {
                super::client_leave(&ctx, &s, &conn).await;
                let _ = conn.tx.send(Out::Close);
                session = None;
                break;
            }
            Some(op) => {
                let _ = req_tx.send((s, Request { op, op_num: h.op, pid: h.pid, flags: h.flags, body: f.body.to_vec() }));
            }
            None => conn.reply_fail(h.op, h.pid, h.flags, &Code::UnknownOp.failure()),
        }
    }
    drop(req_tx);
    let _ = processor.await;
    let code = leave_code.lock().ok().and_then(|g| *g);
    if let Some(s) = session {
        super::socket_dropped(&ctx, &s, conn.id, code).await;
    }
    let _ = conn.tx.send(Out::Close);
    let _ = writer.await;
}

async fn send_leave(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    w: &mut Writer,
    code: Code,
    details: Option<serde_json::Value>,
) -> Result<(), axum::Error> {
    let mut body = code.leave();
    body.details = details;
    let pid = w.pid();
    let bytes = frame::encode(FrameKind::Request, Op::Leave.as_u16(), pid, &serde_json::to_vec(&body).unwrap_or_default());
    sink.send(Message::Binary(bytes)).await?;
    sink.send(Message::Close(None)).await
}
