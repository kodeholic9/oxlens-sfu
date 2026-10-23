// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:AFF.type.Affiliation · 연:AFF.op.AFFILIATION · 연:RULE-AFF-PROC · model: claude-opus-5-5
//! 소속 — 그 서버에 입장한 방(= 구독) · 발행 방 하나.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Affiliation {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AffiliationReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffiliationRes {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}
