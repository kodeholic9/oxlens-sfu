// author: kodeholic (powered by Claude)
//! 시그널링 — WS 소켓 · pid · 윈도우 · 대기표 · ACK(SDK:IMPL-MOD-TABLE internal/signaling · 연:TRN.frame.WS · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-REQ-LIFE).
//! 소켓의 읽기 · 쓰기만 태스크고, 대기표는 SDK 루프가 쥔다(루프 하나 — SDK:RULE-API-ASYNC). 응답은 요청에 실어 둔 쓰임새(`Purpose`)로 루프가 직접 가른다 — 콜백을 주입하지 않는다.
//! ★받은 통지는 ACK 을 먼저 보내고 루프에 넘긴다 · LEAVE 통지는 ACK 하지 않고 끊김 사유로 들고 있는다.
//! ★대기표는 세션에 묶인다 — 끊겨도 기한 전이면 들고 있다가 같은 세션이 이어지면 처음 차례 · 처음 pid 로 다시 보내고, 새 세션이면 버린다.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_rustls::rustls::ClientConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::generated::ws_wire::{CODES, Lane, Op};
use crate::wire::frame::{self, Frame, Kind};

/// 소켓 사건 — sgen = 몇째 소켓(옛 소켓의 사건은 루프가 버린다)
#[derive(Debug)]
pub(crate) enum SockIn {
    Open(u64),
    Frame(u64, Frame),
    Closed(u64),
}

struct Socket {
    out: mpsc::UnboundedSender<Message>,
    tasks: Vec<AbortHandle>,
}

