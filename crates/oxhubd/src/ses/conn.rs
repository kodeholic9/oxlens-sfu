// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SES-READ-AND-PROCESS · 정:IMPL-SES-SOCKET-LIFE · 연:RULE-TRN-CLOSE · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-WINDOW-PER-DIRECTION · 연:RULE-TRN-LANES · 연:RULE-SES-BIND-PROC · 연:RULE-SES-HEARTBEAT · 연:RULE-SES-LEAVE-CARRIES-REASON · 정:CAND-SES-PREBIND-2001 · 정:RULE-CHAT-GATEWAY-MODULE · 정:IMPL-SES-DATA · 정:RULE-OPS-DROPS · 정:IMPL-CHAT-LIVE-BUNDLE · 연:RULE-CHAT-CATCHUP-BUNDLE · model: claude-opus-5-5
//! WS 연결 하나 — 읽기와 처리를 가른다 · `HEARTBEAT` 는 읽는 즉시 · 방 요청은 연결 순서대로 · 채팅 요청은 따로 둔 통로에서 연결 순서대로 · 서버 통지는 4단 큐 + ACK 창 · 창이 찼을 때만 데이터 단 줄 끝의 실시간 채팅 통지에 합친다 · 송신 통로는 건수와 바이트로 묶는다 · 끊을 때는 `LEAVE` 먼저.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use oxsig::frame::{self, FrameKind};
use oxsig::msg::{ChatMessage, MessagesBody};
use oxsig::{Code, Op};
use tokio::sync::mpsc;

use super::{Ctx, Session};

pub const T_BIND: Duration = Duration::from_secs(10);
pub const T_ACK: Duration = Duration::from_secs(30);
pub const QUEUE_MAX: usize = 1_000;

static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct Receipt {
    pub user: String,
    pub msg_ids: Vec<String>,
    pub done: Option<tokio::sync::oneshot::Sender<()>>,
    pub slot: Option<tokio::sync::OwnedSemaphorePermit>,
}

pub enum Out {
    Reply { flags: u8, op: u16, pid: u32, body: Vec<u8> },
    Notice { op: Op, body: Vec<u8>, receipt: Option<Receipt> },
    Ack(u32),
    Refused(u32),
    Leave { code: Code, details: Option<serde_json::Value> },
    Close,
}

impl Out {
    fn bytes(&self) -> usize {
        match self {
            Out::Reply { body, .. } | Out::Notice { body, .. } => body.len(),
            _ => 0,
        }
    }
}

pub struct Held {
    bytes: AtomicUsize,
    max: usize,
}

impl Held {
    pub fn new(max: usize) -> Arc<Held> {
        Arc::new(Held { bytes: AtomicUsize::new(0), max })
    }

    pub fn bytes(&self) -> usize {
        self.bytes.load(Ordering::Acquire)
    }

    fn take(&self, n: usize) -> bool {
        if self.bytes.fetch_add(n, Ordering::AcqRel) + n > self.max {
            self.give(n);
            return false;
        }
        true
    }

    fn grow(&self, n: usize) {
        self.bytes.fetch_add(n, Ordering::AcqRel);
    }

    fn give(&self, n: usize) {
        self.bytes.fetch_sub(n, Ordering::AcqRel);
    }

    fn over(&self) -> bool {
        self.bytes() > self.max
    }
}

#[derive(Clone)]
pub struct Conn {
    pub id: u64,
    pub tx: mpsc::Sender<Out>,
    pub over: Arc<tokio::sync::Notify>,
    pub held: Arc<Held>,
}

