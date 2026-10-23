// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:SUB.type.MlineTable · 연:SUB.type.MlineEntry · 연:SUB.op.READY · 연:SUB.op.SUBSCRIBE_LAYER · 연:RULE-SUB-LAYER-PROC · model: claude-opus-5-5
//! 구독 — 연결마다 하나인 배정표(줄 = 구독 m-line) · 준비 신호 · 레이어 상한.

use serde::{Deserialize, Serialize};

use crate::trk::MediaKind;

pub const PRIORITY_DEFAULT: u8 = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineEntry {
    pub mid: String,
    pub kind: MediaKind,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineTable {
    pub sfu_id: String,
    pub arev: u64,
    pub mlines: Vec<MlineEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyReq {
    pub room_id: String,
    #[serde(default)]
    pub arev: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerTarget {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub targets: Vec<LayerTarget>,
}
