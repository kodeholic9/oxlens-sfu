// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-HOME-LOST · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-BUS-NOTICE-ENVELOPE · model: claude-opus-5-5
//! node 안 Handle 과 node 사이 `q/node/{node}/call` 이 함께 나르는 봉투 · 답 · 통지 봉투(JSON).

use oxsig::Failure;
use oxsig::msg::{PcMode, Permission};
use serde::{Deserialize, Serialize};

pub mod handle {
    tonic::include_proto!("handle");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Op,
    SessionGone,
    Resume,
    CreateRoom,
    GetRoom,
    ListRooms,
    Admin,
    LogLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Claims {
    pub participant_type: u8,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub kind: CallKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<u16>,
    #[serde(default)]
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<PcMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<Home>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<Claims>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_user: Option<String>,
}

impl Envelope {
    pub fn new(kind: CallKind) -> Envelope {
        Envelope {
            kind,
            op: None,
            body: serde_json::Value::Null,
            user_id: None,
            session: None,
            sid: None,
            pc_mode: None,
            home: None,
            claims: None,
            method: None,
            path: None,
            target_user: None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Envelope, serde_json::Error> {
        serde_json::from_slice(b)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "r", rename_all = "snake_case")]
pub enum Reply {
    Ok { body: serde_json::Value },
    Fail { failure: Failure },
    Http { status: u16, body: serde_json::Value },
}

impl Reply {
    pub fn ok(body: serde_json::Value) -> Reply {
        Reply::Ok { body }
    }

    pub fn fail(f: impl Into<Failure>) -> Reply {
        Reply::Fail { failure: f.into() }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Reply, serde_json::Error> {
        serde_json::from_slice(b)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_roundtrip() {
        let r = Reply::fail(oxsig::Code::RoomNotFound);
        assert_eq!(Reply::decode(&r.encode()).unwrap(), r);
        let e = Envelope { op: Some(0x0201), ..Envelope::new(CallKind::Op) };
        assert_eq!(Envelope::decode(&e.encode()).unwrap(), e);
    }
}
