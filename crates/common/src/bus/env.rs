// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:CAND-BUS-HANDLE-SHAPE · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:CAND-BUS-NOTICE-ENVELOPE · 정:IMPL-BUS-HOME-LOST · 운영:LOG.log.LINE · model: claude-opus-5-5
//! 봉투 — `Call`(hub → sfud) · `Reply` · `Notice`(sfud 가 `ev` 로 put). node 안 gRPC 와 node 사이 `q/node/{node}/call` 이 같은 형을 나른다.

use std::collections::BTreeMap;

use oxsig::code::{Code, Failure};
use oxsig::types::{PcMode, Permission};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionCtx {
    pub user_id: String,
    pub session: String,
    pub pc_mode: PcMode,
    pub home_node: String,
    pub home_inst: String,
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Op {
        ctx: SessionCtx,
        op: u16,
        body: serde_json::Value,
    },
    SessionGone {
        user_id: String,
        session: String,
        code: u16,
    },
    Resume {
        ctx: SessionCtx,
    },
    RoomCreate {
        room_id: String,
        name: String,
        capacity: u32,
        unused_ttl_secs: u32,
        departure_ttl_secs: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_codecs: Option<oxsig::types::MediaCodecs>,
    },
    RoomGet {
        room_id: String,
        tracks: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        me: Option<String>,
    },
    RoomList,
    Admin {
        method: String,
        path: String,
        #[serde(default)]
        query: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<serde_json::Value>,
    },
    LogLevel {
        level: String,
        ttl_secs: u64,
    },
}

impl Call {
    pub fn name(&self) -> &'static str {
        match self {
            Call::Op { .. } => "op",
            Call::SessionGone { .. } => "session_gone",
            Call::Resume { .. } => "resume",
            Call::RoomCreate { .. } => "room_create",
            Call::RoomGet { .. } => "room_get",
            Call::RoomList => "room_list",
            Call::Admin { .. } => "admin",
            Call::LogLevel { .. } => "log_level",
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Call, String> {
        serde_json::from_slice(b).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Ok { body: serde_json::Value },
    Fail { failure: Failure },
    Http { status: u16, body: serde_json::Value },
}

impl Reply {
    pub fn ok<T: Serialize>(body: &T) -> Reply {
        Reply::Ok { body: serde_json::to_value(body).unwrap_or(serde_json::Value::Null) }
    }

    pub fn fail(code: Code) -> Reply {
        Reply::Fail { failure: Failure::of(code) }
    }

    pub fn failure(f: Failure) -> Reply {
        Reply::Fail { failure: f }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Reply, String> {
        serde_json::from_slice(b).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub op: u16,
    pub body: serde_json::Value,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
}

impl Notice {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Notice, String> {
        serde_json::from_slice(b).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_and_reply_round_trip() {
        let c = Call::RoomGet { room_id: "r".into(), tracks: true, me: None };
        assert_eq!(Call::decode(&c.encode()).expect("call"), c);
        let r = Reply::fail(Code::ROOM_NOT_FOUND);
        assert_eq!(Reply::decode(&r.encode()).expect("reply"), r);
        let v: serde_json::Value = serde_json::from_slice(&Call::RoomList.encode()).expect("json");
        assert_eq!(v, serde_json::json!({"call": "room_list"}));
    }

    #[test]
    fn notice_shape() {
        let n = Notice { op: 0x0701, body: serde_json::json!({}), exclude: vec!["u1".into()], room_id: Some("r".into()), sfu_id: None };
        let v: serde_json::Value = serde_json::from_slice(&n.encode()).expect("json");
        assert_eq!(v["exclude"][0], "u1");
        assert!(v.get("sfu_id").is_none());
    }
}
