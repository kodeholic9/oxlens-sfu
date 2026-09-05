// author: kodeholic (powered by Claude)
//! 연§6-5 MESSAGE · §6-6 TASK.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `0x0501` 보낼 때. 보낸 사람에게는 에코하지 않는다 — 자기 것은 `pid` 로 안다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSend {
    pub room_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSendRes {
    pub msg_id: String,
}

/// `0x0501` 받을 때 — `user_id` 는 서버가 세션에서 넣는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRecv {
    pub room_id: String,
    pub user_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPhase {
    Request,
    Report,
}

/// `0x0601 TASK` — `report` 는 응답이 아니라 새 `00` 프레임. 시킨 것과 보고를 잇는 것은 `req_id` 뿐.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub phase: TaskPhase,
    pub req_id: u32,
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

pub const TASK_PROBE: &str = "probe";

impl Task {
    pub fn probe_request(req_id: u32) -> Self {
        Self { phase: TaskPhase::Request, req_id, task_type: TASK_PROBE.into(), params: Some(Value::Object(Default::default())), result: None }
    }
    pub fn report(&self, result: Value) -> Self {
        Self { phase: TaskPhase::Report, req_id: self.req_id, task_type: self.task_type.clone(), params: None, result: Some(result) }
    }
    /// 연§6-6 — 모르는 `type` 은 조용히 버리지 않고 `1002` 로 답한다.
    pub fn is_known_type(&self) -> bool {
        self.task_type == TASK_PROBE
    }
}
