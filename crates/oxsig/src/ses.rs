// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:SES.op.BIND · 연:SES.op.RESUME · 연:SES.op.HEARTBEAT · 연:SES.op.LEAVE · 연:SES.http.TOKEN · 연:RULE-SES-PC-MODE-SESSION · 연:RULE-SES-TOKEN-CLAIMS · model: claude-opus-5-5
//! 세션 — 접속 · 이어받기 · 하트비트 · 끝냄 · 토큰 발급.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::r#pub::Publication;
use crate::room::{Permission, RoomSnapshot};
use crate::sub::MlineTable;

pub const CLIENT_VER: u32 = 1;
pub const SERVER_VER: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    One,
    #[serde(rename = "2pc")]
    Two,
}

impl PcMode {
    pub fn parse(s: &str) -> Option<PcMode> {
        match s {
            "1pc" => Some(PcMode::One),
            "2pc" => Some(PcMode::Two),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PcMode::One => "1pc",
            PcMode::Two => "2pc",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindReq {
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ver: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindRes {
    pub user_id: String,
    pub server_ver: u32,
    pub heartbeat_interval: u64,
    pub session_id: String,
    pub resume_window_ms: u64,
    pub pc_mode: PcMode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ResumeRes {
    pub snapshot: BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
    pub mline_tables: Vec<MlineTable>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenReq {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenRes {
    pub token: String,
    pub expires_in: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: u64,
    pub exp: u64,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
    pub iat: u64,
    pub exp: u64,
}
