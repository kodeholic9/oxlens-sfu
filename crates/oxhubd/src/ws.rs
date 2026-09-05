// author: kodeholic (powered by Claude)
//! WS 어댑터 — 소켓과 `Conn` 상태기 사이. 업그레이드는 무인증(연§5-0 — 인증은 `BIND`), binary 프레임만.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use dashmap::DashMap;
use futures_util::StreamExt;
use oxsig::op::Op;
use oxsig::{CloseCode, FailCode, Failure};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::backend::{Backend, Envelope, fail_frame};
use crate::conn::{Action, Conn};
use crate::session::SessionRegistry;

/// 다른 연결이 이 연결에 보내는 것 — 통지(op·body) 또는 절단 지시.
pub struct ConnHandle {
    pub notify: mpsc::Sender<(u16, Vec<u8>)>,
    pub close: mpsc::Sender<CloseCode>,
}

pub struct Hub {
    pub registry: Arc<SessionRegistry>,
    pub backend: Arc<dyn Backend>,
    pub conns: DashMap<u64, ConnHandle>,
    pub flow_window: usize,
    pub idle_timeout: Duration,
    conn_seq: AtomicU64,
}

impl Hub {
    pub fn new(registry: Arc<SessionRegistry>, backend: Arc<dyn Backend>, flow_window: usize, idle_timeout: Duration) -> Self {
        Self { registry, backend, conns: DashMap::new(), flow_window, idle_timeout, conn_seq: AtomicU64::new(1) }
    }

    /// 연§7-0-2 짝 — 연결에 통지 하나(wire body 그대로). 살아 있지 않으면 `false`(RESUME 스냅샷이 대신한다).
    pub fn notify_raw(&self, conn_id: u64, op: u16, body: Vec<u8>) -> bool {
        self.conns.get(&conn_id).is_some_and(|h| h.notify.try_send((op, body)).is_ok())
    }

    /// 사용자에게 통지 — 붙어 있는 연결이 없으면 `false`.
    pub fn notify_user(&self, user_id: &str, op: u16, body: Vec<u8>) -> bool {
        self.registry.conn_of_user(user_id).is_some_and(|c| self.notify_raw(c, op, body))
    }

    pub fn notify_user_json(&self, user_id: &str, op: Op, body: &Value) -> bool {
        self.notify_user(user_id, op.code(), body.to_string().into_bytes())
    }
}

pub async fn upgrade(State(hub): State<Arc<Hub>>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(socket, hub))
}

async fn send_close(socket: &mut WebSocket, code: CloseCode) {
    let _ = socket.send(Message::Close(Some(CloseFrame { code: code.code(), reason: code.reason().into() }))).await;
}

async fn run(mut socket: WebSocket, hub: Arc<Hub>) {
    let conn_id = hub.conn_seq.fetch_add(1, Ordering::Relaxed);
    let (notify_tx, mut notify_rx) = mpsc::channel::<(u16, Vec<u8>)>(oxsig::timers::QUEUE_OVERFLOW + 1);
    let (close_tx, mut close_rx) = mpsc::channel::<CloseCode>(1);
    hub.conns.insert(conn_id, ConnHandle { notify: notify_tx, close: close_tx });
    let mut conn = Conn::new(Instant::now(), hub.flow_window, hub.idle_timeout);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    info!(conn_id, "ws open");

    'main: loop {
        let actions: Vec<Action> = tokio::select! {
            m = socket.next() => match m {
                Some(Ok(Message::Binary(b))) => conn.on_frame(&b, Instant::now()),
                Some(Ok(Message::Text(_))) => vec![Action::Close(CloseCode::ProtocolError)],
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => Vec::new(),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break 'main,
            },
            Some((op, body)) = notify_rx.recv() => conn.on_notify_raw(op, &body, Instant::now()),
            Some(code) = close_rx.recv() => vec![Action::Close(code)],
            _ = tick.tick() => conn.on_tick(Instant::now()),
        };
        for a in actions {
            match a {
                Action::Send(f) => {
                    if socket.send(Message::Binary(f)).await.is_err() {
                        break 'main;
                    }
                }
                Action::Close(code) => {
                    warn!(conn_id, code = code.code(), reason = code.reason(), "ws close");
                    send_close(&mut socket, code).await;
                    break 'main;
                }
                Action::Bind { pid, req } => {
                    let out = match hub.registry.bind(&req, conn_id, Instant::now()) {
                        Ok(o) => o,
                        Err(code) => {
                            for a in conn.bind_failed(pid, code) {
                                if let Action::Send(f) = a && socket.send(Message::Binary(f)).await.is_err() {
                                    break 'main;
                                }
                            }
                            continue;
                        }
                    };
                    if let Some((old, code)) = out.close_old
                        && let Some(h) = hub.conns.get(&old)
                    {
                        let _ = h.close.try_send(code);
                    }
                    info!(conn_id, session = %out.session_id, user = %out.res.user_id, resumed = out.resumed, "bind");
                    for a in conn.bound(pid, &out.res) {
                        if let Action::Send(f) = a && socket.send(Message::Binary(f)).await.is_err() {
                            break 'main;
                        }
                    }
                }
                Action::Backend { op, pid, body } => {
                    let Some(sid) = conn.session_id().map(str::to_owned) else { continue };
                    let wire = match hub.registry.get(&sid) {
                        Some(s) if op != Op::Resume || hub.registry.take_resume(&sid).is_ok() => {
                            let pc_mode = serde_json::to_value(s.pc_mode).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
                            hub.backend.handle(Envelope { session_id: sid, user_id: s.user_id, pc_mode, floor_priority: u32::from(s.floor_priority), op, pid, body }).await
                        }
                        Some(_) => fail_frame(op.code(), pid, &Failure::new(FailCode::SessionNotFound)),
                        None => fail_frame(op.code(), pid, &Failure::new(FailCode::SessionNotFound)),
                    };
                    for a in conn.on_reply(wire) {
                        if let Action::Send(f) = a && socket.send(Message::Binary(f)).await.is_err() {
                            break 'main;
                        }
                    }
                }
            }
        }
    }
    hub.conns.remove(&conn_id);
    if let Some(sid) = conn.session_id() {
        hub.registry.detach(sid, conn_id, Instant::now());
    }
    info!(conn_id, "ws closed");
}
