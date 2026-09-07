// author: kodeholic (powered by Claude)
//! 클라 WS 연결 하나의 상태기 — 연§3-1 끊는 조건 · §3-2 흐름 제어 · §6-1 `BIND` 전 op 거부 · §10-3 Close.
//! ★순수 함수다: 시계와 바이트를 받아 `Action` 목록을 돌려준다(정§2-3 계약 6). 소켓은 `ws.rs` 가 든다.

use std::time::{Duration, Instant};

use common::flow::OutboundQueue;
use oxsig::body::session::BindReq;
use oxsig::frame::{self, Header, Kind};
use oxsig::op::Op;
use oxsig::timers;
use oxsig::{CloseCode, FailCode, Failure};
use serde_json::Value;

use crate::backend::{fail_frame, ok_frame};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Send(Vec<u8>),
    /// 프레임 먼저, 절단 나중(정§19 집행 계약 ①).
    Close(CloseCode),
    Bind { pid: u32, req: BindReq },
    Backend { op: Op, pid: u32, body: Value },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    AwaitBind { since: Instant },
    Bound { session_id: String },
}

pub struct Conn {
    phase: Phase,
    last_activity: Instant,
    outbound: OutboundQueue,
    idle_timeout: Duration,
}

impl Conn {
    pub fn new(now: Instant, flow_window: usize, idle_timeout: Duration) -> Self {
        Self { phase: Phase::AwaitBind { since: now }, last_activity: now, outbound: OutboundQueue::new(flow_window), idle_timeout }
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.phase {
            Phase::Bound { session_id } => Some(session_id),
            Phase::AwaitBind { .. } => None,
        }
    }

    /// 클라 프레임 하나.
    pub fn on_frame(&mut self, bytes: &[u8], now: Instant) -> Vec<Action> {
        self.last_activity = now;
        let (h, body) = match frame::decode(bytes) {
            Ok(x) => x,
            Err(_) => return vec![Action::Close(CloseCode::ProtocolError)],
        };
        if h.kind.is_response() {
            // 클라 ACK(= 빈 `01`) 또는 실패 응답 — 내 통지 윈도우를 푼다.
            return self.outbound.ack(h.pid, now).into_iter().map(Action::Send).collect();
        }
        let body = match frame::body_json(body) {
            Ok(v) => v,
            Err(_) => return vec![Action::Close(CloseCode::ProtocolError)],
        };
        let Some(op) = Op::from_code(h.op) else {
            return vec![self.fail(h.op, h.pid, FailCode::UnknownOp)];
        };
        match (&self.phase, op) {
            (Phase::AwaitBind { .. }, Op::Bind) => match serde_json::from_value::<BindReq>(body) {
                Ok(req) => vec![Action::Bind { pid: h.pid, req }],
                Err(_) => vec![self.fail(h.op, h.pid, FailCode::InvalidPayload)],
            },
            (Phase::AwaitBind { .. }, _) => vec![self.fail(h.op, h.pid, FailCode::NotBound)],
            (Phase::Bound { .. }, Op::Heartbeat) => vec![Action::Send(ok_frame(Op::Heartbeat, h.pid, &Value::Null))],
            (Phase::Bound { .. }, Op::Bind) => vec![self.fail(h.op, h.pid, FailCode::InvalidPayload)],
            (Phase::Bound { .. }, op) if op.is_notification() => vec![self.fail(h.op, h.pid, FailCode::InvalidPayload)],
            (Phase::Bound { .. }, op) => vec![Action::Backend { op, pid: h.pid, body }],
        }
    }

    pub fn bound(&mut self, pid: u32, res: &oxsig::body::session::BindRes) -> Vec<Action> {
        self.phase = Phase::Bound { session_id: res.session_id.clone() };
        let body = serde_json::to_value(res).unwrap_or(Value::Null);
        vec![Action::Send(ok_frame(Op::Bind, pid, &body))]
    }

