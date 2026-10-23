// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:AUX.op.MESSAGE · 연:AUX.op.TASK · 연:AUX.type.ProbeResult · 연:RULE-AUX-TASK-PROC · model: claude-opus-5-5
//! 부가 통신 — 문자 중계 · 시켜서 하고 보고하기.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReq {
    pub room_id: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRes {
    pub msg_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageBody {
    pub room_id: String,
    pub user_id: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRequest {
    pub phase: String,
    pub req_id: u32,
    #[serde(rename = "type")]
    pub kind_of: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskReport {
    pub phase: String,
    pub req_id: u32,
    #[serde(rename = "type")]
    pub kind_of: String,
    pub result: Value,
}

pub const TASK_TYPES: [&str; 1] = ["probe"];
