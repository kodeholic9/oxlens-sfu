// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:SES.op.BIND · 연:SES.op.RESUME · 연:SES.op.LEAVE · 연:ROOM.op.ROOM_JOIN · 연:AFF.op.AFFILIATION · 연:AUX.op.TASK · 연:RULE-PUB-REMOVE-FORM · 연:RULE-ROOM-NOTIFY-VERSION-FOUR · 연:SES.http.TOKEN · 연:ROOM.http.GET_ROOM · model: claude-opus-5-5
//! op 마다 요청 · 응답 · 통지 몸통 — HTTP 접속점 몸통도 여기 둔다.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::code::{Code, Failure};
use crate::types::{
    Affiliation, Duplex, ExtMap, Kind, MediaCodecs, MemberInfo, MlineTable, PcMode, Permission, ProbeResult, Publication, PublishTrack,
    RoomSnapshot, RoomSummary, ServerConfig, TrackEntry, Version,
};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResumeRes {
    pub snapshot: BTreeMap<String, RoomSnapshot>,
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

impl LeaveBody {
    pub fn of(code: Code) -> LeaveBody {
        LeaveBody { code: code.0, name: code.name().to_string(), message: None, details: None }
    }

    pub fn from_failure(f: &Failure) -> LeaveBody {
        LeaveBody { code: f.code, name: f.name.clone(), message: f.message.clone(), details: f.details.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJoinReqCodecs {
    pub kind: Kind,
    pub pt: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveRes {
    pub room_id: String,
    pub affiliation: Affiliation,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl PublishTracksReq {
    pub fn room_id(&self) -> &str {
        match self {
            PublishTracksReq::Remove { room_id, .. } | PublishTracksReq::Add { room_id, .. } => room_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTracksResAddTracks {
    pub mid: String,
    pub track_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum PublishTracksRes {
    #[serde(rename = "add")]
    Add { tracks: Vec<PublishTracksResAddTracks> },
    #[serde(rename = "remove")]
    Remove {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSetRes {
    pub ssrc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noop: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyReq {
    pub room_id: String,
    pub arev: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadyRes {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub targets: Vec<SubscribeLayerReqTargets>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscribeLayerRes {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartReq {
    pub sfu_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartResPublish {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartResSubscribe {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartRes {
    pub publish: IceRestartResPublish,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<IceRestartResSubscribe>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffiliationRes {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
    pub cause: AffiliationResCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReq {
    pub room_id: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRes {
    pub msg_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkEventBodyType {
    #[serde(rename = "suspect")]
    Suspect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEventBody {
    #[serde(rename = "type")]
    pub type_: LinkEventBodyType,
    pub sfu_id: String,
    pub ufrag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineEventBody {
    pub table: MlineTable,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRes {
    pub token: String,
    pub expires_in: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRoomsRes {
    pub rooms: Vec<RoomSummary>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRoomRes {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub media_codecs: MediaCodecs,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_tracks_branches_parse() {
        let add: PublishTracksReq = serde_json::from_value(serde_json::json!({
            "room_id": "r", "tracks": [{"kind": "audio", "ssrc": 1, "mid": "0", "pt": 111, "codec": "opus"}]
        }))
        .expect("add");
        assert!(matches!(add, PublishTracksReq::Add { .. }));
        let rm: PublishTracksReq = serde_json::from_value(serde_json::json!({"action": "remove", "room_id": "r", "track_ids": ["t"]})).expect("remove");
        assert!(matches!(rm, PublishTracksReq::Remove { .. }));
        let res = serde_json::to_value(PublishTracksRes::Remove {}).expect("res");
        assert_eq!(res, serde_json::json!({"action": "remove"}));
    }

    #[test]
    fn task_phase_is_inner_tag() {
        let t: TaskReq = serde_json::from_value(serde_json::json!({"phase": "request", "req_id": 3, "type": "probe", "params": {}})).expect("task");
        assert!(matches!(t, TaskReq::Request { req_id: 3, .. }));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let r: RoomJoinReq = serde_json::from_value(serde_json::json!({"room_id": "r", "participant_type": 2, "x": 1})).expect("join");
        assert_eq!(r.room_id, "r");
    }

    #[test]
    fn leave_body_has_no_permanent() {
        let v = serde_json::to_value(LeaveBody::of(Code::HEARTBEAT_TIMEOUT)).expect("leave");
        assert_eq!(v, serde_json::json!({"code": 4008, "name": "HEARTBEAT_TIMEOUT"}));
    }
}