    pub fn bind_failed(&self, pid: u32, code: FailCode) -> Vec<Action> {
        vec![self.fail(Op::Bind.code(), pid, code)]
    }

    /// 뒷단 응답 — 큐를 우회해 바로 나간다(응답은 윈도우를 안 쓴다).
    pub fn on_reply(&self, wire: Vec<u8>) -> Vec<Action> {
        vec![Action::Send(wire)]
    }

    /// 서버 통지 — 우선순위 큐 + 윈도우.
    pub fn on_notify(&mut self, op: Op, body: &Value, now: Instant) -> Vec<Action> {
        let bytes = if body.is_null() { Vec::new() } else { body.to_string().into_bytes() };
        self.on_notify_raw(op.code(), &bytes, now)
    }

    /// sfud 가 만든 body 바이트 그대로(재해석 금지 — 정§19 ③).
    pub fn on_notify_raw(&mut self, op: u16, body: &[u8], now: Instant) -> Vec<Action> {
        self.outbound.enqueue(op, body, now).into_iter().map(Action::Send).collect()
    }

    /// 주기 판정 — `T-bind`(4003) · 무응답 30초(4003) · ACK 30초(4001) · 밀림 1,000(4002).
    pub fn on_tick(&self, now: Instant) -> Vec<Action> {
        if let Phase::AwaitBind { since } = self.phase
            && now.duration_since(since) >= Duration::from_millis(timers::BIND_TIMEOUT_MS)
        {
            return vec![Action::Close(CloseCode::HeartbeatTimeout)];
        }
        if now.duration_since(self.last_activity) >= self.idle_timeout {
            return vec![Action::Close(CloseCode::HeartbeatTimeout)];
        }
        if self.outbound.expired(now, Duration::from_millis(timers::RESPONSE_TIMEOUT_MS)) {
            return vec![Action::Close(CloseCode::FlowTimeout)];
        }
        if self.outbound.pending_count() > timers::QUEUE_OVERFLOW {
            return vec![Action::Close(CloseCode::FlowOverflow)];
        }
        Vec::new()
    }

    fn fail(&self, op: u16, pid: u32, code: FailCode) -> Action {
        Action::Send(fail_frame(op, pid, &Failure::new(code)))
    }
}

