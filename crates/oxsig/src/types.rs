// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:AFF.type.Affiliation · 연:LINK.type.ServerConfig · 연:ROOM.type.MemberInfo · 연:ROOM.type.Version · 연:SUB.type.MlineTable · 연:TRK.type.TrackEntry · 연:RULE-SUB-TABLE-ONE-SOURCE · 연:RULE-ROOM-PARTICIPANT-TYPE · model: claude-opus-5-5
//! 이름 붙은 형 — 여러 op 이 함께 싣는 묶음.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "audio")]
    Audio,
    #[serde(rename = "video")]
    Video,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Audio => "audio",
            Kind::Video => "video",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Duplex {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "half")]
    Half,
}

impl Duplex {
    pub fn as_str(self) -> &'static str {
        match self {
            Duplex::Full => "full",
            Duplex::Half => "half",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    V1pc,
    #[serde(rename = "2pc")]
    V2pc,
}

impl PcMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PcMode::V1pc => "1pc",
            PcMode::V2pc => "2pc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "camera")]
    Camera,
    #[serde(rename = "screen")]
    Screen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Affiliation {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProbeResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_tracks: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_tracks: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfigIce {
    pub ip: String,
    pub port: u16,
    pub publish_ufrag: String,
    pub publish_pwd: String,
    pub subscribe_ufrag: String,
    pub subscribe_pwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerConfigDtlsSetup {
    #[serde(rename = "passive")]
    Passive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfigDtls {
    pub fingerprint: String,
    pub setup: ServerConfigDtlsSetup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub sfu_id: String,
    pub pc_mode: PcMode,
    pub ice: ServerConfigIce,
    pub dtls: ServerConfigDtls,
    pub codecs: Vec<CodecPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs_sub: Option<Vec<CodecPolicy>>,
    pub extmap: Vec<ExtMap>,
    pub max_bitrate_bps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecPolicy {
    pub kind: Kind,
    pub name: String,
    pub rtcp_fb: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExtMap {
    pub id: u8,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    pub track_id: String,
    pub room_id: String,
    pub kind: Kind,
    pub duplex: Duplex,
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    pub simulcast: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTrack {
    pub kind: Kind,
    pub ssrc: u32,
    pub mid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    pub pt: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_ssrc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulcast: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaCodecs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub user_count: u32,
    pub created_at: u64,
    pub rec: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    pub select: bool,
    pub participant_type: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Permission {
    pub publish_audio: bool,
    pub publish_video: bool,
    pub publish_screen: bool,
    pub floor_request: bool,
}

impl Default for Permission {
    fn default() -> Permission {
        Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineEntry {
    pub mid: String,
    pub kind: Kind,
    pub pt: u8,
    pub codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub ssrc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_ssrc: Option<u32>,
    pub stream_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineTable {
    pub sfu_id: String,
    pub arev: u64,
    pub mlines: Vec<MlineEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackEntryType {
    #[serde(rename = "individual")]
    Individual,
    #[serde(rename = "slot")]
    Slot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackEntry {
    #[serde(rename = "type")]
    pub type_: TrackEntryType,
    pub room_id: String,
    pub track_id: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulcast: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalability: Option<String>,
}

pub type SnapshotMap = BTreeMap<String, RoomSnapshot>;
