// author: kodeholic (powered by Claude)
//! 공통 스키마 — 연§4. JSON 의 모르는 필드는 무시한다(연§3-1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Duplex {
    Full,
    Half,
}

/// 연§6-1 — 세션 단위. 모르는 값은 `1002`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    OnePc,
    #[serde(rename = "2pc")]
    TwoPc,
}

/// 연§4-1 — 받을 트랙 하나. 네 경로가 같은 보관본 하나에 들어간다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackEntry {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub kind: MediaKind,
    pub ssrc: u32,
    pub track_id: String,
    /// 십진 정수 문자열. 고갈이면 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<Duplex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_ssrc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pt: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulcast: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalability: Option<String>,
}

impl TrackEntry {
    /// 연§4-1 — `mid` 없는 항목으로 SDP 를 조립하지 않는다.
    pub fn is_reachable(&self) -> bool {
        self.mid.is_some()
    }
    /// 무전 슬롯 = `user_id` 없음. `track_id` 는 파싱하지 않는다.
    pub fn is_slot(&self) -> bool {
        self.user_id.is_none()
    }
    /// 연§9-5 정렬은 수치로.
    pub fn mid_num(&self) -> Option<u32> {
        self.mid.as_deref().and_then(|m| m.parse().ok())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IceConfig {
    pub ip: String,
    pub port: u16,
    pub publish_ufrag: String,
    pub publish_pwd: String,
    pub subscribe_ufrag: String,
    pub subscribe_pwd: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DtlsConfig {
    /// `"sha-256 AB:CD:…"` — 가공 없이 `a=fingerprint:` 뒤에 치환한다.
    pub fingerprint: String,
    /// 항상 `"passive"` → 클라는 `active`.
    pub setup: String,
}

/// PT·클럭·fmtp 는 없다 — 협상 산물이라 클라 offer 값을 쓴다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecSpec {
    pub kind: MediaKind,
    pub name: String,
    #[serde(default)]
    pub rtcp_fb: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extmap {
    pub id: u8,
    pub uri: String,
}

/// 연§4-2 — `ROOM_JOIN` 응답에만. `sfu_id` 가 신원이고 `version.epoch` 와 같은 값.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub sfu_id: String,
    pub pc_mode: PcMode,
    pub ice: IceConfig,
    pub dtls: DtlsConfig,
    pub codecs: Vec<CodecSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs_sub: Option<Vec<CodecSpec>>,
    pub extmap: Vec<Extmap>,
    pub max_bitrate_bps: u64,
}

/// 연§4-3 — S→C 전용. 그 미디어 서버가 아는 만큼이다.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Affiliation {
    pub sub_rooms: Vec<String>,
    pub pub_room: Option<String>,
}

impl Affiliation {
    /// 연§6-4 불변식 — 깨진 응답은 서버 결함, 반영하지 않는다.
    pub fn is_consistent(&self) -> bool {
        self.pub_room.as_ref().is_none_or(|p| self.sub_rooms.contains(p))
    }
}

/// 연§4-4 — `role` 은 라벨, `select` 는 입장 시점 의도.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: String,
    #[serde(default = "default_role")]
    pub role: u8,
    #[serde(default = "default_select")]
    pub select: bool,
}

pub fn default_role() -> u8 {
    255
}
pub fn default_select() -> bool {
    true
}

/// 연§4-6 — 방마다 단조증가. 발급자는 미디어 서버 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    pub epoch: String,
    pub seq: u64,
}

/// 연§4-6 클라 규칙 셋.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionJudgement {
    /// epoch 가 다르다 → 보관본을 통째로 버리고 재구축
    Rebuild,
    /// seq 가 작거나 같다 → 버린다
    Stale,
    /// 바로 다음 → 반영
    Apply,
    /// 건너뛰었다 → 통짜로 다시 받는다
    Gap,
}

impl Version {
    pub fn judge(stored: Option<&Version>, incoming: &Version) -> VersionJudgement {
        let Some(s) = stored else { return VersionJudgement::Apply };
        if s.epoch != incoming.epoch {
            VersionJudgement::Rebuild
        } else if incoming.seq <= s.seq {
            VersionJudgement::Stale
        } else if incoming.seq == s.seq + 1 {
            VersionJudgement::Apply
        } else {
            VersionJudgement::Gap
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn track_entry_unknown_fields_ignored_and_slot_rules() {
        let v = json!({"room_id":"r1","kind":"video","ssrc":5,"track_id":"ptt-r1-video","mid":"12","duplex":"half","pt":96,"codec":"VP8","future":1});
        let t: TrackEntry = serde_json::from_value(v).unwrap();
        assert!(t.is_slot() && t.is_reachable());
        assert_eq!(t.mid_num(), Some(12));
        assert_eq!(t.duplex, Some(Duplex::Half));
    }

    #[test]
    fn version_rules() {
        let s = Version { epoch: "e".into(), seq: 10 };
        let mk = |e: &str, q| Version { epoch: e.into(), seq: q };
        assert_eq!(Version::judge(Some(&s), &mk("e", 11)), VersionJudgement::Apply);
        assert_eq!(Version::judge(Some(&s), &mk("e", 10)), VersionJudgement::Stale);
        assert_eq!(Version::judge(Some(&s), &mk("e", 13)), VersionJudgement::Gap);
        assert_eq!(Version::judge(Some(&s), &mk("f", 1)), VersionJudgement::Rebuild);
        assert_eq!(Version::judge(None, &mk("f", 1)), VersionJudgement::Apply);
    }

    #[test]
    fn affiliation_invariant_and_pc_mode_strings() {
        assert!(Affiliation { sub_rooms: vec!["r1".into()], pub_room: Some("r1".into()) }.is_consistent());
        assert!(!Affiliation { sub_rooms: vec![], pub_room: Some("r1".into()) }.is_consistent());
        assert_eq!(serde_json::to_string(&PcMode::OnePc).unwrap(), "\"1pc\"");
        let m: MemberInfo = serde_json::from_value(json!({"user_id":"u"})).unwrap();
        assert_eq!((m.role, m.select), (255, true));
    }
}
