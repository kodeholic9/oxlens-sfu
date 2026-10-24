// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-NOTICE-ENVELOPE · 정:IMPL-BUS-HOME-LOST · 정:IMPL-SES-HANDOVER · model: claude-opus-5-5
//! 봉투 — node 안(gRPC `Call`)과 node 사이(`q/node/{node}/call`)가 같은 형을 나른다 · 통지(`ev/*` put) 봉투.

use oxsig::{CreateRoomReq, Failure, GetRoomRes, MlineTable, PcMode, Publication, RoomSnapshot, RoomSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Home {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoneCause {
    Leave,
    Window,
    Cut,
    Evict,
    Shutdown,
}

impl GoneCause {
    pub fn as_str(self) -> &'static str {
        match self {
            GoneCause::Leave => "leave",
            GoneCause::Window => "window",
            GoneCause::Cut => "cut",
            GoneCause::Evict => "evict",
            GoneCause::Shutdown => "shutdown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Op {
        op: u16,
        body: serde_json::Value,
        user_id: String,
        session: String,
        pc_mode: PcMode,
        home: Home,
        participant_type: u8,
        hidden: bool,
        #[serde(default)]
        metadata: Option<serde_json::Value>,
        #[serde(default)]
        permission: Option<oxsig::Permission>,
    },
    SessionGone { user_id: String, session: String, cause: GoneCause },
    Resume { user_id: String, session: String, home: Home },
    CreateRoom { req: CreateRoomReq, room_id: String, unused_ttl_secs: u32, departure_ttl_secs: u32, capacity: u32 },
    GetRoom { room_id: String, tracks: bool, user_id: Option<String> },
    ListRooms,
    Admin { what: Admin },
    LogLevel { level: String, ttl_secs: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "admin", rename_all = "snake_case")]
pub enum Admin {
    Rooms,
    RoomSnapshot { room_id: String },
    UserMedia { user_id: String },
    Users,
    Drops,
    Destroy { room_id: String, if_version: Option<String> },
    Reap { room_id: String, user_id: String },
    SetPermission { room_id: String, user_id: String, permission: oxsig::Permission },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Ok(serde_json::Value),
    Fail(Failure),
}

impl Reply {
    pub fn ok<T: Serialize>(v: &T) -> Reply {
        Reply::Ok(serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
    }

    pub fn fail(code: oxsig::Code) -> Reply {
        Reply::Fail(code.failure())
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Reply {
        serde_json::from_slice(b).unwrap_or_else(|e| Reply::Fail(oxsig::Code::InternalError.failure_with(format!("reply: {e}"), None)))
    }
}

impl Call {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Call, String> {
        serde_json::from_slice(b).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumePart {
    pub rooms: BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
    pub mline_table: Option<MlineTable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomList {
    pub rooms: Vec<RoomSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomDetail(pub GetRoomRes);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub op: u16,
    pub body: serde_json::Value,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub sfu_id: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
}

impl Notice {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Option<Notice> {
        serde_json::from_slice(b).ok()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "q", rename_all = "snake_case")]
pub enum NodeQuery {
    Call { call: Call, unit: Option<String> },
    Session { sid: String },
    Evict { user_id: String, keep_sid: String },
    Cut { user_id: String },
    Bus,
    Rooms,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_round_trip() {
        let c = Call::SessionGone { user_id: "u".into(), session: "ab".into(), cause: GoneCause::Cut };
        assert_eq!(Call::decode(&c.encode()), Ok(c));
        let r = Reply::fail(oxsig::Code::RoomNotFound);
        assert_eq!(Reply::decode(&r.encode()), r);
        let n = Notice { op: 0x0701, body: serde_json::json!({"a":1}), exclude: vec!["u".into()], room_id: Some("r".into()), sfu_id: None, target: None };
        assert_eq!(Notice::decode(&n.encode()), Some(n));
    }
}
