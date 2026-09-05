// author: kodeholic (powered by Claude)
//! 연§6-7 통지 넷 — 전부 S→C, 방 스코프, `version` 동반, 클라 의무는 ACK 하나.

use serde::{Deserialize, Serialize};

use crate::schema::{Affiliation, Duplex, MediaKind, TrackEntry, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantEventType {
    Joined,
    Left,
}

/// `0x0701` — `role`·`select` 는 `joined` 만.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantEvent {
    #[serde(rename = "type")]
    pub event_type: ParticipantEventType,
    pub room_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackAction {
    Add,
    Remove,
}

/// `0x0702` — 한 메시지는 한 방 것. 원소도 `room_id` 를 갖는다(보관본 규칙).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackEvent {
    pub action: TrackAction,
    pub room_id: String,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackStateType {
    Muted,
    Duplex,
    Live,
}

/// `0x0703` — 표시만 고친다. `track_id` 가 식별 키, `ssrc` 는 참고값.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackState {
    #[serde(rename = "type")]
    pub state_type: TrackStateType,
    pub user_id: String,
    pub track_id: String,
    pub ssrc: u32,
    pub kind: MediaKind,
    pub room_id: String,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEventType {
    Affiliation,
    SyncRequired,
}

/// 서버가 소속을 강제로 바꾼 사유. 결말은 `cause` 가 아니라 목록이 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForcedCause {
    Moderate,
    Kick,
    RoomClosed,
    MediaLost,
}

/// `0x0704` — 나에게만 unicast. `affiliation` 은 부분 목록(그 서버가 아는 만큼).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEvent {
    #[serde(rename = "type")]
    pub event_type: RoomEventType,
    pub room_id: String,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<Affiliation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<ForcedCause>,
    /// `sync_required` 의 로그용 값 — 처리는 `type` 으로만.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl RoomEvent {
    /// `affiliation` 통지에서 이 방이 목록에 남아 있는가 — 없으면 방이 닫힌 것.
    pub fn still_member(&self) -> bool {
        self.affiliation.as_ref().is_some_and(|a| a.sub_rooms.contains(&self.room_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn room_event_membership_is_judged_by_list() {
        let ev: RoomEvent = serde_json::from_value(json!({
            "type":"affiliation","room_id":"r1","version":{"epoch":"e","seq":3},
            "affiliation":{"sub_rooms":["r2"],"pub_room":null},"cause":"moderate"
        })).unwrap();
        assert!(!ev.still_member());
        let sr: RoomEvent = serde_json::from_value(json!({"type":"sync_required","room_id":"r1","version":{"epoch":"e","seq":4},"reason":"no_media_flow"})).unwrap();
        assert_eq!(sr.event_type, RoomEventType::SyncRequired);
    }

    #[test]
    fn track_state_by_type_fields() {
        let ts: TrackState = serde_json::from_value(json!({
            "type":"duplex","user_id":"u1","track_id":"t","ssrc":1,"kind":"video","room_id":"r1",
            "version":{"epoch":"e","seq":1},"duplex":"half","active":false,"source":"camera"
        })).unwrap();
        assert_eq!((ts.state_type, ts.active), (TrackStateType::Duplex, Some(false)));
    }
}
