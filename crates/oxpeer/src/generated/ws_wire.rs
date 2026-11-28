// ★생성 파일 — gen/ws.py 가 IR(ws) 에서 짓는다. 손으로 고치지 않는다. 배선하지 않는다(대조용).
// 원천: context/spec/protocol/areas/*/model.ts
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane { Recovery, Session, Data, Diagnostic }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op { Affiliation, Task, Message, SpaceHistory, SpaceList, SpaceView, Catchup, Messages, IceRestart, LinkEvent, MlineEvent, PublishTracks, TrackSet, RoomJoin, RoomLeave, ParticipantEvent, ParticipantState, RoomEvent, Bind, Resume, Heartbeat, Leave, Ready, SubscribeLayer, TrackEvent, TrackState }

impl Op {
    pub const ALL: &'static [Op] = &[Op::Affiliation, Op::Task, Op::Message, Op::SpaceHistory, Op::SpaceList, Op::SpaceView, Op::Catchup, Op::Messages, Op::IceRestart, Op::LinkEvent, Op::MlineEvent, Op::PublishTracks, Op::TrackSet, Op::RoomJoin, Op::RoomLeave, Op::ParticipantEvent, Op::ParticipantState, Op::RoomEvent, Op::Bind, Op::Resume, Op::Heartbeat, Op::Leave, Op::Ready, Op::SubscribeLayer, Op::TrackEvent, Op::TrackState];
    pub fn from_u16(v: u16) -> Option<Op> {
        match v { 0x0401 => Some(Op::Affiliation), 0x0601 => Some(Op::Task), 0x0501 => Some(Op::Message), 0x0502 => Some(Op::SpaceHistory), 0x0503 => Some(Op::SpaceList), 0x0504 => Some(Op::SpaceView), 0x0505 => Some(Op::Catchup), 0x0506 => Some(Op::Messages), 0x0305 => Some(Op::IceRestart), 0x0706 => Some(Op::LinkEvent), 0x0707 => Some(Op::MlineEvent), 0x0301 => Some(Op::PublishTracks), 0x0304 => Some(Op::TrackSet), 0x0201 => Some(Op::RoomJoin), 0x0202 => Some(Op::RoomLeave), 0x0701 => Some(Op::ParticipantEvent), 0x0705 => Some(Op::ParticipantState), 0x0704 => Some(Op::RoomEvent), 0x0101 => Some(Op::Bind), 0x0102 => Some(Op::Resume), 0x0103 => Some(Op::Heartbeat), 0x0104 => Some(Op::Leave), 0x0302 => Some(Op::Ready), 0x0303 => Some(Op::SubscribeLayer), 0x0702 => Some(Op::TrackEvent), 0x0703 => Some(Op::TrackState), _ => None }
    }
    pub fn as_u16(self) -> u16 {
        match self { Op::Affiliation => 0x0401, Op::Task => 0x0601, Op::Message => 0x0501, Op::SpaceHistory => 0x0502, Op::SpaceList => 0x0503, Op::SpaceView => 0x0504, Op::Catchup => 0x0505, Op::Messages => 0x0506, Op::IceRestart => 0x0305, Op::LinkEvent => 0x0706, Op::MlineEvent => 0x0707, Op::PublishTracks => 0x0301, Op::TrackSet => 0x0304, Op::RoomJoin => 0x0201, Op::RoomLeave => 0x0202, Op::ParticipantEvent => 0x0701, Op::ParticipantState => 0x0705, Op::RoomEvent => 0x0704, Op::Bind => 0x0101, Op::Resume => 0x0102, Op::Heartbeat => 0x0103, Op::Leave => 0x0104, Op::Ready => 0x0302, Op::SubscribeLayer => 0x0303, Op::TrackEvent => 0x0702, Op::TrackState => 0x0703, }
    }
    pub fn lane(self) -> Lane {
        match self { Op::Affiliation => Lane::Session, Op::Task => Lane::Diagnostic, Op::Message => Lane::Data, Op::SpaceHistory => Lane::Data, Op::SpaceList => Lane::Data, Op::SpaceView => Lane::Data, Op::Catchup => Lane::Data, Op::Messages => Lane::Data, Op::IceRestart => Lane::Session, Op::LinkEvent => Lane::Session, Op::MlineEvent => Lane::Session, Op::PublishTracks => Lane::Session, Op::TrackSet => Lane::Session, Op::RoomJoin => Lane::Session, Op::RoomLeave => Lane::Session, Op::ParticipantEvent => Lane::Session, Op::ParticipantState => Lane::Session, Op::RoomEvent => Lane::Session, Op::Bind => Lane::Session, Op::Resume => Lane::Session, Op::Heartbeat => Lane::Session, Op::Leave => Lane::Session, Op::Ready => Lane::Session, Op::SubscribeLayer => Lane::Session, Op::TrackEvent => Lane::Session, Op::TrackState => Lane::Session, }
    }
    pub fn outside_window(self) -> bool {
        matches!(self, Op::Heartbeat)
    }
}

