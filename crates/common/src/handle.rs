// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-REQUEST · 정:RULE-DOC-PLANES · 정:IMPL-BUS-HOME-LOST · 정:IMPL-SES-RESUME · 정:IMPL-ROOM-RESYNC · model: claude-opus-5-5
//! node 안 gRPC Handle 과 node 사이 `q/node` 가 같이 나르는 봉투 — 세션 봉투(주입한 신원 · `pc_mode` · 홈) · 부름 · 답.

use std::collections::BTreeMap;

use oxsig::err::Failure;
use oxsig::room::{CreateRoomReq, Permission};
use oxsig::ses::PcMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod pb {
    tonic::include_proto!("oxlens.handle.v1");
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub session_id: String,
    pub user_id: String,
    pub pc_mode: PcMode,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub permission: Permission,
    pub home_node: String,
    pub home_inst: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum Call {
    Op { env: Envelope, op: u16, body: Value },
    SessionGone { session_id: String, user_id: String, path: String },
    Resume { env: Envelope },
    CreateRoom { req: CreateRoomReq, unused_ttl_secs: Option<u32>, departure_ttl_secs: Option<u32>, capacity: u32 },
    GetRoom { room_id: String, tracks: bool, session_id: Option<String>, user_id: Option<String> },
    ListRooms,
    Admin { method: String, path: String, query: BTreeMap<String, String>, body: Value },
    LogLevel { level: String, ttl_secs: Option<u64> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Ok { body: Value },
    Fail { failure: Failure },
    Http { status: u16, body: Value },
}

impl Reply {
    pub fn ok(body: Value) -> Reply {
        Reply::Ok { body }
    }

    pub fn fail(f: Failure) -> Reply {
        Reply::Fail { failure: f }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> Result<Reply, String> {
        serde_json::from_slice(b).map_err(|e| e.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_round_trips() {
        let env = Envelope {
            session_id: "s".into(),
            user_id: "u".into(),
            pc_mode: PcMode::Two,
            hidden: false,
            participant_type: 0,
            metadata: None,
            permission: Permission::default(),
            home_node: "n".into(),
            home_inst: "i".into(),
        };
        let c = Call::Op { env, op: oxsig::op::ROOM_JOIN, body: serde_json::json!({"room_id":"r"}) };
        assert_eq!(Call::decode(&c.encode()), Ok(c));
    }
}
