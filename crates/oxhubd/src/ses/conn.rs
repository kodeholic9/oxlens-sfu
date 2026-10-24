// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-SES-READ-AND-PROCESS · 정:IMPL-SES-SOCKET-LIFE · 정:IMPL-SES-BIND · 연:TRN.frame.WS · 연:RULE-TRN-CLOSE · 연:RULE-TRN-ONE-REPLY · 연:RULE-SES-BIND-PROC · 연:RULE-SES-HEARTBEAT · 연:RULE-SES-LEAVE-PROC · 연:RULE-SES-SOCKET-CLOSE-NORMAL · 연:RULE-ERR-CLOSE-EMPTY · model: claude-opus-5-5
//! 연결 하나 — 읽기(즉시 답하는 `HEARTBEAT` · ACK · 끊는 조건)와 처리(연결 단위 차례)와 쓰기(송신 큐 · 시한)를 가른다.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use oxsig::{Code, Frame, FrameError, Kind, Op};
use tokio::sync::mpsc;

use super::core::{self as qcore, Item, OutQueue, Verdict};
use super::{Ctx, Out, Session};

pub struct Conn {
    pub generation: u64,
    pub opened_ms: u64,
    pub last_rx: AtomicU64,
    pub left: AtomicBool,
    pub session: Mutex<Option<Arc<Session>>>,
}

impl Conn {
    pub fn session(&self) -> Option<Arc<Session>> {
        self.session.lock().ok().and_then(|g| g.clone())
    }

    pub fn bound(&self) -> bool {
        self.session().is_some()
    }
}

pub enum Job {
    Frame(Frame),
    Leave,
}

fn leave_frame(code: Code, pid: u32) -> Vec<u8> {
    Frame::new(Kind::Request, Op::Leave.as_u16(), pid, oxsig::parse::encode(&code.leave())).encode()
}

pub fn fail_bytes(f: &Frame, failure: &oxsig::Failure) -> Vec<u8> {
    f.reply(Kind::Fail, oxsig::parse::encode(failure)).encode()
}

pub fn ok_bytes(f: &Frame, body: Vec<u8>) -> Vec<u8> {
    f.reply(Kind::Ok, body).encode()
}

pub fn response(op: u16, bytes: Vec<u8>) -> Out {
    let lane = Op::from_u16(op).map(|o| o.lane().index()).unwrap_or(1);
    Out::Item(Item { lane, op, needs_ack: false, bytes })
}

pub async fn serve(ctx: Arc<Ctx>, ws: WebSocket) {
    let generation = ctx.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let now = common::time::wall_ms();
    let conn = Arc::new(Conn { generation, opened_ms: now, last_rx: AtomicU64::new(now), left: AtomicBool::new(false), session: Mutex::new(None) });
    let (sink, mut stream) = ws.split();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Out>();
    let (job_tx, job_rx) = mpsc::unbounded_channel::<Job>();
    let writer = tokio::spawn(write_loop(ctx.clone(), conn.clone(), sink, out_rx));
    let worker = tokio::spawn(work_loop(ctx.clone(), conn.clone(), job_rx, out_tx.clone()));
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { break };
        conn.last_rx.store(common::time::wall_ms(), Ordering::Relaxed);
        let raw = match msg {
            Message::Binary(b) => b,
            Message::Close(_) => break,
            Message::Text(_) => {
                let _ = out_tx.send(Out::Leave(Code::ProtocolError));
                break;
            }
            _ => continue,
        };
        conn.last_rx.store(common::time::wall_ms(), Ordering::Relaxed);
        if conn.left.load(Ordering::Relaxed) {
            continue;
        }
        let f = match Frame::decode(&raw) {
            Ok(f) => f,
            Err(e) => {
                super::count(&ctx, Code::ProtocolError);
                if e != FrameError::TooLarge
                    && let Some((op, pid)) = Frame::peek_op_pid(&raw)
                {
                    let probe = Frame { flags: 0, op, pid, body: Vec::new() };
                    let _ = out_tx.send(Out::Direct(fail_bytes(&probe, &Code::ProtocolError.failure())));
                }
                let _ = out_tx.send(Out::Leave(Code::ProtocolError));
                break;
            }
        };
        match f.kind() {
            Kind::Ok | Kind::Fail => {
                let _ = out_tx.send(Out::Ack(f.op, f.pid));
                continue;
            }
            Kind::Request => {}
        }
        if oxsig::parse::json_value(&f.body).is_err() {
            super::count(&ctx, Code::ProtocolError);
            let _ = out_tx.send(Out::Direct(fail_bytes(&f, &Code::ProtocolError.failure())));
            let _ = out_tx.send(Out::Leave(Code::ProtocolError));
            break;
        }
        match Op::from_u16(f.op) {
            Some(Op::Heartbeat) if conn.bound() => {
                let _ = out_tx.send(Out::Direct(ok_bytes(&f, Vec::new())));
            }
            Some(Op::Leave) if conn.bound() => {
                conn.left.store(true, Ordering::Relaxed);
                let _ = job_tx.send(Job::Leave);
            }
            _ => {
                let _ = job_tx.send(Job::Frame(f));
            }
        }
    }
    drop(job_tx);
    if let Some(s) = conn.session() {
        let mut st = s.lock();
        if st.socket.as_ref().map(|x| x.generation) == Some(conn.generation) {
            st.socket = None;
            if !st.gone {
                st.dropped_at = Some(common::time::wall_ms());
                tracing::info!(kind = "ev", spec = "SES.S.drop", session = %s.fp, user = %s.id.user, "socket dropped — session kept for the window");
            }
        }
    }
    drop(out_tx);
    let _ = worker.await;
    let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
}