/// 오류 코드 — (번호, 이름, permanent). 폐기 번호는 RETIRED
pub const CODES: &[(u16, &str, Option<bool>)] = &[
    (1001, "UNKNOWN_OP", Some(true)),
    (1002, "INVALID_PAYLOAD", Some(true)),
    (1003, "MISSING_FIELD", Some(true)),
    (1004, "VERSION_MISMATCH", Some(true)),
    (1005, "CODEC_REQUIRED", Some(true)),
    (1006, "CODEC_MISMATCH", Some(true)),
    (1007, "FIELD_CONFLICT", Some(true)),
    (1008, "PROTOCOL_ERROR", Some(true)),
    (2001, "NOT_BOUND", Some(false)),
    (2002, "TOKEN_INVALID", Some(true)),
    (2003, "TOKEN_EXPIRED", Some(false)),
    (2004, "INVALID_API_KEY", Some(true)),
    (2005, "CLAIM_NOT_ALLOWED", Some(true)),
    (2006, "NOT_AUTHORIZED", Some(true)),
    (2008, "SESSION_NOT_FOUND", Some(false)),
    (2009, "SESSION_REVOKED", None),
    (2010, "DUPLICATE_SESSION", None),
    (3001, "ROOM_NOT_FOUND", Some(false)),
    (3002, "NOT_IN_ROOM", Some(false)),
    (3004, "ROOM_NOT_EMPTY", Some(false)),
    (3005, "TRACK_NOT_FOUND", Some(false)),
    (3006, "TRACK_OP_UNSUPPORTED", Some(true)),
    (3008, "SSRC_COLLISION", Some(true)),
    (3009, "PRECONDITION_FAILED", Some(false)),
    (3010, "PEER_NOT_FOUND", Some(false)),
    (3011, "NOT_PUB_ROOM", Some(false)),
    (3012, "SPACE_NOT_FOUND", Some(true)),
    (3013, "NOT_SPACE_MEMBER", Some(false)),
    (3014, "CHAT_DISABLED", Some(true)),
    (3015, "MSG_ID_CONFLICT", Some(true)),
    (3016, "REF_NOT_FOUND", Some(true)),
    (3017, "REF_DELETED", Some(true)),
    (3018, "NOT_REF_SENDER", Some(true)),
    (3019, "REC_NOT_FOUND", Some(true)),
    (4001, "ROOM_FULL", Some(false)),
    (4002, "TRACK_LIMIT", Some(false)),
    (4003, "QUOTA_EXCEEDED", Some(false)),
    (4004, "LISTEN_LIMIT", Some(false)),
    (4005, "MID_LIMIT", Some(false)),
    (4006, "FLOW_TIMEOUT", None),
    (4007, "FLOW_OVERFLOW", None),
    (4008, "HEARTBEAT_TIMEOUT", None),
    (4009, "OVERLOADED", Some(false)),
    (5001, "SFU_UNAVAILABLE", Some(false)),
    (5002, "SFU_ERROR", Some(false)),
    (5003, "INTERNAL_ERROR", Some(false)),
    (5004, "SERVER_SHUTDOWN", None),
];
pub const RETIRED: &[u16] = &[2007, 3003, 3007];

/// AFF.type.Affiliation — 어느 방을 구독하고 어디로 발행하나 (§[AFF.type.Affiliation])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Affiliation {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
}

