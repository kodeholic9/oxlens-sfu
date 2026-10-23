// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:ROOM.type.MemberInfo · 연:ROOM.type.Version · 연:ROOM.type.MediaCodecs · 연:ROOM.op.ROOM_JOIN · 연:ROOM.op.ROOM_LEAVE · 연:ROOM.op.PARTICIPANT_EVENT · 연:ROOM.op.PARTICIPANT_STATE · 연:ROOM.op.ROOM_EVENT · 연:ROOM.http.LIST_ROOMS · 연:ROOM.http.CREATE_ROOM · 연:ROOM.http.GET_ROOM · 연:ROOM.type.RoomSummary · 연:RULE-ROOM-PERMISSION-AXIS · model: claude-opus-5-5
//! 방 — 명단 · 판번호 · 입장 · 퇴장 · 방 통지 넷 · HTTP 셋의 형.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aff::Affiliation;
use crate::link::{ExtMap, ServerConfig};
use crate::sub::MlineTable;
use crate::trk::{MediaKind, TrackEntry};

pub const ROLE_DEFAULT: u8 = 255;
pub const CAPACITY_MAX: u32 = 1000;
pub const ROOM_ID_MAX_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    #[serde(default = "yes")]
    pub publish_audio: bool,
    #[serde(default = "yes")]
    pub publish_video: bool,
    #[serde(default = "yes")]
    pub publish_screen: bool,
    #[serde(default = "yes")]
    pub floor_request: bool,
}

fn yes() -> bool {
    true
}

impl Default for Permission {
    fn default() -> Self {
        Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true }
    }
}

impl Permission {
    pub fn is_default(&self) -> bool {
        self.publish_audio && self.publish_video && self.publish_screen && self.floor_request
    }

    pub fn denied_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.publish_audio {
            out.push("publish_audio");
        }
        if !self.publish_video {
            out.push("publish_video");
        }
        if !self.publish_screen {
            out.push("publish_screen");
        }
        if !self.floor_request {
            out.push("floor_request");
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: String,
    #[serde(default = "role_default")]
    pub role: u8,
    pub select: bool,
    pub participant_type: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

fn role_default() -> u8 {
    ROLE_DEFAULT
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaCodecs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
}

impl MediaCodecs {
    pub const AUDIO_DEFAULT: &'static str = "opus";
    pub const VIDEO_DEFAULT: &'static str = "H264";

    pub fn filled(&self) -> MediaCodecs {
        let pick = |v: &Option<String>, d: &str| match v {
            Some(s) if !s.is_empty() => s.clone(),
            _ => d.to_string(),
        };
        MediaCodecs {
            audio: Some(pick(&self.audio, Self::AUDIO_DEFAULT)),
            video: Some(pick(&self.video, Self::VIDEO_DEFAULT)),
        }
    }

    pub fn audio(&self) -> &str {
        self.audio.as_deref().unwrap_or(Self::AUDIO_DEFAULT)
    }

    pub fn video(&self) -> &str {
        self.video.as_deref().unwrap_or(Self::VIDEO_DEFAULT)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JoinCodec {
    pub kind: MediaKind,
    pub pt: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinReq {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extmap: Option<Vec<ExtMap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs: Option<Vec<JoinCodec>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinRes {
    pub room_id: String,
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub affiliation: Affiliation,
    pub server_config: ServerConfig,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
    pub media_codecs: MediaCodecs,
    pub mline_table: MlineTable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomLeaveRes {
    pub room_id: String,
    pub affiliation: Affiliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantEventType {
    Joined,
    Left,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParticipantEvent {
    #[serde(rename = "type")]
    pub kind: ParticipantEventType,
    pub room_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
    pub version: Version,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParticipantState {
    #[serde(rename = "type")]
    pub kind: String,
    pub user_id: String,
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    pub permission: Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffiliationCause {
    Kick,
    RoomClosed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomEvent {
    Affiliation { room_id: String, affiliation: Affiliation, cause: AffiliationCause },
    SyncRequired { room_id: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub user_count: u32,
    pub created_at: u64,
    pub rec: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListRoomsRes {
    pub rooms: Vec<RoomSummary>,
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct CreateRoomReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unused_ttl_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_ttl_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_codecs: Option<MediaCodecs>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateRoomRes {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub media_codecs: MediaCodecs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GetRoomRes {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub user_count: u32,
    pub participants: Vec<MemberInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub me: Option<MemberInfo>,
    pub created_at: u64,
    pub media_codecs: MediaCodecs,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<TrackEntry>>,
}

pub fn room_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ROOM_ID_MAX_BYTES
        && !id.chars().any(|c| matches!(c, '/' | '*' | '$' | '?' | '#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_id_forbids_bus_key_characters() {
        assert!(room_id_ok("r1"));
        for bad in ["", "a/b", "a*", "a$", "a?", "a#"] {
            assert!(!room_id_ok(bad), "{bad}");
        }
        assert!(!room_id_ok(&"x".repeat(256)));
    }

    #[test]
    fn room_event_tags() {
        let e = RoomEvent::SyncRequired { room_id: "r".into(), reason: "no_media_flow".into() };
        let v = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(v["type"], "sync_required");
        let k = RoomEvent::Affiliation {
            room_id: "r".into(),
            affiliation: Affiliation { sub_rooms: vec![], pub_room: None },
            cause: AffiliationCause::RoomClosed,
        };
        let v = serde_json::to_value(&k).unwrap_or_default();
        assert_eq!(v["cause"], "room_closed");
        assert!(v["affiliation"]["pub_room"].is_null());
    }
}
