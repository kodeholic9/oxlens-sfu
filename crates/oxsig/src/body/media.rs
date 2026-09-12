// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6-3 · model: claude-opus-5

//! 미디어 축 — `PUBLISH_TRACKS` · `READY` · `SUBSCRIBE_LAYER` · `TRACK_SET`.

use serde::{Deserialize, Serialize};

use crate::body::session::Duplex;
use crate::types::{Kind, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublishAction {
    /// ★부재는 `add` 다.
    #[default]
    Add,
    Remove,
}

/// 등록할 트랙 하나(`action:"add"`).
///
/// ★**`pt` 는 audio 도 필수다** — 폴백을 두면 무음을 조용히 만든다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTrack {
    pub kind: Kind,
    /// ★**내 offer 의 그 m-line `a=ssrc` 에서 읽는다** — 클라가 짓지 않는다.
    pub ssrc: u32,
    pub mid: String,
    /// ★**video 는 필수** — 없으면 전체 `1005`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// ★**협상 확정본의 파라미터 원문.** kind 를 가리지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    pub pt: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
    /// 시뮬캐스트가 아닐 때만.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_ssrc: Option<u32>,
    /// ★**video 만** · 닫힌 집합 — 그 밖의 값이나 audio 항목에 실리면 `1002`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    /// 없으면 서버 추론 — video 만(half=false · full=true). ★**audio 는 언제나 false**.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulcast: Option<bool>,
}

/// `0x0301 PUBLISH_TRACKS` 요청. ★**전량 수용 또는 전량 거절** — 부분 수용이 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTracksReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<PublishAction>,
    /// ★**등록 검사의 기준 방**이다(무전 코덱 `1006` 등).
    pub room_id: String,
    /// `add` 의 항목들.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<PublishTrack>,
    /// `remove` 는 ★**등록 응답이 준 `track_id` 로 지목한다**(`mid`·`ssrc` 가 아니다).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track_ids: Vec<String>,
    /// ★협상 결과 번호 — 안 보내면 서버 선언값.
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

/// 등록이 준 이름 — ★**이후 모든 식별이 이 값이다.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTrack {
    pub mid: String,
    pub track_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishTracksRes {
    pub action: PublishAction,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<PublishedTrack>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadyType {
    /// ★받기 협상이 성공한 뒤 — 막아 둔 영상을 흘리고 키프레임을 요청한다.
    Tracks,
    /// ★전이중 video 가 실제로 프레임을 내기 시작한 뒤 — 트랙당 한 번.
    Camera,
}

/// `0x0302 READY` 요청. 응답은 빈 body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyReq {
    /// ★**방마다** 보낸다 — 협상은 서버 단위지만 이 신호는 방 단위다.
    pub room_id: String,
    #[serde(rename = "type")]
    pub ready_type: ReadyType,
    /// ★`camera` 면 필수 — 프레임을 내기 시작한 그 트랙.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
}

/// 받을 레이어 하나. ★`spatial`·`temporal` 은 **상한**이고 생략은 *"안 바꾼다"* 다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerTarget {
    pub track_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<u8>,
    /// ★**별개 축이다** — 레이어 값에 섞으면 모르는 값이 최고 화질로 떨어져 대역이 폭증한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u16>,
}

/// `0x0303 SUBSCRIBE_LAYER` 요청. 응답은 빈 body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeLayerReq {
    pub room_id: String,
    pub targets: Vec<LayerTarget>,
}

/// `0x0304 TRACK_SET` 요청.
///
/// ★**`muted` 와 `duplex` 는 같이 못 온다**(`1007`) — 한 사실은 한 프레임이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSetReq {
    pub room_id: String,
    /// ★**이것을 먼저 쓴다.**
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    /// `track_id` 가 없을 때의 폴백.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssrc: Option<u32>,
    /// ★**전이중 트랙만** — 반이중에 오면 `3006`(송출 여부는 발언권이 정한다).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
}

impl TrackSetReq {
    /// ★같이 못 오는 필드가 같이 왔나 — 그 판정은 `1007` 이다.
    pub fn has_field_conflict(&self) -> bool {
        self.muted.is_some() && self.duplex.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_에도_pt_가_필수다() {
        // ★폴백은 무음을 조용히 만든다 — 형이 필수로 요구해야 한다.
        let err = serde_json::from_str::<PublishTrack>(
            r#"{"kind":"audio","ssrc":1,"mid":"0"}"#,
        );
        assert!(err.is_err(), "★pt 없는 항목은 형에서 걸린다");
    }

    #[test]
    fn 해제는_track_id_로_지목한다() {
        let r: PublishTracksReq = serde_json::from_str(
            r#"{"action":"remove","room_id":"r1","track_ids":["t1"]}"#,
        )
        .expect("parse");
        assert_eq!(r.action, Some(PublishAction::Remove));
        assert!(r.tracks.is_empty());
        assert_eq!(r.track_ids, vec!["t1".to_string()]);
    }

    #[test]
    fn 상한_생략은_안_바꾼다() {
        let t: LayerTarget = serde_json::from_str(r#"{"track_id":"t1","paused":true}"#).expect("parse");
        assert_eq!(t.spatial, None);
        assert_eq!(t.temporal, None);
        assert_eq!(t.paused, Some(true), "★paused 는 레이어 값과 별개 축이다");
    }

    #[test]
    fn muted_와_duplex_는_같이_못_온다() {
        let r = TrackSetReq {
            room_id: "r1".into(),
            track_id: Some("t1".into()),
            ssrc: None,
            muted: Some(true),
            duplex: Some(Duplex::Half),
        };
        assert!(r.has_field_conflict(), "★한 사실은 한 프레임이다 — 1007");
    }

    #[test]
    fn ready_는_방마다다() {
        let r: ReadyReq =
            serde_json::from_str(r#"{"room_id":"r1","type":"tracks"}"#).expect("parse");
        assert_eq!(r.ready_type, ReadyType::Tracks);
        assert!(r.track_id.is_none(), "★tracks 에는 track_id 가 없다");
    }
}