async fn work_loop(ctx: Arc<Ctx>, conn: Arc<Conn>, mut jobs: mpsc::UnboundedReceiver<Job>, out: mpsc::UnboundedSender<Out>) {
    while let Some(job) = jobs.recv().await {
        match job {
            Job::Leave => {
                if let Some(s) = conn.session() {
                    super::destroy(&ctx, &s, common::bus::envelope::GoneCause::Leave).await;
                }
                let _ = out.send(Out::Close);
                break;
            }
            Job::Frame(f) => {
                let bytes = super::route::handle(&ctx, &conn, &f, &out).await;
                if let Some(b) = bytes {
                    let _ = out.send(response(f.op, b));
                }
            }
        }
    }
}

async fn write_loop(ctx: Arc<Ctx>, conn: Arc<Conn>, mut sink: futures_util::stream::SplitSink<WebSocket, Message>, mut rx: mpsc::UnboundedReceiver<Out>) {
    let mut q = OutQueue::new(usize::from(ctx.policy.hub.ws_flow_window));
    let mut pid: u32 = 0;
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let idle_ms = u64::from(ctx.policy.hub.heartbeat_timeout_ms);
    let mut leave: Option<Code> = None;
    'outer: loop {
        while let Some(item) = q.pop() {
            let bytes = if item.needs_ack {
                pid = pid.wrapping_add(1);
                q.sent(item.op, pid, common::time::wall_ms());
                Frame::new(Kind::Request, item.op, pid, item.bytes).encode()
            } else {
                item.bytes
            };
            if sink.send(Message::Binary(bytes)).await.is_err() {
                break 'outer;
            }
        }
        tokio::select! {
            m = rx.recv() => match m {
                None => break,
                Some(Out::Item(i)) => {
                    if q.push(i) == Verdict::Overflow {
                        leave = Some(Code::FlowOverflow);
                        break;
                    }
                }
                Some(Out::Direct(b)) => {
                    if sink.send(Message::Binary(b)).await.is_err() {
                        break;
                    }
                }
                Some(Out::Close) => break,
                Some(Out::Ack(op, p)) => {
                    q.ack(op, p);
                }
                Some(Out::Leave(code)) => {
                    leave = Some(code);
                    break;
                }
            },
            _ = tick.tick() => {
                let now = common::time::wall_ms();
                if !conn.bound() && now.saturating_sub(conn.opened_ms) >= qcore::BIND_TIMEOUT_MS {
                    leave = Some(Code::HeartbeatTimeout);
                    break;
                }
                if conn.bound() && qcore::idle_expired(conn.last_rx.load(Ordering::Relaxed), now, idle_ms) {
                    leave = Some(Code::HeartbeatTimeout);
                    break;
                }
                if q.ack_overdue(now) {
                    leave = Some(Code::FlowTimeout);
                    break;
                }
            }
        }
    }
    if let Some(code) = leave {
        super::count(&ctx, code);
        pid = pid.wrapping_add(1);
        let _ = sink.send(Message::Binary(leave_frame(code, pid))).await;
        if let Some(s) = conn.session() {
            tracing::info!(kind = "ev", spec = code.catalog(), session = %s.fp, user = %s.id.user, code = code.number(), "LEAVE sent — closing socket");
        }
    }
    q.clear();
    let _ = sink.send(Message::Close(None)).await;
    let _ = sink.close().await;
}
