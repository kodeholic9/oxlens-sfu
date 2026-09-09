// author: kodeholic (powered by Claude)
//! 연§6-3 미디어 — PUBLISH_TRACKS · READY · SUBSCRIBE_LAYER · TRACK_SET.

use serde::{Deserialize, Serialize};

use crate::code::FailCode;
use crate::schema::{Duplex, Extmap, MediaKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublishAction {
    #[default]
    Add,
    Remove,
}

/// `tracks[]` 원소(`add`). `ssrc`·`mid`·`pt` 필수, video 는 `codec` 필수.
/// `fmtp` 는 kind 무관 — 협상 확정본(answer)에 있으면 반드시 싣는다(연§6-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTrack {
    pub kind: MediaKind,
    /// 시뮬캐스트면 `0` 이 정상.
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
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulcast: Option<bool>,
}

impl PublishTrack {
    /// 연§6-3 — `simulcast` 가 없으면 서버 추론(half=false, full=true). audio 는 시뮬캐스트가 없다.
    pub fn simulcast_effective(&self) -> bool {
        self.kind == MediaKind::Video && self.simulcast.unwrap_or(self.duplex != Some(Duplex::Half))
    }
    /// 연§6-3 실패 — `1005` video 인데 codec 없음 · `1003` 시뮬캐스트 아닌데 `ssrc=0`.
    pub fn validate(&self) -> Result<(), FailCode> {
        if self.kind == MediaKind::Video && self.codec.is_none() {
            return Err(FailCode::CodecRequired);
        }
        if self.ssrc == 0 && !self.simulcast_effective() {
            return Err(FailCode::MissingField);
        }
        Ok(())
    }
}

/// `0x0301` 요청 — `add` 는 `tracks[]`, `remove` 는 `track_ids[]`(형상이 다르다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTracksReq {
    #[serde(default)]
    pub action: PublishAction,
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<PublishTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twcc_extmap_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rid_extmap_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_rid_extmap_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid_extmap_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_level_extmap_id: Option<u8>,
}

