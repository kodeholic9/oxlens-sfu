// author: kodeholic (powered by Claude)
//! 방 귀속 op 의 뒷단 — 정§15-2 라우팅. hub 는 envelope 에 세션 신원을 주입해 넘기고 응답 wire 를 그대로 통과시킨다.

use async_trait::async_trait;
use oxsig::frame::{Header, Kind, encode_json};
use oxsig::op::Op;
use oxsig::{FailCode, Failure};
use serde_json::Value;

/// envelope — `user_id` 는 세션에서 주입한다(body 의 것을 믿지 않는다).
#[derive(Debug, Clone)]
pub struct Envelope {
    pub session_id: String,
    pub user_id: String,
    pub op: Op,
    pub pid: u32,
    pub body: Value,
}

#[async_trait]
pub trait Backend: Send + Sync {
    /// 응답 wire 하나(성공 또는 실패 프레임). 정§19 ②: 요청 하나에 응답은 정확히 하나.
    async fn handle(&self, env: Envelope) -> Vec<u8>;
}

pub fn ok_frame(op: Op, pid: u32, body: &Value) -> Vec<u8> {
    encode_json(&Header { kind: Kind::Ok, reserved: 0, op: op.code(), pid }, body)
}

pub fn fail_frame(op: u16, pid: u32, failure: &Failure) -> Vec<u8> {
    let body = serde_json::to_value(failure).unwrap_or(Value::Null);
    encode_json(&Header { kind: Kind::Fail, reserved: 0, op, pid }, &body)
}

/// sfud 가 아직 없는 배치 — 방 귀속 op 전부 `5001`.
pub struct NoBackend;

#[async_trait]
impl Backend for NoBackend {
    async fn handle(&self, env: Envelope) -> Vec<u8> {
        fail_frame(env.op.code(), env.pid, &Failure::new(FailCode::SfuUnavailable).message("no media server attached"))
    }
}