/// AUX.type.ProbeResult — `type:"probe"` 의 `result` (§[AUX.op.TASK])
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
pub enum ChatMessageKind {
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "react")]
    React,
    #[serde(rename = "delete")]
    Delete,
    #[serde(rename = "join")]
    Join,
    #[serde(rename = "leave")]
    Leave,
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
    #[serde(rename = "talk")]
    Talk,
    #[serde(rename = "talk_update")]
    TalkUpdate,
}

/// CHAT.type.ChatMessage — 메시지 한 건 (§[CHAT.op.MESSAGE])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub space_id: String,
    pub msg_id: String,
    pub user_id: String,
    pub t: u64,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ChatMessageKind>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub ref_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub talk: Option<TalkInfo>,
}

/// CHAT.type.TalkRec — 녹취 (§[CHAT.op.MESSAGE])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TalkRec {
    pub path: String,
    pub size: u64,
    pub mime: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TalkInfoState {
    #[serde(rename = "talking")]
    Talking,
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "short")]
    Short,
    #[serde(rename = "cut")]
    Cut,
    #[serde(rename = "silent")]
    Silent,
    #[serde(rename = "lost")]
    Lost,
}

/// CHAT.type.TalkInfo — 발언 상태 (§[CHAT.op.MESSAGE])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TalkInfo {
    pub state: TalkInfoState,
    pub room_id: String,
    pub start: u64,
    pub deadline: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rec: Option<TalkRec>,
}

/// CHAT.type.SpaceEntry — 내 공간 한 줄 (§[CHAT.op.SPACE_LIST])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceEntry {
    pub space_id: String,
    pub total: u64,
    pub read: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_t: Option<u64>,
}

/// ERR.type.Failure — 실패 응답 body (§[ERR.type.Failure])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub code: u16,
    pub name: String,
    pub permanent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    V1pc,
    #[serde(rename = "2pc")]
    V2pc,
}

/// ServerConfigIce — 칸 안의 묶음
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

/// ServerConfigDtls — 칸 안의 묶음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfigDtls {
    pub fingerprint: String,
    pub setup: ServerConfigDtlsSetup,
}

/// LINK.type.ServerConfig — 미디어를 붙이는 재료 (§[LINK.type.ServerConfig])
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

/// LINK.type.CodecPolicy — 코덱 정책 (§[LINK.type.ServerConfig])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecPolicy {
    pub kind: Kind,
    pub name: String,
    pub rtcp_fb: Vec<String>,
}

/// LINK.type.ExtMap — 확장 헤더 번호 (§[LINK.type.ServerConfig])
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

/// PUB.type.Publication — 발행 층 (§[SES.op])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Publication {
    pub track_id: String,
    pub mid: String,
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

/// PUB.type.PublishTrack — `tracks[]` 원소 (§[RULE-PUB-REMOVE-FORM])
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

/// ROOM.type.MediaCodecs — 그 방의 미디어 코덱 (§[RULE-ROOM-MEDIA-CODECS])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaCodecs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
}

/// ROOM.type.RoomSummary — 방 목록의 한 줄 (§[ROOM.http.LIST_ROOMS])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub user_count: u32,
    pub created_at: u64,
    pub rec: bool,
    pub record: Vec<String>,
}

/// ROOM.type.MemberInfo — `MemberInfo` (§[ROOM.type.MemberInfo])
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

/// ROOM.type.Permission — 권한 비트 (§[ROOM.type.MemberInfo])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Permission {
    pub publish_audio: bool,
    pub publish_video: bool,
    pub publish_screen: bool,
    pub floor_request: bool,
}

/// ROOM.type.RoomSnapshot — 방 스냅샷 (§[SES.op])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

/// ROOM.type.Version — 무엇이 최신인가 (§[ROOM.type.Version])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub seq: u64,
}

/// SUB.type.MlineEntry — 배정표의 줄 — 구독 m-line 하나 (§[SUB.type.MlineTable])
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

/// SUB.type.MlineTable — 구독 m-line 배정표 (§[SUB.type.MlineTable])
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

/// TRK.type.TrackEntry — 받을 트랙 하나 (§[TRK.type.TrackEntry])
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
    pub csrc: Option<u32>,
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

/// CHAT.http.REC_FILE.params — GET /rec/{space_id}/{file} 녹취 내려받기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecFileParams {
    pub space_id: String,
    pub file: String,
}

