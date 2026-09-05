// author: kodeholic (powered by Claude)
//! 연§6-4 AFFILIATION — 발언 방 전환 전용. 요청 하나 = 미디어 서버 하나.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::Version;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AffiliationReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

/// 응답의 `cause` 는 둘뿐 — 강제 변경은 `ROOM_EVENT{affiliation}` 이 나른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cause {
    User,
    Implicit,
}

/// 응답 — 적용 후 그 서버가 아는 만큼의 스냅샷 + 바뀐 방마다 `versions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffiliationRes {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
    pub cause: Cause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
    #[serde(default)]
    pub versions: BTreeMap<String, Version>,
}

impl AffiliationRes {
    /// 연§6-4 불변식 `pub_room ⊆ sub_rooms` — 깨지면 서버 결함, 반영하지 않는다.
    pub fn is_consistent(&self) -> bool {
        self.pub_room.as_ref().is_none_or(|p| self.sub_rooms.contains(p))
    }
}
