// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:SES.op.BIND 연:SES.op.RESUME 연:SES.op.LEAVE 연:ROOM.op.ROOM_JOIN 연:ROOM.op.ROOM_LEAVE 연:ROOM.type.MemberInfo 연:ROOM.type.Version 연:ROOM.type.RoomSnapshot 연:PUB.op.PUBLISH_TRACKS 연:PUB.op.TRACK_SET 연:SUB.op.READY 연:SUB.op.SUBSCRIBE_LAYER 연:SUB.type.MlineTable 연:SUB.type.MlineEntry 연:TRK.type.TrackEntry 연:TRK.op.TRACK_EVENT 연:TRK.op.TRACK_STATE 연:AFF.op.AFFILIATION 연:AUX.op.MESSAGE 연:AUX.op.TASK 연:LINK.op.ICE_RESTART 연:LINK.op.LINK_EVENT 연:LINK.op.MLINE_EVENT 연:LINK.type.ServerConfig 연:ROOM.http.CREATE_ROOM 연:ROOM.http.GET_ROOM 연:ROOM.http.LIST_ROOMS 연:SES.http.TOKEN 연:RULE-TRN-UNKNOWN-JSON-FIELD · model: claude-opus-5-5
//! WS 메시지 · HTTP 접속점의 형 — 연동규격서 원천 모형(IR ws)의 칸 그대로.

use serde::{Deserialize, Serialize};

pub use crate::code::Failure;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Affiliation {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    V1pc,
    #[serde(rename = "2pc")]
    V2pc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfigDtls {
    pub fingerprint: String,
    pub setup: ServerConfigDtlsSetup,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "audio")]
    Audio,
    #[serde(rename = "video")]
    Video,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecPolicy {
    pub kind: Kind,
    pub name: String,
    pub rtcp_fb: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtMap {
    pub id: u8,
    pub uri: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duplex {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "half")]
    Half,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[serde(rename = "camera")]
    Camera,
    #[serde(rename = "screen")]
    Screen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaCodecs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Permission {
    pub publish_audio: bool,
    pub publish_video: bool,
    pub publish_screen: bool,
    pub floor_request: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MlineTable {
    pub sfu_id: String,
    pub arev: u64,
    pub mlines: Vec<MlineEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackEntryType {
    #[serde(rename = "individual")]
    Individual,
    #[serde(rename = "slot")]
    Slot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListRoomsRes {
    pub rooms: Vec<RoomSummary>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateRoomReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unused_ttl_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departure_ttl_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_codecs: Option<MediaCodecs>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateRoomRes {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub media_codecs: MediaCodecs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetRoomParams {
    pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetRoomQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenReq {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenRes {
    pub token: String,
    pub expires_in: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffiliationReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffiliationResCause {
    #[serde(rename = "user")]
    User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffiliationRes {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
    pub cause: AffiliationResCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageReq {
    pub room_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageRes {
    pub msg_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageBody {
    pub room_id: String,
    pub user_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskReqRequestType {
    #[serde(rename = "probe")]
    Probe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase")]
pub enum TaskReq {
    #[serde(rename = "request")]
    Request {
        req_id: u32,
        #[serde(rename = "type")]
        type_: TaskReqRequestType,
        params: serde_json::Value,
    },
    #[serde(rename = "report")]
    Report {
        req_id: u32,
        #[serde(rename = "type")]
        type_: TaskReqRequestType,
        result: ProbeResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskRes {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartReq {
    pub sfu_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartResPublish {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartResSubscribe {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartRes {
    pub publish: IceRestartResPublish,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<IceRestartResSubscribe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkEventBodyType {
    #[serde(rename = "suspect")]
    Suspect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEventBody {
    #[serde(rename = "type")]
    pub type_: LinkEventBodyType,
    pub sfu_id: String,
    pub ufrag: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MlineEventBody {
    pub table: MlineTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishTracksReqAddAction {
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishTracksReqRemoveAction {
    #[serde(rename = "remove")]
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PublishTracksReq {
    Remove {
        action: PublishTracksReqRemoveAction,
        room_id: String,
        track_ids: Vec<String>,
    },
    Add {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<PublishTracksReqAddAction>,
        room_id: String,
        tracks: Vec<PublishTrack>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        twcc_extmap_id: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rid_extmap_id: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repair_rid_extmap_id: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mid_extmap_id: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio_level_extmap_id: Option<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishTracksResAddTracks {
    pub mid: String,
    pub track_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum PublishTracksRes {
    #[serde(rename = "add")]
    Add {
        tracks: Vec<PublishTracksResAddTracks>,
    },
    #[serde(rename = "remove")]
    Remove {},
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSetReq {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssrc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSetRes {
    pub ssrc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noop: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinReqCodecs {
    pub kind: Kind,
    pub pt: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinReq {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extmap: Option<Vec<ExtMap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs: Option<Vec<RoomJoinReqCodecs>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomLeaveRes {
    pub room_id: String,
    pub affiliation: Affiliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantEventBodyType {
    #[serde(rename = "joined")]
    Joined,
    #[serde(rename = "left")]
    Left,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticipantEventBody {
    #[serde(rename = "type")]
    pub type_: ParticipantEventBodyType,
    pub room_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantStateBodyType {
    #[serde(rename = "permission")]
    Permission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticipantStateBody {
    #[serde(rename = "type")]
    pub type_: ParticipantStateBodyType,
    pub user_id: String,
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    pub permission: Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomEventBodyAffiliationCause {
    #[serde(rename = "kick")]
    Kick,
    #[serde(rename = "room_closed")]
    RoomClosed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RoomEventBody {
    #[serde(rename = "affiliation")]
    Affiliation {
        room_id: String,
        affiliation: Affiliation,
        cause: RoomEventBodyAffiliationCause,
    },
    #[serde(rename = "sync_required")]
    SyncRequired { room_id: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindReq {
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ver: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<PcMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindRes {
    pub user_id: String,
    pub server_ver: u32,
    pub heartbeat_interval: u64,
    pub session_id: String,
    pub resume_window_ms: u64,
    pub pc_mode: PcMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResumeReq {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeRes {
    pub snapshot: std::collections::BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
    pub mline_tables: Vec<MlineTable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeartbeatReq {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeartbeatRes {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LeaveReq {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaveBody {
    pub code: u16,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyReq {
    pub room_id: String,
    pub arev: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadyRes {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeLayerReqTargets {
    pub track_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub targets: Vec<SubscribeLayerReqTargets>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscribeLayerRes {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackEventBody {
    pub action: PublishTracksReqAddAction,
    pub room_id: String,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackStateBodyType {
    #[serde(rename = "muted")]
    Muted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackStateBody {
    #[serde(rename = "type")]
    pub type_: TrackStateBodyType,
    pub user_id: String,
    pub track_id: String,
    pub kind: Kind,
    pub room_id: String,
    pub version: Version,
    pub muted: bool,
}