/// ROOM.http.LIST_ROOMS.res — GET /rooms 방 목록
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListRoomsRes {
    pub rooms: Vec<RoomSummary>,
    pub total: u32,
}

/// ROOM.http.CREATE_ROOM.req — POST /rooms 방 생성
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Vec<String>>,
}

/// ROOM.http.CREATE_ROOM.res — POST /rooms 방 생성
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateRoomRes {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub record: Vec<String>,
    pub media_codecs: MediaCodecs,
}

/// ROOM.http.GET_ROOM.params — GET /rooms/{room_id} 방 상세
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetRoomParams {
    pub room_id: String,
}

/// ROOM.http.GET_ROOM.query — GET /rooms/{room_id} 방 상세
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetRoomQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<u8>,
}

/// ROOM.http.GET_ROOM.res — GET /rooms/{room_id} 방 상세
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
    pub record: Vec<String>,
    pub media_codecs: MediaCodecs,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<TrackEntry>>,
}

/// SES.http.TOKEN.req — POST /auth/token 토큰 발급
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

/// SES.http.TOKEN.res — POST /auth/token 토큰 발급
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenRes {
    pub token: String,
    pub expires_in: u32,
}

/// AFF.op.AFFILIATION.req — 0x0401 어디로 발행할까
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffiliationReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select_join_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect_join_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffiliationResCause {
    #[serde(rename = "user")]
    User,
}

/// AFF.op.AFFILIATION.res — 0x0401 어디로 발행할까
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffiliationRes {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
    pub cause: AffiliationResCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskReqRequestType {
    #[serde(rename = "probe")]
    Probe,
}

/// 갈래 phase — 내부 태그
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

/// AUX.op.TASK.res — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskRes {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageReqKind {
    #[serde(rename = "edit")]
    Edit,
    #[serde(rename = "react")]
    React,
    #[serde(rename = "delete")]
    Delete,
}

/// CHAT.op.MESSAGE.req — 0x0501 메시지
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageReq {
    pub space_id: String,
    pub msg_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MessageReqKind>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub ref_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<serde_json::Value>,
}

/// CHAT.op.MESSAGE.res — 0x0501 메시지
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageRes {
    pub t: u64,
}

/// CHAT.op.SPACE_HISTORY.req — 0x0502 다시 가져오기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceHistoryReq {
    pub space_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// CHAT.op.SPACE_HISTORY.res — 0x0502 다시 가져오기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceHistoryRes {
    pub messages: Vec<ChatMessage>,
    pub relations: Vec<ChatMessage>,
    pub more: bool,
}

/// CHAT.op.SPACE_LIST.req — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpaceListReq {}

/// CHAT.op.SPACE_LIST.res — 0x0503 내 공간 목록
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceListRes {
    pub spaces: Vec<SpaceEntry>,
}

/// CHAT.op.SPACE_VIEW.req — 0x0504 읽음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceViewReq {
    pub space_id: String,
}

/// CHAT.op.SPACE_VIEW.res — 0x0504 읽음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceViewRes {
    pub read: u64,
}

/// CHAT.op.CATCHUP.req — 0x0505 따라잡기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatchupReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<u64>,
}

/// CHAT.op.CATCHUP.res — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CatchupRes {}

/// CHAT.op.MESSAGES.body — 0x0506 메시지 묶음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagesBody {
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
}

/// LINK.op.ICE_RESTART.req — 0x0305 그 서버 연결을 다시 뚫는다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartReq {
    pub sfu_id: String,
}

/// IceRestartResPublish — 칸 안의 묶음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartResPublish {
    pub ufrag: String,
    pub pwd: String,
}

/// IceRestartResSubscribe — 칸 안의 묶음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceRestartResSubscribe {
    pub ufrag: String,
    pub pwd: String,
}

/// LINK.op.ICE_RESTART.res — 0x0305 그 서버 연결을 다시 뚫는다
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

/// LINK.op.LINK_EVENT.body — 0x0706 그 서버 연결이 흔들린다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkEventBody {
    #[serde(rename = "type")]
    pub type_: LinkEventBodyType,
    pub sfu_id: String,
    pub ufrag: String,
}

/// LINK.op.MLINE_EVENT.body — 0x0707 구독 m-line 배정표가 **바뀌었다**
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

