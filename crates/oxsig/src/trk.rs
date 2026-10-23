// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:TRK.type.TrackEntry · 연:TRK.op.TRACK_EVENT · 연:TRK.op.TRACK_STATE · 연:RULE-TRK-FIELDS-BY-TYPE · 연:RULE-TRK-PER-PATH · model: claude-opus-5-5
//! 트랙 — 받을 트랙 하나(개인 · 슬롯)와 그 목록 · 속성 통지.

use serde::{Deserialize, Serialize};

use crate::room::Version;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Audio => "audio",
            MediaKind::Video => "video",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    Individual,
    Slot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Camera,
    Screen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackEntry {
    #[serde(rename = "type")]
    pub kind_of: TrackType,
    pub room_id: String,
    pub track_id: String,
    pub kind: MediaKind,
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

impl TrackEntry {
    pub fn removal(&self) -> TrackEntry {
        TrackEntry {
            kind_of: self.kind_of,
            room_id: self.room_id.clone(),
            track_id: self.track_id.clone(),
            kind: self.kind,
            codec: None,
            fmtp: None,
            user_id: None,
            source: None,
            active: None,
            muted: None,
            simulcast: None,
            scalability: None,
        }
    }
}

pub fn slot_track_id(room_id: &str, kind: MediaKind) -> String {
    format!("ptt-{room_id}-{}", kind.as_str())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackAction {
    Add,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackEvent {
    pub action: TrackAction,
    pub room_id: String,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackState {
    #[serde(rename = "type")]
    pub kind_of: String,
    pub user_id: String,
    pub track_id: String,
    pub kind: MediaKind,
    pub room_id: String,
    pub version: Version,
    pub muted: bool,
}
