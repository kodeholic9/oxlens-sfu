// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:LINK.type.ServerConfig · 연:LINK.type.CodecPolicy · 연:LINK.fact.ServerExtMap · 연:LINK.op.ICE_RESTART · 연:LINK.op.LINK_EVENT · 연:LINK.op.MLINE_EVENT · 연:LINK.fact.SdpClock · model: claude-opus-5-5
//! 연결 — 미디어를 붙이는 재료 · ICE 재시작 · 연결 의심 · 배정표 통지.

use serde::{Deserialize, Serialize};

use crate::sub::MlineTable;
use crate::trk::MediaKind;

pub const EXT_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const EXT_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const EXT_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const EXT_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const EXT_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const EXT_RRID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";

pub const SERVER_EXTMAP: [(u8, &str); 6] = [
    (1, EXT_MID),
    (4, EXT_AUDIO_LEVEL),
    (5, EXT_ABS_SEND_TIME),
    (6, EXT_TWCC),
    (10, EXT_RID),
    (11, EXT_RRID),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtMap {
    pub id: u8,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecPolicy {
    pub kind: MediaKind,
    pub name: String,
    pub rtcp_fb: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceInfo {
    pub ip: String,
    pub port: u16,
    pub publish_ufrag: String,
    pub publish_pwd: String,
    pub subscribe_ufrag: String,
    pub subscribe_pwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp_port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DtlsInfo {
    pub fingerprint: String,
    pub setup: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub sfu_id: String,
    pub pc_mode: String,
    pub ice: IceInfo,
    pub dtls: DtlsInfo,
    pub codecs: Vec<CodecPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs_sub: Option<Vec<CodecPolicy>>,
    pub extmap: Vec<ExtMap>,
    pub max_bitrate_bps: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartReq {
    pub sfu_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceCred {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceRestartRes {
    pub publish: IceCred,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<IceCred>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEvent {
    #[serde(rename = "type")]
    pub kind_of: String,
    pub sfu_id: String,
    pub ufrag: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlineEvent {
    pub table: MlineTable,
}

pub fn clock_of(codec: &str) -> u32 {
    if codec.eq_ignore_ascii_case("opus") {
        48_000
    } else {
        90_000
    }
}