impl PublishTracksReq {
    /// 형상 검사 — 항목 하나라도 틀리면 전체 거절(연§6-3 원자성).
    pub fn validate(&self) -> Result<(), FailCode> {
        match self.action {
            PublishAction::Add => {
                if self.tracks.is_empty() || !self.track_ids.is_empty() {
                    return Err(FailCode::InvalidPayload);
                }
                if self.tracks.len() > crate::timers::MAX_TRACKS_PER_REQUEST {
                    return Err(FailCode::TrackLimit);
                }
                self.tracks.iter().try_for_each(PublishTrack::validate)
            }
            PublishAction::Remove => {
                if self.track_ids.is_empty() || !self.tracks.is_empty() {
                    return Err(FailCode::InvalidPayload);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTrack {
    pub mid: String,
    pub track_id: String,
}

/// 응답 — `remove` 는 `tracks` 필드 자체가 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTracksRes {
    pub action: PublishAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<PublishedTrack>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadyType {
    /// 받기 협상 성공 뒤 — 그 서버에서 들어가 있는 방마다.
    Tracks,
    /// `1pc` 전용 — 2단계 협상 직후, 확정본 신고.
    Transport,
    /// 카메라가 실제로 프레임을 내기 시작한 뒤 — `track_id` 필수.
    Camera,
}

/// `READY{transport}` 의 코덱 줄 — 확정 answer 의 PT 표.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecLine {
    pub pt: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
}

/// `0x0302 READY` 요청 / 응답 빈 body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyReq {
    pub room_id: String,
    #[serde(rename = "type")]
    pub ready_type: ReadyType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extmap: Option<Vec<Extmap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs: Option<Vec<CodecLine>>,
}

impl ReadyReq {
    /// 연§6-3 실패 `1003` — `camera` 에 `track_id` 없음 · `transport` 에 `extmap`/`codecs` 없음.
    pub fn validate(&self) -> Result<(), FailCode> {
        let ok = match self.ready_type {
            ReadyType::Tracks => true,
            ReadyType::Camera => self.track_id.is_some(),
            ReadyType::Transport => self.extmap.is_some() && self.codecs.is_some(),
        };
        if ok { Ok(()) } else { Err(FailCode::MissingField) }
    }
}

/// 연§6-3 — 요청은 "지정"이 아니라 "상한". 생략한 필드는 안 바꾼다. `paused` 는 별개 축.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

pub const LAYER_PRIORITY_DEFAULT: u8 = 128;

/// `0x0303 SUBSCRIBE_LAYER` 요청 / 응답 빈 body. 대상별 실패는 조용히 건너뛴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub targets: Vec<LayerTarget>,
}

/// `0x0304 TRACK_SET` 요청 — `muted` 와 `duplex` 는 배타(`1007`). 식별은 `track_id` 우선, `ssrc` 폴백.
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

impl TrackSetReq {
    pub fn validate(&self) -> Result<(), FailCode> {
        if self.muted.is_some() == self.duplex.is_some() {
            return Err(FailCode::FieldConflict);
        }
        if self.track_id.is_none() && self.ssrc.is_none() {
            return Err(FailCode::MissingField);
        }
        Ok(())
    }
}

/// 응답 — 요청한 축만 돌아온다. `noop:true` = 이미 그 상태.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn publish_shapes_and_validation() {
        let add: PublishTracksReq = serde_json::from_value(json!({
            "room_id":"r1","tracks":[{"kind":"video","ssrc":0,"mid":"1","codec":"VP8","pt":96,"simulcast":true}]
        })).unwrap();
        assert_eq!(add.action, PublishAction::Add);
        assert_eq!(add.validate(), Ok(()));
        let no_codec: PublishTracksReq = serde_json::from_value(json!({
            "room_id":"r1","tracks":[{"kind":"video","ssrc":5,"mid":"1","pt":96}]
        })).unwrap();
        assert_eq!(no_codec.validate(), Err(FailCode::CodecRequired));
        let zero_ssrc: PublishTracksReq = serde_json::from_value(json!({
            "room_id":"r1","tracks":[{"kind":"audio","ssrc":0,"mid":"0","pt":111}]
        })).unwrap();
        assert_eq!(zero_ssrc.validate(), Err(FailCode::MissingField));
        let rm: PublishTracksReq = serde_json::from_value(json!({"action":"remove","room_id":"r1","track_ids":["t1"]})).unwrap();
        assert_eq!(rm.validate(), Ok(()));
        let res = PublishTracksRes { action: PublishAction::Remove, tracks: None };
        let v = serde_json::to_value(&res).unwrap();
        assert!(v.get("tracks").is_none());
        assert!(v.get("intent").is_none(), "연§6-3 — intent 필드는 없다");
    }

    #[test]
    fn ready_and_track_set_rules() {
        let cam: ReadyReq = serde_json::from_value(json!({"room_id":"r1","type":"camera"})).unwrap();
        assert_eq!(cam.validate(), Err(FailCode::MissingField));
        let tr: ReadyReq = serde_json::from_value(json!({"room_id":"r1","type":"transport","extmap":[],"codecs":[]})).unwrap();
        assert_eq!(tr.validate(), Ok(()));
        let both: TrackSetReq = serde_json::from_value(json!({"room_id":"r1","track_id":"t","muted":true,"duplex":"half"})).unwrap();
        assert_eq!(both.validate(), Err(FailCode::FieldConflict));
        let none: TrackSetReq = serde_json::from_value(json!({"room_id":"r1","track_id":"t"})).unwrap();
        assert_eq!(none.validate(), Err(FailCode::FieldConflict));
        let ok: TrackSetReq = serde_json::from_value(json!({"room_id":"r1","ssrc":9,"duplex":"half"})).unwrap();
        assert_eq!(ok.validate(), Ok(()));
    }
}