impl Conn {
    pub fn send(&self, o: Out) -> bool {
        let n = o.bytes();
        if !self.held.take(n) {
            self.over.notify_one();
            return false;
        }
        match self.tx.try_send(o) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.held.give(n);
                self.over.notify_one();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.held.give(n);
                false
            }
        }
    }

    pub fn reply_ok(&self, op: u16, pid: u32, req_flags: u8, body: Vec<u8>) {
        let flags = (req_flags & !frame::KIND_MASK) | FrameKind::Ok.bits();
        self.send(Out::Reply { flags, op, pid, body });
    }

    pub fn reply_fail(&self, op: u16, pid: u32, req_flags: u8, f: &oxsig::Failure) {
        let flags = (req_flags & !frame::KIND_MASK) | FrameKind::Fail.bits();
        self.send(Out::Reply { flags, op, pid, body: serde_json::to_vec(f).unwrap_or_default() });
    }

    pub fn leave(&self, code: Code, s: Option<&super::Session>) {
        super::code_line(&code.failure(), "LEAVE", s, None);
        self.send(Out::Leave { code, details: None });
    }
}

pub struct Request {
    pub op: Op,
    pub op_num: u16,
    pub pid: u32,
    pub flags: u8,
    pub body: Vec<u8>,
}

enum Body {
    Bytes(Vec<u8>),
    Live(Vec<ChatMessage>),
}

struct Item {
    op: Op,
    body: Body,
    receipt: Option<Receipt>,
    size: usize,
}

impl Item {
    fn join(&mut self, body: &[u8], receipt: Option<Receipt>, wrap: usize, max: usize) -> Result<usize, Option<Receipt>> {
        let grown = match &self.body {
            Body::Live(_) => self.size + 1 + body.len(),
            Body::Bytes(_) if self.op == Op::Message => wrap + self.size + 1 + body.len(),
            Body::Bytes(_) => return Err(receipt),
        };
        if grown > max {
            return Err(receipt);
        }
        let Ok(m) = serde_json::from_slice::<ChatMessage>(body) else { return Err(receipt) };
        if let Body::Bytes(b) = &self.body {
            let Ok(first) = serde_json::from_slice::<ChatMessage>(b) else { return Err(receipt) };
            self.body = Body::Live(vec![first]);
            self.op = Op::Messages;
        }
        if let Body::Live(v) = &mut self.body {
            let at = v.partition_point(|x| (x.t, x.msg_id.as_str()) <= (m.t, m.msg_id.as_str()));
            v.insert(at, m);
        }
        if let Some(n) = receipt {
            match &mut self.receipt {
                Some(r) => r.msg_ids.extend(n.msg_ids),
                slot => *slot = Some(n),
            }
        }
        let added = grown - self.size - body.len();
        self.size = grown;
        Ok(added)
    }
}

pub struct Writer {
    lanes: [VecDeque<Item>; 4],
    inflight: HashMap<u32, (Instant, Option<Receipt>)>,
    next_pid: u32,
    window: usize,
    bundle_max: usize,
    wrap: usize,
    held: Arc<Held>,
}

fn acked(ctx: &Ctx, r: Receipt) {
    if let Some(chat) = ctx.chat.get() {
        chat.acked(&r.user, r.msg_ids);
    }
    if let Some(d) = r.done {
        let _ = d.send(());
    }
}

impl Writer {
    pub fn new(window: usize, bundle_max: usize, held: Arc<Held>) -> Writer {
        let wrap = serde_json::to_vec(&MessagesBody { messages: Vec::new(), live: Some(true) }).map(|v| v.len()).unwrap_or(0);
        Writer { lanes: Default::default(), inflight: HashMap::new(), next_pid: 1, window, bundle_max, wrap, held }
    }

    pub fn queued(&self) -> usize {
        self.lanes.iter().map(|l| l.len()).sum()
    }

    pub fn push(&mut self, op: Op, body: Vec<u8>, receipt: Option<Receipt>) -> Result<(), Code> {
        let lane = &mut self.lanes[op.lane() as usize];
        let receipt = match lane.back_mut() {
            Some(back) if op == Op::Message => match back.join(&body, receipt, self.wrap, self.bundle_max) {
                Ok(added) => {
                    self.held.grow(added);
                    None
                }
                Err(r) => Some(r),
            },
            _ => Some(receipt),
        };
        if let Some(receipt) = receipt {
            let size = body.len();
            lane.push_back(Item { op, body: Body::Bytes(body), receipt, size });
        }
        if self.queued() > QUEUE_MAX || self.held.over() {
            return Err(Code::FlowOverflow);
        }
        Ok(())
    }

