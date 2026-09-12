// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6-7 · §4-6-3 · model: claude-opus-5

//! S→C 통지 다섯.
//!
//! ★**접미사가 규율이다** — `_EVENT` 는 존재·관계가 바뀌었다(구조를 고친다) ·
//! `_STATE` 는 속성이 바뀌었다(표시만 고친다).
//!
//! ★★**`version` 은 델타 넷에만 실린다** — `ROOM_EVENT` 는 방 공통 스냅샷 축이 아니라 안 싣는다.

use serde::{Deserialize, Serialize};

use crate::body::data::AffiliationCause;
use crate::types::{Affiliation, Kind, Permission, TrackEntry, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantChange {
    Joined,
    Left,
}

/// `0x0701 PARTICIPANT_EVENT` — 누가 들고났다. ★**투명 참가자는 여기 안 실린다.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantEvent {
    #[serde(rename = "type")]
    pub change: ParticipantChange,
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
    pub version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackAction {
    Add,
    Remove,
}

/// `0x0702 TRACK_EVENT` — 받을 트랙이 생기거나 없어졌다.
///
/// ★**한 메시지에 방 하나다** — 섞으면 `version` 이 어느 방 것인지 정의되지 않는다.
/// ★**발행자 본인에게도 자기 항목이 간다**(`assign` 없는 항목) — 안 보내면 그 사람만 `seq` 가 건너뛴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackEvent {
    pub action: TrackAction,
    pub room_id: String,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

/// `0x0703 TRACK_STATE` — 트랙 **속성**이 바뀌었다. ★지금 실린 것은 `muted` 하나다.
///
/// ★**반이중 트랙에는 오지 않는다** — 무전에는 mute 가 없다(송출은 발언권이 정한다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackState {
    #[serde(rename = "type")]
    pub state_type: TrackStateType,
    pub user_id: String,
    pub track_id: String,
    pub ssrc: u32,
    pub kind: Kind,
    pub room_id: String,
    pub version: Version,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackStateType {
    Muted,
}

/// `0x0705 PARTICIPANT_STATE` — 참가자 **속성**이 바뀌었다.
///
/// ★**그 방 전원에게** 간다(본인만이 아니다 — 지령대가 *"누가 말할 수 있나"* 를 그린다).
/// ★**비트 넷 전량**을 싣는다(델타가 아니다 — 통지 하나를 놓치면 영구히 어긋난다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantState {
    #[serde(rename = "type")]
    pub state_type: ParticipantStateType,
    pub user_id: String,
    pub room_id: String,
    pub version: Version,
    pub permission: Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantStateType {
    Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEventType {
    Affiliation,
    SyncRequired,
}

/// `0x0704 ROOM_EVENT` — 나와 방의 관계·정합성. ★**unicast** 이고 ★**`version` 을 싣지 않는다.**
///
/// 소속은 방 공통 스냅샷 밖의 값이라 되감길 대상이 없고, 방 소멸은 올릴 카운터가 이미 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomEvent {
    #[serde(rename = "type")]
    pub event_type: RoomEventType,
    /// ★이 변화를 일으킨 방.
    pub room_id: String,
    /// `affiliation` 일 때 — 바뀐 뒤의 스냅샷.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<Affiliation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<AffiliationCause>,
    /// `sync_required` 일 때 — 로그·사람용이다(클라가 분기하지 않는다).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> Version {
        Version { epoch: "sfu-1".into(), seq: 10 }
    }

    #[test]
    fn 방_이벤트는_version_을_안_싣는다() {
        let e = RoomEvent {
            event_type: RoomEventType::Affiliation,
            room_id: "r1".into(),
            affiliation: Some(Affiliation { sub_rooms: vec![], pub_room: None }),
            cause: Some(AffiliationCause::RoomClosed),
            reason: None,
        };
        let json = serde_json::to_string(&e).expect("ser");
        assert!(!json.contains("version"), "★되감길 스냅샷이 없다 — 실으면 hub 가 번호를 지어야 한다");
        assert!(json.contains("room_closed"));
    }

    #[test]
    fn 델타_넷은_version_을_싣는다() {
        let p = ParticipantEvent {
            change: ParticipantChange::Joined,
            room_id: "r1".into(),
            user_id: "u1".into(),
            role: None,
            select: None,
            participant_type: None,
            metadata: None,
            version: v(),
        };
        assert!(serde_json::to_string(&p).expect("ser").contains("\"seq\":10"));
    }

    #[test]
    fn 권한_통지는_넷_전량이다() {
        let s: ParticipantState = serde_json::from_str(
            r#"{"type":"permission","user_id":"u1","room_id":"r1",
                "version":{"epoch":"e","seq":3},
                "permission":{"publish_audio":true,"publish_video":false,
                              "publish_screen":false,"floor_request":true}}"#,
        )
        .expect("parse");
        // ★델타면 놓친 통지 하나가 영구히 어긋난 비트를 남긴다 — 형이 전량을 요구한다.
        assert!(!s.permission.publish_video);
        assert!(s.permission.floor_request);
    }

    #[test]
    fn 트랙_이벤트는_방_하나다() {
        let e: TrackEvent = serde_json::from_str(
            r#"{"action":"add","room_id":"r1","tracks":[],"version":{"epoch":"e","seq":1}}"#,
        )
        .expect("parse");
        assert_eq!(e.room_id, "r1");
        assert_eq!(e.action, TrackAction::Add);
    }

    #[test]
    fn 트랙_상태는_muted_하나다() {
        let s: TrackState = serde_json::from_str(
            r#"{"type":"muted","user_id":"u1","track_id":"t1","ssrc":9,"kind":"video",
                "room_id":"r1","version":{"epoch":"e","seq":2},"muted":true}"#,
        )
        .expect("parse");
        assert_eq!(s.state_type, TrackStateType::Muted);
        assert!(s.muted);
    }
}
