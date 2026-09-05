// author: kodeholic (powered by Claude)
//! 연§6-1 세션 — BIND · RESUME · HEARTBEAT(빈 body).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::{MediaKind, MemberInfo, PcMode, TrackEntry, Version};

/// 이 규격서의 세대.
pub const CLIENT_VER: u32 = 1;

/// `0x0101 BIND` 요청 — 토큰은 body 로. `session_id` 가 유효하면 그것이 이긴다(인증 두 갈래).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindReq {
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default = "default_client_ver")]
    pub client_ver: u32,
    #[serde(default = "default_pc_mode")]
    pub pc_mode: PcMode,
}

pub fn default_client_ver() -> u32 {
    CLIENT_VER
}
pub fn default_pc_mode() -> PcMode {
    PcMode::TwoPc
}

/// `BIND` 응답 — `session_id` 가 보낸 것과 같으면 세션이 살아 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindRes {
    pub user_id: String,
    pub role: String,
    pub server_ver: u32,
    pub heartbeat_interval: u64,
    pub session_id: String,
    pub resume_window_ms: u64,
    pub pc_mode: PcMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumePublish {
    pub track_id: String,
    pub kind: MediaKind,
}

/// `0x0102 RESUME` 요청 — 살아 있는 것만 신고한다. `session_id` 필드는 없다(`BIND` 가 특정했다).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResumeReq {
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub publish: Vec<ResumePublish>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

/// `RESUME` 응답 — 방 단위 판정, `resumed` 방마다 `snapshot` 필수, 발언권은 없다.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ResumeRes {
    #[serde(default)]
    pub resumed: Vec<String>,
    #[serde(default)]
    pub failed: Vec<String>,
    /// 로그·사람용 — 판단은 `failed` 소속으로.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reason: BTreeMap<String, String>,
    #[serde(default)]
    pub publish_failed: Vec<String>,
    #[serde(default)]
    pub snapshot: BTreeMap<String, RoomSnapshot>,
}
