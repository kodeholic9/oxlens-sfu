// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:RULE-BUS-REQUEST 정:RULE-BUS-EVENTS 정:IMPL-BUS-HOME-LOST 정:IMPL-SES-HANDOVER 정:CAND-BUS-HANDLE-SHAPE 정:CAND-BUS-NOTICE-ENVELOPE · model: claude-opus-5-5
//! B 평면 봉투 — node 안 gRPC `Call` 과 node 사이 `q/node/{node}/call` 이 같은 형을 나른다 · `ev` put 의 값.

use serde::{Deserialize, Serialize};

use crate::code::{Code, Failure};
use crate::msg::{CreateRoomReq, PcMode, Permission};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ctx {
    pub user_id: String,
    pub session: String,
    pub pc_mode: PcMode,
    pub home: Home,
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "admin", rename_all = "snake_case")]
pub enum AdminCall {
    Rooms,
    Drops,
    RoomSnapshot { room_id: String },
    SfuRooms,
    UserMedia { user_id: String },
    Links { users: Vec<String> },
    Reap { room_id: String, user_id: String },
    Destroy { room_id: String, if_version: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Op { ctx: Ctx, op: u16, body: String },
    SessionGone { user_id: String, session: String },
    Resume { ctx: Ctx },
    CreateRoom { room_id: String, req: CreateRoomReq },
    GetRoom { room_id: String, tracks: bool, viewer: Option<String> },
    ListRooms,
    Admin { req: AdminCall },
    LogLevel { level: String, ttl_secs: Option<u32> },
}

impl Call {
    pub fn kind(&self) -> &'static str {
        match self {
            Call::Op { .. } => "op",
            Call::SessionGone { .. } => "session_gone",
            Call::Resume { .. } => "resume",
            Call::CreateRoom { .. } => "create_room",
            Call::GetRoom { .. } => "get_room",
            Call::ListRooms => "list_rooms",
            Call::Admin { .. } => "admin",
            Call::LogLevel { .. } => "log_level",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Ok { body: String },
    Fail { failure: Failure },
    Http { status: u16, body: String },
}

impl Reply {
    pub fn ok<T: Serialize>(v: &T) -> Reply {
        match serde_json::to_string(v) {
            Ok(body) => Reply::Ok { body },
            Err(e) => Reply::Fail { failure: Code::InternalError.failure_msg(e.to_string()) },
        }
    }

    pub fn empty() -> Reply {
        Reply::Ok { body: String::new() }
    }

    pub fn fail(code: Code) -> Reply {
        Reply::Fail { failure: code.failure() }
    }

    pub fn failure(f: Failure) -> Reply {
        Reply::Fail { failure: f }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self)
            .unwrap_or_else(|_| b"{\"reply\":\"fail\",\"failure\":{\"code\":5003,\"name\":\"INTERNAL_ERROR\",\"permanent\":false}}".to_vec())
    }

    pub fn decode(b: &[u8]) -> Reply {
        serde_json::from_slice(b).unwrap_or_else(|e| Reply::Fail { failure: Code::SfuError.failure_msg(e.to_string()) })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub op: u16,
    pub body: String,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_round_trip() {
        let c = Call::Op {
            ctx: Ctx {
                user_id: "u1".into(),
                session: "0a1b2c3d".into(),
                pc_mode: PcMode::V2pc,
                home: Home { node: "node-a".into(), inst: "h1".into() },
                participant_type: 0,
                hidden: false,
                metadata: None,
                permission: None,
            },
            op: 0x0201,
            body: "{\"room_id\":\"r1\"}".into(),
        };
        let s = serde_json::to_string(&c).expect("ser");
        let d: Call = serde_json::from_str(&s).expect("de");
        assert_eq!(c, d);
        assert_eq!(d.kind(), "op");
    }

    #[test]
    fn reply_decode_failure_is_sfu_error() {
        match Reply::decode(b"garbage") {
            Reply::Fail { failure } => assert_eq!(failure.code, 5002),
            other => panic!("{other:?}"),
        }
    }
}
