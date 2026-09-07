// author: kodeholic (powered by Claude)
//! 연§6-2 방 — ROOM_JOIN · ROOM_LEAVE. 참여의 입구는 `ROOM_JOIN` 하나, 청취는 `select:false`.

use serde::{Deserialize, Serialize};

use crate::schema::{default_role, default_select, Affiliation, MemberInfo, ServerConfig, TrackEntry, Version};

/// `participant_type` — `0` 사람 / `1` 녹화 / `2` 봇. ★토큰이 정한다(연§5-2) — 요청에 자리가 없다.
pub const PARTICIPANT_USER: u8 = 0;
pub const PARTICIPANT_RECORDER: u8 = 1;
pub const PARTICIPANT_BOT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJoinReq {
    pub room_id: String,
    #[serde(default = "default_role")]
    pub role: u8,
    #[serde(default = "default_select")]
    pub select: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinRes {
    pub room_id: String,
    pub participants: Vec<MemberInfo>,
    pub affiliation: Affiliation,
    pub server_config: ServerConfig,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveRes {
    pub room_id: String,
    pub affiliation: Affiliation,
    pub version: Version,
}