/// 갈래 action — 태그 없음(선언 순서대로 시도 — 판별 칸에 기본값이 있다)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PublishTracksReq {
    #[serde(rename = "add")]
    Add {
    #[serde(default, skip_serializing_if = "Option::is_none")]
            action: Option<PublishTracksReqAddAction>,
        join_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
            dd_extmap_id: Option<u8>,
    },
    #[serde(rename = "remove")]
    Remove {
        action: PublishTracksReqRemoveAction,
        room_id: String,
        join_id: String,
        track_ids: Vec<String>,
    },
}

/// PublishTracksResAddTracks — 칸 안의 묶음
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublishTracksResAddTracks {
    pub mid: String,
    pub track_id: String,
}

/// 갈래 action — 내부 태그
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum PublishTracksRes {
    #[serde(rename = "add")]
    Add {
        tracks: Vec<PublishTracksResAddTracks>,
    },
    #[serde(rename = "remove")]
    Remove {

    },
}

/// PUB.op.TRACK_SET.req — 0x0304 트랙 상태 바꾸기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSetReq {
    pub room_id: String,
    pub join_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssrc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
}

/// PUB.op.TRACK_SET.res — 0x0304 트랙 상태 바꾸기
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

/// RoomJoinReqCodecs — 칸 안의 묶음
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

/// ROOM.op.ROOM_JOIN.req — 0x0201 방 입장
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_id: Option<String>,
}

/// ROOM.op.ROOM_JOIN.res — 0x0201 방 입장
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomJoinRes {
    pub room_id: String,
    pub join_id: String,
    pub participants: Vec<MemberInfo>,
    pub me: MemberInfo,
    pub affiliation: Affiliation,
    pub server_config: ServerConfig,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
    pub media_codecs: MediaCodecs,
    pub mline_table: MlineTable,
}

/// ROOM.op.ROOM_LEAVE.req — 0x0202 방 퇴장
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_id: Option<String>,
}

/// ROOM.op.ROOM_LEAVE.res — 0x0202 방 퇴장
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

/// ROOM.op.PARTICIPANT_EVENT.body — 0x0701 누가 들고났다
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

/// ROOM.op.PARTICIPANT_STATE.body — 0x0705 참가자 **속성**이 바뀌었다
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

/// 갈래 type — 내부 태그
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
    SyncRequired {
        room_id: String,
        reason: String,
    },
}

/// SES.op.BIND.req — 0x0101 접속한다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ver: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<PcMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tag: Option<bool>,
}

/// SES.op.BIND.res — 0x0101 접속한다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindRes {
    pub user_id: String,
    pub server_ver: u32,
    pub heartbeat_interval: u64,
    pub session_id: String,
    pub resume_window_ms: u64,
    pub pc_mode: PcMode,
    pub source_tag: bool,
}

/// SES.op.RESUME.req — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResumeReq {}

/// SES.op.RESUME.res — 0x0102 시그널만 끊겼다 다시 붙었다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeRes {
    pub snapshot: std::collections::BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
    pub mline_tables: Vec<MlineTable>,
}

/// SES.op.HEARTBEAT.req — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeartbeatReq {}

/// SES.op.HEARTBEAT.res — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeartbeatRes {}

/// SES.op.LEAVE.req — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LeaveReq {}

/// SES.op.LEAVE.body — 0x0104 끝낸다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaveBody {
    pub code: u16,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// SUB.op.READY.req — 0x0302 준비 됐다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyReq {
    pub room_id: String,
    pub join_id: String,
    pub arev: u64,
}

/// SUB.op.READY.res — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadyRes {}

/// SubscribeLayerReqTargets — 칸 안의 묶음
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

/// SUB.op.SUBSCRIBE_LAYER.req — 0x0303 받을 레이어 고르기
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub join_id: String,
    pub targets: Vec<SubscribeLayerReqTargets>,
}

/// SUB.op.SUBSCRIBE_LAYER.res — 빈 body
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscribeLayerRes {}

/// TRK.op.TRACK_EVENT.body — 0x0702 받을 트랙이 **생기거나 없어졌다**
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

/// TRK.op.TRACK_STATE.body — 0x0703 트랙 **속성**이 바뀌었다
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
