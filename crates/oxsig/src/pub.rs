// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:PUB.type.PublishTrack · 연:PUB.op.PUBLISH_TRACKS · 연:PUB.op.TRACK_SET · 연:PUB.type.Publication · 연:RULE-PUB-REMOVE-FORM · 연:RULE-PUB-PUBLISH-RES · 연:RULE-PUB-SET-RES · model: claude-opus-5-5
//! 발행 — 트랙 올리기 · 내리기 · 속성 전환 · 발행 층.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::trk::{MediaKind, Source};

pub const TRACKS_PER_REQUEST: usize = 8;
pub const TRACKS_PER_USER: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Duplex {
    Full,
    Half,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishTrack {
    pub kind: MediaKind,
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

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExtIds {
    pub twcc: Option<u8>,
    pub rid: Option<u8>,
    pub repair_rid: Option<u8>,
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PublishTracksReq {
    Add { room_id: String, tracks: Vec<Value>, ext: ExtIds },
    Remove { room_id: String, track_ids: Vec<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishParseError {
    Invalid,
    Missing,
}

fn ext_id(v: &Value, k: &str) -> Result<Option<u8>, PublishParseError> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x.as_u64().filter(|n| *n <= 255).map(|n| Some(n as u8)).ok_or(PublishParseError::Invalid),
    }
}

impl PublishTracksReq {
    pub fn parse(v: &Value) -> Result<PublishTracksReq, PublishParseError> {
        let room_id = match v.get("room_id") {
            Some(Value::String(s)) => s.clone(),
            Some(_) => return Err(PublishParseError::Invalid),
            None => return Err(PublishParseError::Missing),
        };
        let action = match v.get("action") {
            None | Some(Value::Null) => "add",
            Some(Value::String(s)) if s == "add" => "add",
            Some(Value::String(s)) if s == "remove" => "remove",
            Some(_) => return Err(PublishParseError::Invalid),
        };
        if action == "remove" {
            let ids = match v.get("track_ids") {
                Some(Value::Array(a)) => a,
                Some(_) => return Err(PublishParseError::Invalid),
                None => return Err(PublishParseError::Missing),
            };
            let mut track_ids = Vec::with_capacity(ids.len());
            for x in ids {
                track_ids.push(x.as_str().ok_or(PublishParseError::Invalid)?.to_string());
            }
            return Ok(PublishTracksReq::Remove { room_id, track_ids });
        }
        let tracks = match v.get("tracks") {
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return Err(PublishParseError::Invalid),
            None => return Err(PublishParseError::Missing),
        };
        let ext = ExtIds {
            twcc: ext_id(v, "twcc_extmap_id")?,
            rid: ext_id(v, "rid_extmap_id")?,
            repair_rid: ext_id(v, "repair_rid_extmap_id")?,
            mid: ext_id(v, "mid_extmap_id")?,
            audio_level: ext_id(v, "audio_level_extmap_id")?,
        };
        Ok(PublishTracksReq::Add { room_id, tracks, ext })
    }

    pub fn room_id(&self) -> &str {
        match self {
            PublishTracksReq::Add { room_id, .. } | PublishTracksReq::Remove { room_id, .. } => room_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTrack {
    pub mid: String,
    pub track_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishAddRes {
    pub action: String,
    pub tracks: Vec<PublishedTrack>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishRemoveRes {
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSetRes {
    pub ssrc: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noop: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    pub track_id: String,
    pub room_id: String,
    pub kind: MediaKind,
    pub duplex: Duplex,
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    pub simulcast: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn action_defaults_to_add() {
        let r = PublishTracksReq::parse(&json!({"room_id":"r","tracks":[]}));
        assert!(matches!(r, Ok(PublishTracksReq::Add { .. })));
        let r = PublishTracksReq::parse(&json!({"action":"remove","room_id":"r","track_ids":["t"]}));
        assert!(matches!(r, Ok(PublishTracksReq::Remove { .. })));
        assert_eq!(PublishTracksReq::parse(&json!({"tracks":[]})), Err(PublishParseError::Missing));
        assert_eq!(PublishTracksReq::parse(&json!({"room_id":"r","action":"x"})), Err(PublishParseError::Invalid));
    }
}
