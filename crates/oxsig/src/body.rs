// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:SES.op.BIND · 연:SES.op.RESUME · 연:SES.op.LEAVE · 연:ROOM.op.ROOM_JOIN · 연:ROOM.op.ROOM_LEAVE · 연:ROOM.op.PARTICIPANT_EVENT · 연:ROOM.op.PARTICIPANT_STATE · 연:ROOM.op.ROOM_EVENT · 연:PUB.op.PUBLISH_TRACKS · 연:PUB.op.TRACK_SET · 연:SUB.op.READY · 연:SUB.op.SUBSCRIBE_LAYER · 연:LINK.op.ICE_RESTART · 연:LINK.op.LINK_EVENT · 연:LINK.op.MLINE_EVENT · 연:AFF.op.AFFILIATION · 연:AUX.op.MESSAGE · 연:AUX.op.TASK · 연:TRK.op.TRACK_EVENT · 연:TRK.op.TRACK_STATE · 연:RULE-PUB-REMOVE-FORM · 연:RULE-PUB-PUBLISH-RES · 연:RULE-AFF-RES-FORM · 연:RULE-AFF-CAUSE-USER · model: claude-opus-5-5
//! op 몸통 — 요청 · 응답 · 통지.

use serde::{Deserialize, Serialize};

use crate::types::{Affiliation, Duplex, ExtMap, Kind, MediaCodecs, MemberInfo, MlineTable, PcMode, Permission, ProbeResult, Publication, PublishTrack, RoomSnapshot, ServerConfig, TrackEntry, Version};

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
    Remove {
        action: PublishTracksReqRemoveAction,
        room_id: String,
        track_ids: Vec<String>,
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
    Add { tracks: Vec<PublishTracksResAddTracks> },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_tracks_untagged_takes_both_shapes() {
        let add: PublishTracksReq = serde_json::from_str(r#"{"room_id":"r1","tracks":[{"kind":"audio","ssrc":1,"mid":"0","pt":111}]}"#).expect("add");
        assert!(matches!(add, PublishTracksReq::Add { .. }));
        let rm: PublishTracksReq = serde_json::from_str(r#"{"action":"remove","room_id":"r1","track_ids":["t"]}"#).expect("remove");
        assert!(matches!(rm, PublishTracksReq::Remove { .. }));
        let res = serde_json::to_string(&PublishTracksRes::Remove {}).expect("ser");
        assert_eq!(res, r#"{"action":"remove"}"#);
    }

    #[test]
    fn room_event_carries_type_tag() {
        let ev = RoomEventBody::SyncRequired { room_id: "r1".into(), reason: "no_media_flow".into() };
        let v: serde_json::Value = serde_json::to_value(&ev).expect("ser");
        assert_eq!(v["type"], "sync_required");
    }
}