impl Socket {
    /// 닫기를 보내고 놓는다 — 쓰기 태스크가 그것을 내보내고 스스로 끝난다(떼어 낼 때 끊지 않는다)
    fn release(mut self) {
        let _ = self.out.send(Message::Close(None));
        self.tasks.clear();
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// WS 하나 — `ws://` 는 TCP · `wss://` 는 TLS 위(context 202611/20261108d E2)
async fn open_ws(url: &str, auth: &str, tls: Option<&Arc<ClientConfig>>) -> Option<tokio_tungstenite::WebSocketStream<crate::tls::Stream>> {
    let s = crate::tls::dial(auth, tls).await.ok()?;
    tokio_tungstenite::client_async(url, s).await.ok().map(|(ws, _)| ws)
}

fn spawn_socket(url: String, auth: String, tls: Option<Arc<ClientConfig>>, sgen: u64, tx: mpsc::UnboundedSender<SockIn>) -> Socket {
    let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let h = tokio::spawn(async move {
        let Some(ws) = open_ws(&url, &auth, tls.as_ref()).await else {
            let _ = tx.send(SockIn::Closed(sgen));
            return;
        };
        let _ = tx.send(SockIn::Open(sgen));
        let (mut sink, mut stream) = ws.split();
        loop {
            tokio::select! {
                m = out_rx.recv() => match m {
                    Some(Message::Close(c)) => {
                        let _ = sink.send(Message::Close(c)).await;
                        break;
                    }
                    Some(m) => {
                        if sink.send(m).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                m = stream.next() => match m {
                    Some(Ok(Message::Binary(b))) => match frame::decode(&b) {
                        Ok(f) => {
                            let _ = tx.send(SockIn::Frame(sgen, f));
                        }
                        Err(_) => break,
                    },
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
            }
        }
        let _ = tx.send(SockIn::Closed(sgen));
    });
    Socket { out, tasks: vec![h.abort_handle()] }
}

/// 요청의 쓰임새 — 응답이 오면 루프가 이것으로 받는 곳을 고른다
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Purpose {
    Bind,
    Resume,
    Heartbeat,
    RoomJoin { room: String, epoch: u64, jid: i64 },
    RoomLeave { room: String, epoch: u64 },
    Ready { room: String, epoch: u64 },
    Layer { room: String, epoch: u64, id: u64 },
    IceRestart { server: String, req: i64 },
    Publish { track: u64, pid: i64 },
    Unpublish,
    TrackSet { track: u64 },
    Affiliation { server: Option<String> },
    Message { id: u64 },
}

/// 요청의 끝 — 응답(성공 · 실패) · 소켓이 끝나 버림(소켓 op) · 세션이 끝나 버림 · 기한 초과
#[derive(Debug)]
pub(crate) enum Outcome {
    Ok(Value),
    Fail(Value),
    Transport,
    Closed,
    Expired,
}

struct Pending {
    op: Op,
    body: Value,
    purpose: Purpose,
    pid: u32,
    sent_at: Option<Instant>,
}

/// 소켓에 묶인 op — 소켓이 끝나면 버린다(연:RULE-TRN-REQ-LIFE 2)
fn socket_op(op: Op) -> bool {
    matches!(op, Op::Bind | Op::Resume | Op::Heartbeat)
}

fn lane_rank(l: Lane) -> u8 {
    match l {
        Lane::Recovery => 0,
        Lane::Session => 1,
        Lane::Data => 2,
        Lane::Diagnostic => 3,
    }
}

/// 받은 프레임 하나의 결과
#[derive(Debug, PartialEq)]
pub(crate) enum Got {
    /// 아는 op 의 통지 — ACK 은 이미 나갔다
    Notice(Op, Value),
    /// 모르는 op 의 통지 — `1001` 실패로 이미 답했고 버린다
    Unknown(u16),
    /// 위로 올릴 것 없음(응답 · LEAVE · HEARTBEAT · 다른 소켓 세대)
    Nothing,
}

pub(crate) struct Sig {
    url: String,
    auth: String,
    tls: Option<Arc<ClientConfig>>,
    sock: Option<Socket>,
    sgen: u64,
    pid: u32,
    window: usize,
    req_timeout: Duration,
    waiting: VecDeque<Pending>,
    inflight: HashMap<u32, Pending>,
    held: Vec<Pending>,
    up: bool,
    ended: bool,
    open: bool,
    leave: Option<Value>,
    last_rx: Option<Instant>,
}

impl Sig {
    pub(crate) fn new(url: String, auth: String, tls: Option<Arc<ClientConfig>>, req_timeout: Duration) -> Sig {
        Sig { url, auth, tls, sock: None, sgen: 0, pid: 0, window: 1, req_timeout, waiting: VecDeque::new(), inflight: HashMap::new(), held: Vec::new(), up: false, ended: false, open: false, leave: None, last_rx: None }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.sgen
    }

    pub(crate) fn last_rx(&self) -> Option<Instant> {
        self.last_rx
    }

    /// 새 소켓을 연다 — 옛 소켓은 떼고 소켓에 묶인 요청은 버린다 · 대기표는 남긴다
    pub(crate) fn connect(&mut self, tx: &mpsc::UnboundedSender<SockIn>, ended: &mut Vec<(Purpose, Outcome)>) {
        if let Some(s) = self.sock.take() {
            s.release();
        }
        self.detach(ended);
        self.sgen += 1;
        self.ended = false;
        self.leave = None;
        self.sock = Some(spawn_socket(self.url.clone(), self.auth.clone(), self.tls.clone(), self.sgen, tx.clone()));
    }

    /// LEAVE 없이 소켓을 닫는다(사유 없는 끊김)
    pub(crate) fn close(&mut self, ended: &mut Vec<(Purpose, Outcome)>) {
        if let Some(s) = self.sock.take() {
            s.release();
        }
        self.detach(ended);
    }

    /// 세션 끝 — LEAVE 는 큐를 거치지 않고 곧바로 · 남은 대기표는 closed
    pub(crate) fn send_leave(&mut self, ended: &mut Vec<(Purpose, Outcome)>) {
        let pid = self.next_pid();
        if let Some(s) = self.sock.take() {
            let _ = s.out.send(Message::Binary(frame::encode(Kind::Msg, Op::Leave.as_u16(), pid, &Value::Null)));
            s.release();
        }
        self.detach(ended);
        self.session_end(ended);
    }

    pub(crate) fn opened(&mut self, sgen: u64) -> bool {
        if sgen != self.sgen || self.sock.is_none() {
            return false;
        }
        self.open = true;
        self.last_rx = Some(Instant::now());
        true
    }

    /// 소켓이 닫혔다 — 이 소켓의 것이면 들고 있던 LEAVE 사유를 돌려준다
    pub(crate) fn closed(&mut self, sgen: u64, ended: &mut Vec<(Purpose, Outcome)>) -> Option<Option<Value>> {
        if sgen != self.sgen || self.sock.is_none() {
            return None;
        }
        let leave = self.leave.take();
        self.detach(ended);
        Some(leave)
    }

    /// 요청 — 끝은 루프가 `Outcome` 으로 받는다
    pub(crate) fn request(&mut self, op: Op, body: Value, purpose: Purpose, ended: &mut Vec<(Purpose, Outcome)>) {
        if self.ended {
            ended.push((purpose, Outcome::Closed));
            return;
        }
        if socket_op(op) && !self.open {
            ended.push((purpose, Outcome::Transport));
            return;
        }
        let p = Pending { op, body, purpose, pid: 0, sent_at: None };
        if op.outside_window() {
            self.transmit(p, ended);
            return;
        }
        self.waiting.push_back(p);
        self.pump(ended);
    }

    /// 세션이 섰다 — same = 같은 세션이 이어졌다(들고 있던 요청을 처음 차례 · 처음 pid 로) · 아니면 옛 세션 요청을 버린다
    pub(crate) fn session_up(&mut self, same: bool, ended: &mut Vec<(Purpose, Outcome)>) {
        let mut held = std::mem::take(&mut self.held);
        held.sort_by_key(|p| p.pid);
        if !same {
            for p in held {
                ended.push((p.purpose, Outcome::Transport));
            }
            for p in self.waiting.drain(..) {
                ended.push((p.purpose, Outcome::Transport));
            }
        } else {
            for p in held {
                self.transmit(p, ended);
            }
        }
        self.up = true;
        self.pump(ended);
    }

    /// 세션이 끝났다 — 대기표 전부 closed · 다음 connect() 전까지 새 요청도 closed
    pub(crate) fn session_end(&mut self, ended: &mut Vec<(Purpose, Outcome)>) {
        self.ended = true;
        for p in self.held.drain(..) {
            ended.push((p.purpose, Outcome::Closed));
        }
        for p in self.waiting.drain(..) {
            ended.push((p.purpose, Outcome::Closed));
        }
        for (_, p) in self.inflight.drain() {
            ended.push((p.purpose, Outcome::Closed));
        }
        self.up = false;
    }

    /// 쓰임새가 맞는 요청을 버린다(그 방의 요청 버림 — 효과 ws.abandon) — 응답이 와도 짝이 없어 버려진다
    pub(crate) fn forget(&mut self, pred: impl Fn(&Purpose) -> bool) {
        self.waiting.retain(|p| !pred(&p.purpose));
        self.held.retain(|p| !pred(&p.purpose));
        self.inflight.retain(|_, p| !pred(&p.purpose));
    }

    fn next_pid(&mut self) -> u32 {
        self.pid = self.pid.wrapping_add(1);
        self.pid
    }

    fn pump(&mut self, ended: &mut Vec<(Purpose, Outcome)>) {
        if !self.open {
            return;
        }
        while self.inflight.values().filter(|p| !p.op.outside_window()).count() < self.window {
            let best = self.waiting.iter().enumerate().filter(|(_, w)| self.up || socket_op(w.op)).min_by_key(|(_, w)| lane_rank(w.op.lane())).map(|(i, _)| i);
            let Some(i) = best else { return };
            let Some(p) = self.waiting.remove(i) else { return };
            self.transmit(p, ended);
        }
    }

    fn transmit(&mut self, mut p: Pending, ended: &mut Vec<(Purpose, Outcome)>) {
        if self.sock.is_none() || !self.open {
            if socket_op(p.op) {
                ended.push((p.purpose, Outcome::Transport));
            } else if p.pid != 0 {
                self.held.push(p);
            } else {
                self.waiting.push_front(p);
            }
            return;
        }
        if p.pid == 0 {
            p.pid = self.next_pid();
            p.sent_at = Some(Instant::now());
        }
        let bytes = frame::encode(Kind::Msg, p.op.as_u16(), p.pid, &p.body);
        if let Some(sock) = &self.sock {
            let _ = sock.out.send(Message::Binary(bytes));
        }
        self.inflight.insert(p.pid, p);
    }

    fn ack(&self, op: u16, pid: u32) {
        if let Some(s) = &self.sock {
            let _ = s.out.send(Message::Binary(frame::encode(Kind::Ok, op, pid, &Value::Null)));
        }
    }

    /// 모르는 통지 op 의 실패 응답 — `1001 UNKNOWN_OP`(연:RULE-TRN-NOTIFY-ACK) · 아는 op 의 body 를 못 읽은 것은 이 길이 아니라 ACK
    fn refuse_unknown(&self, op: u16, pid: u32) {
        let (code, name, permanent) = CODES.iter().copied().find(|c| c.1 == "UNKNOWN_OP").unwrap_or((1001, "UNKNOWN_OP", Some(true)));
        let body = serde_json::json!({ "code": code, "name": name, "permanent": permanent.unwrap_or(true) });
        if let Some(s) = &self.sock {
            let _ = s.out.send(Message::Binary(frame::encode(Kind::Fail, op, pid, &body)));
        }
    }

    /// 받은 프레임 — 통지면 `Notice(op, body)` · 모르는 op 의 통지면 `Unknown(op)`(실패로 답했다 — 부르는 쪽이 WARN) · 응답이면 그 요청의 끝을 ended 에
    pub(crate) fn receive(&mut self, sgen: u64, f: Frame, ended: &mut Vec<(Purpose, Outcome)>) -> Got {
        if sgen != self.sgen {
            return Got::Nothing;
        }
        self.last_rx = Some(Instant::now());
        let op = Op::from_u16(f.op);
        if f.kind == Kind::Msg {
            let Some(op) = op else {
                self.refuse_unknown(f.op, f.pid);
                return Got::Unknown(f.op);
            };
            if op == Op::Leave {
                self.leave = Some(f.body);
                return Got::Nothing;
            }
            self.ack(f.op, f.pid);
            if op == Op::Heartbeat {
                return Got::Nothing;
            }
            return Got::Notice(op, f.body);
        }
        let Some(p) = self.inflight.remove(&f.pid) else { return Got::Nothing };
        if Some(p.op) != op {
            self.inflight.insert(p.pid, p);
            return Got::Nothing;
        }
        ended.push((p.purpose, if f.kind == Kind::Ok { Outcome::Ok(f.body) } else { Outcome::Fail(f.body) }));
        self.pump(ended);
        Got::Nothing
    }

    /// 기한(T-req) — 넘긴 요청만 버리고 소켓을 닫는다 · 들고 있는 요청도 끊긴 동안 센다 · 닫아야 하면 true
    pub(crate) fn check_timeouts(&mut self, ended: &mut Vec<(Purpose, Outcome)>) -> bool {
        let now = Instant::now();
        let over = |p: &Pending| p.sent_at.is_some_and(|t| now.duration_since(t) > self.req_timeout);
        let mut cut = false;
        let late: Vec<u32> = self.inflight.values().filter(|p| over(p)).map(|p| p.pid).collect();
        for pid in late {
            cut = true;
            if let Some(p) = self.inflight.get(&pid)
                && socket_op(p.op) {
                    continue;
                }
            if let Some(p) = self.inflight.remove(&pid) {
                ended.push((p.purpose, Outcome::Expired));
            }
        }
        let (gone, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.held).into_iter().partition(|p| over(p));
        self.held = keep;
        for p in gone {
            ended.push((p.purpose, Outcome::Expired));
        }
        cut
    }

    /// 기한 시계를 돌릴 일이 있나(보낸 것 · 들고 있는 것)
    pub(crate) fn counting(&self) -> bool {
        !self.inflight.is_empty() || !self.held.is_empty()
    }

    /// 소켓을 뗀다 — 소켓에 묶인 요청은 버리고 보낸 요청은 대기표에 남긴다
    fn detach(&mut self, ended: &mut Vec<(Purpose, Outcome)>) {
        self.sock = None;
        self.open = false;
        self.up = false;
        for (_, p) in self.inflight.drain() {
            if socket_op(p.op) {
                ended.push((p.purpose, Outcome::Transport));
            } else {
                self.held.push(p);
            }
        }
        let (gone, keep): (VecDeque<Pending>, VecDeque<Pending>) = std::mem::take(&mut self.waiting).into_iter().partition(|p| socket_op(p.op));
        self.waiting = keep;
        for p in gone {
            ended.push((p.purpose, Outcome::Transport));
        }
    }
}

#[cfg(test)]
mod notify_ack_test {
    use super::*;

    fn sig_with_socket() -> (Sig, mpsc::UnboundedReceiver<Message>) {
        let mut s = Sig::new(String::new(), String::new(), None, Duration::from_secs(10));
        let (out, rx) = mpsc::unbounded_channel::<Message>();
        s.sock = Some(Socket { out, tasks: Vec::new() });
        (s, rx)
    }

    fn sent(rx: &mut mpsc::UnboundedReceiver<Message>) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        while let Ok(Message::Binary(b)) = rx.try_recv() {
            v.push(b.to_vec());
        }
        v
    }

    #[test]
    fn unknown_notify_op_is_refused_with_1001_and_dropped() {
        let (mut s, mut rx) = sig_with_socket();
        let mut ended = Vec::new();
        let got = s.receive(0, Frame { kind: Kind::Msg, op: 0x7f7f, pid: 88, reserved: 0, body: serde_json::json!({ "x": 1 }) }, &mut ended);
        let out = sent(&mut rx);
        assert_eq!(out.len(), 1, "C-proc-RULE-TRN-NOTIFY-ACK-STEPS① 모르는 통지 op 에 응답 하나");
        let f = frame::decode(&out[0]).expect("실패 응답 프레임");
        assert_eq!(
            (f.kind, f.op, f.pid, f.body["code"].as_u64(), f.body["name"].as_str(), f.body["permanent"].as_bool()),
            (Kind::Fail, 0x7f7f, 88, Some(1001), Some("UNKNOWN_OP"), Some(true)),
            "C-proc-RULE-TRN-NOTIFY-ACK-STEPS① op · pid 받은 그대로 · flags 10 · Failure 1001 UNKNOWN_OP"
        );
        assert!(got == Got::Unknown(0x7f7f) && ended.is_empty(), "C-proc-RULE-TRN-NOTIFY-ACK-STEPS② 모르는 op 는 처리 0 - 위로는 WARN 할 op 번호만 올린다");
    }

    #[test]
    fn a_known_notify_op_with_an_unreadable_body_is_acked() {
        let (mut s, mut rx) = sig_with_socket();
        let mut ended = Vec::new();
        let got = s.receive(0, Frame { kind: Kind::Msg, op: Op::TrackState.as_u16(), pid: 89, reserved: 0, body: serde_json::json!({ "room_id": 7, "nonsense": [1, 2] }) }, &mut ended);
        let out = sent(&mut rx);
        let f = frame::decode(&out[0]).expect("ACK 프레임");
        assert_eq!((out.len(), f.kind, f.pid, out[0].len()), (1, Kind::Ok, 89, 8), "C-proc-RULE-TRN-NOTIFY-ACK-STEPS① 아는 op 는 body 를 못 읽어도 ACK - 실패로 답하는 것은 op 번호를 모를 때뿐");
        assert!(matches!(got, Got::Notice(Op::TrackState, _)), "stand check - 아는 op 는 위로 올라간다(읽기 실패는 받는 영역 몫)");
    }
}