    pub fn pop(&mut self) -> Option<(u32, Op, Vec<u8>)> {
        if self.inflight.len() >= self.window {
            return None;
        }
        let it = self.lanes.iter_mut().find_map(|l| l.pop_front())?;
        self.held.give(it.size);
        let body = match it.body {
            Body::Bytes(b) => b,
            Body::Live(messages) => serde_json::to_vec(&MessagesBody { messages, live: Some(true) }).unwrap_or_default(),
        };
        let pid = self.pid();
        self.inflight.insert(pid, (Instant::now(), it.receipt));
        Some((pid, it.op, body))
    }

    pub fn acked(&mut self, pid: u32) -> Option<Receipt> {
        self.inflight.remove(&pid).and_then(|(_, r)| r)
    }

    pub fn refused(&mut self, pid: u32) -> bool {
        self.inflight.remove(&pid).is_some()
    }

    fn expired(&self) -> bool {
        self.inflight.values().any(|(t, _)| t.elapsed() > T_ACK)
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
    let (tx, mut rx) = mpsc::channel::<Out>(QUEUE_MAX);
    let over = Arc::new(tokio::sync::Notify::new());
    let held = Held::new(ctx.send_queue_max_bytes);
    let conn = Conn { id, tx: tx.clone(), over: over.clone(), held: held.clone() };
    let window = ctx.policy.hub.ws_flow_window.clamp(1, 10) as usize;
    let bundle_max = ctx.chat.get().map(|c| c.limits.live_bundle_bytes).unwrap_or(0);
    let (req_tx, mut req_rx) = mpsc::channel::<(Arc<Session>, Request)>(QUEUE_MAX);
    let (chat_tx, mut chat_rx) = mpsc::channel::<(Arc<Session>, Request)>(QUEUE_MAX);
    let leave_code = Arc::new(std::sync::Mutex::new(None::<Code>));

    let lc = leave_code.clone();
    let wctx = ctx.clone();
    let writer = tokio::spawn(async move {
        let mut w = Writer::new(window, bundle_max, held);
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            let ev = tokio::select! {
                _ = over.notified() => Some(Out::Leave { code: Code::FlowOverflow, details: None }),
                o = rx.recv() => o,
                _ = tick.tick() => {
                    if w.expired() {
                        Some(Out::Leave { code: Code::FlowTimeout, details: None })
                    } else {
                        continue;
                    }
                }
            };
            let Some(o) = ev else { break };
            match o {
                Out::Reply { flags, op, pid, body } => {
                    w.held.give(body.len());
                    if sink.send(Message::Binary(frame::encode_flags(flags, op, pid, &body))).await.is_err() {
                        break;
                    }
                }
                Out::Notice { op, body, receipt } => {
                    if let Err(code) = w.push(op, body, receipt) {
                        let _ = send_leave(&mut sink, &mut w, code, None).await;
                        if let Ok(mut g) = lc.lock() {
                            g.replace(code);
                        }
                        break;
                    }
                }
                Out::Ack(pid) => {
                    if let Some(r) = w.acked(pid) {
                        acked(&wctx, r);
                    }
                }
                Out::Refused(pid) => {
                    if w.refused(pid) {
                        wctx.counters.notice_refused.fetch_add(1, Ordering::Relaxed);
                    }
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
            while let Some((pid, op, body)) = w.pop() {
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
    let cctx = ctx.clone();
    let cconn = conn.clone();
    let chat_processor = tokio::spawn(async move {
        while let Some((s, r)) = chat_rx.recv().await {
            super::handle_request(&cctx, &s, &cconn, r).await;
        }
    });

    let mut session: Option<Arc<Session>> = None;
    loop {
        let limit = if session.is_none() { T_BIND } else { Duration::from_millis(ctx.policy.hub.heartbeat_timeout_ms as u64) };
        let msg = match tokio::time::timeout(limit, stream.next()).await {
            Err(_) => {
                conn.leave(Code::HeartbeatTimeout, session.as_deref());
                break;
            }
            Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(m))) => m,
        };
        let data = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            Message::Text(_) => {
                conn.leave(Code::ProtocolError, session.as_deref());
                break;
            }
            _ => continue,
        };
        let f = match frame::decode(&data) {
            Ok(f) => f,
            Err(_) => {
                conn.leave(Code::ProtocolError, session.as_deref());
                break;
            }
        };
        let h = f.header;
        match h.kind() {
            FrameKind::Ok => {
                conn.send(Out::Ack(h.pid));
                continue;
            }
            FrameKind::Fail => {
                conn.send(Out::Refused(h.pid));
                continue;
            }
            FrameKind::Request => {}
        }
        if !f.body.is_empty() && serde_json::from_slice::<serde_json::Value>(f.body).is_err() {
            conn.reply_fail(h.op, h.pid, h.flags, &Code::ProtocolError.failure());
            conn.leave(Code::ProtocolError, session.as_deref());
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
                                conn.leave(Code::NotBound, session.as_deref());
                                early = true;
                            }
                            Some(Ok(Message::Text(_))) => {
                                super::pre_bind_other(&ctx, None);
                                conn.leave(Code::NotBound, session.as_deref());
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
            conn.leave(Code::NotBound, session.as_deref());
            break;
        };
        match op {
            Some(Op::Heartbeat) => conn.reply_ok(h.op, h.pid, h.flags, Vec::new()),
            Some(Op::Bind) => {
                super::bind_bound(&ctx, &s);
                conn.reply_fail(h.op, h.pid, h.flags, &Code::ProtocolError.failure());
                conn.leave(Code::ProtocolError, session.as_deref());
                break;
            }
            Some(Op::Leave) => {
                super::client_leave(&ctx, &s, &conn).await;
                conn.send(Out::Close);
                session = None;
                break;
            }
            Some(op) => {
                let lane = if crate::chat::is_chat(op) { &chat_tx } else { &req_tx };
                if let Err(e) = lane.try_send((s, Request { op, op_num: h.op, pid: h.pid, flags: h.flags, body: f.body.to_vec() })) {
                    if matches!(e, mpsc::error::TrySendError::Full(_)) {
                        conn.leave(Code::FlowOverflow, session.as_deref());
                    }
                    break;
                }
            }
            None => {
                let f = Code::UnknownOp.failure();
                ctx.counters.code(f.code);
                super::code_line(&f, &format!("{:#06x}", h.op), session.as_deref(), None);
                conn.reply_fail(h.op, h.pid, h.flags, &f);
            }
        }
    }
    drop(req_tx);
    drop(chat_tx);
    let _ = processor.await;
    let _ = chat_processor.await;
    let _ = tokio::time::timeout(T_BIND, conn.tx.send(Out::Close)).await;
    let _ = writer.await;
    let code = leave_code.lock().ok().and_then(|g| *g);
    if let Some(s) = session {
        super::socket_dropped(&ctx, &s, conn.id, code).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_send_queue_raises_overflow_instead_of_growing() {
        let (tx, mut rx) = mpsc::channel::<Out>(2);
        let over = Arc::new(tokio::sync::Notify::new());
        let c = Conn { id: 1, tx, over: over.clone(), held: Held::new(usize::MAX) };
        assert!(c.send(Out::Ack(1)));
        assert!(c.send(Out::Ack(2)));
        assert!(!c.send(Out::Ack(3)), "연:RULE-TRN-FLOW-CONTROL 상한을 넘으면 넣지 않는다");
        assert!(tokio::time::timeout(Duration::from_millis(50), over.notified()).await.is_ok(), "넘침 신호 — writer 가 FLOW_OVERFLOW 로 끊는다");
        assert!(matches!(rx.recv().await, Some(Out::Ack(1))));
    }
}