/// 이 연결이 만든 프레임의 종류를 읽는다(시험·로그용).
pub fn kind_of(frame_bytes: &[u8]) -> Option<(Kind, u16, u32)> {
    Header::decode(frame_bytes).ok().map(|h| (h.kind, h.op, h.pid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::body::session::BindRes;
    use oxsig::schema::PcMode;
    use serde_json::json;

    fn msg(op: u16, pid: u32, body: &Value) -> Vec<u8> {
        frame::encode_json(&Header::msg(op, pid), body)
    }
    fn conn(now: Instant) -> Conn {
        Conn::new(now, 1, Duration::from_millis(timers::IDLE_TIMEOUT_MS))
    }
    fn failure_code(a: &Action) -> Option<u16> {
        let Action::Send(f) = a else { return None };
        let (h, body) = frame::decode(f).ok()?;
        (h.kind == Kind::Fail).then(|| frame::body_json(body).ok()?["code"].as_u64()).flatten().map(|c| c as u16)
    }

    #[test]
    fn before_bind_only_bind_is_accepted() {
        let now = Instant::now();
        let mut c = conn(now);
        let a = c.on_frame(&msg(Op::RoomJoin.code(), 1, &json!({"room_id":"r"})), now);
        assert_eq!(failure_code(&a[0]), Some(2001));
        let a = c.on_frame(&msg(0x1003, 2, &Value::Null), now);
        assert_eq!(failure_code(&a[0]), Some(1001));
        let a = c.on_frame(&msg(Op::Bind.code(), 3, &json!({"token":"t","pc_mode":"1pc"})), now);
        assert!(matches!(&a[0], Action::Bind { pid: 3, req } if req.pc_mode == PcMode::OnePc && req.client_ver == 1));
        let a = c.on_frame(&msg(Op::Bind.code(), 4, &json!({"token":"t","pc_mode":"3pc"})), now);
        assert_eq!(failure_code(&a[0]), Some(1002));
    }

    #[test]
    fn bound_dispatch_heartbeat_and_ack() {
        let now = Instant::now();
        let mut c = conn(now);
        let res = BindRes { user_id: "u".into(), server_ver: 1, heartbeat_interval: 10_000, session_id: "sid".into(), resume_window_ms: 60_000, pc_mode: PcMode::TwoPc };
        let a = c.bound(7, &res);
        assert_eq!(kind_of(match &a[0] { Action::Send(f) => f, _ => panic!() }), Some((Kind::Ok, Op::Bind.code(), 7)));
        let a = c.on_frame(&msg(Op::Heartbeat.code(), 8, &Value::Null), now);
        let Action::Send(f) = &a[0] else { panic!() };
        assert_eq!(f.len(), frame::HEADER_LEN, "HEARTBEAT 응답은 빈 body");
        let a = c.on_frame(&msg(Op::PublishTracks.code(), 9, &json!({"room_id":"r"})), now);
        assert!(matches!(&a[0], Action::Backend { op: Op::PublishTracks, pid: 9, .. }));
        let a = c.on_frame(&msg(Op::TrackEvent.code(), 10, &Value::Null), now);
        assert_eq!(failure_code(&a[0]), Some(1002), "S→C 통지를 클라가 보내면 형식 오류");
        // 통지 → 윈도우 1 → 두 번째는 ACK 뒤에
        let n1 = c.on_notify(Op::ParticipantEvent, &json!({"type":"joined"}), now);
        assert_eq!(n1.len(), 1);
        assert!(c.on_notify(Op::ParticipantEvent, &json!({"type":"left"}), now).is_empty());
        let Action::Send(f1) = &n1[0] else { panic!() };
        let (_, _, pid) = kind_of(f1).unwrap();
        let ack = frame::encode(&Header { kind: Kind::Ok, reserved: 0, op: Op::ParticipantEvent.code(), pid }, &[]);
        let drained = c.on_frame(&ack, now);
        assert_eq!(drained.len(), 1);
    }

    #[test]
    fn disconnect_conditions() {
        let now = Instant::now();
        let mut c = conn(now);
        assert_eq!(c.on_frame(&[1, 3, 0, 0, 0, 0, 0, 0], now), vec![Action::Close(CloseCode::ProtocolError)]);
        assert_eq!(c.on_frame(&msg(Op::Bind.code(), 1, &Value::Null)[..5], now), vec![Action::Close(CloseCode::ProtocolError)]);
        let mut bad = msg(Op::Bind.code(), 1, &Value::Null);
        bad.extend_from_slice(b"not json");
        assert_eq!(c.on_frame(&bad, now), vec![Action::Close(CloseCode::ProtocolError)]);
        assert!(c.on_tick(now + Duration::from_secs(9)).is_empty());
        assert_eq!(c.on_tick(now + Duration::from_secs(10)), vec![Action::Close(CloseCode::HeartbeatTimeout)]);
        let res = BindRes { user_id: "u".into(), server_ver: 1, heartbeat_interval: 10_000, session_id: "sid".into(), resume_window_ms: 60_000, pc_mode: PcMode::TwoPc };
        c.bound(1, &res);
        c.on_notify(Op::RoomEvent, &json!({}), now);
        assert_eq!(c.on_tick(now + Duration::from_secs(31)), vec![Action::Close(CloseCode::HeartbeatTimeout)], "무응답이 ACK 보다 먼저 걸린다");
        c.on_frame(&msg(Op::Heartbeat.code(), 2, &Value::Null), now + Duration::from_secs(20));
        assert_eq!(c.on_tick(now + Duration::from_secs(31)), vec![Action::Close(CloseCode::FlowTimeout)]);
    }
}
