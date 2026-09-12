// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6-2 · §4-2 · model: claude-opus-5

//! 방 축 — `ROOM_JOIN` · `ROOM_LEAVE`. ★**입장이 곧 구독**이라 따로 구독 op 이 없다.

use serde::{Deserialize, Serialize};

use crate::body::session::PcMode;
use crate::types::{Affiliation, Kind, MemberInfo, TrackEntry, Version};

/// 확장 헤더 번호 하나 — `{id, uri}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtmapEntry {
    pub id: u8,
    pub uri: String,
}

/// 코덱 하나. ★**PT·클럭·fmtp 는 없다** — 그것은 협상이 정한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecCap {
    pub kind: Kind,
    pub name: String,
    #[serde(default)]
    pub rtcp_fb: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceConfig {
    pub ip: String,
    pub port: u16,
    /// ★**보내기 연결**의 자격.
    pub publish_ufrag: String,
    pub publish_pwd: String,
    /// ★**받기 연결**의 자격 — 재협상에도 같은 값을 계속 쓴다(바꾸면 브라우저가 ICE 재시작으로 오인).
    pub subscribe_ufrag: String,
    pub subscribe_pwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DtlsConfig {
    /// ★`"sha-256 AB:CD:…"` — 클라는 가공 없이 `a=fingerprint:` 뒤에 붙인다.
    pub fingerprint: String,
    /// ★항상 `"passive"` — 그래서 클라가 `active` 다.
    pub setup: String,
}

/// 미디어를 붙이는 재료(연§4-2). ★`sfu_id` 마다 하나이고 같은 서버의 방들은 같은 값을 받는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// ★**그 미디어 서버의 신원**(불투명) — ★**뜻을 해석하지 않고 같은지만 본다.**
    pub sfu_id: String,
    /// ★**그 서버가 적용한 값** — 세션 확정값과 반드시 같다.
    pub pc_mode: PcMode,
    pub ice: IceConfig,
    pub dtls: DtlsConfig,
    /// 보내기에 쓸 코덱.
    pub codecs: Vec<CodecCap>,
    /// 있을 때만 — 받기 전용 정책.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs_sub: Option<Vec<CodecCap>>,
    pub extmap: Vec<ExtmapEntry>,
    pub max_bitrate_bps: u64,
}

/// `0x0201 ROOM_JOIN` 요청.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJoinReq {
    pub room_id: String,
    /// 기본 `255`. ★**앱이 붙이는 라벨이고 권한이 아니다** — 서버는 저장만 한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<u8>,
    /// 기본 `true` = 이 방에서 말한다. `false` = 듣기만. ★명단에 그대로 에코된다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<bool>,
    /// ★`1pc` 필수 — 내 번호표(확장 ID 표). 서버가 이것을 피해 배정한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extmap: Option<Vec<ExtmapEntry>>,
    /// ★`1pc` 필수 — 내 번호표(PT 표).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codecs: Option<Vec<JoinCodec>>,
}

/// `1pc` 번호표의 코덱 한 줄 — 브라우저 offer 가 선언한 값 그대로.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinCodec {
    pub kind: Kind,
    pub name: String,
    pub pt: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
}

/// `0x0201 ROOM_JOIN` 응답. ★**완결 스냅샷이다** — 기존 트랙까지 담는다.
///
/// 그래서 관심 선언이 늦어 그 창의 통지를 놓쳐도 ★**화면이 빈 채로 서지 않는다**(정§15-4 안전망).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJoinRes {
    pub room_id: String,
    pub participants: Vec<MemberInfo>,
    pub affiliation: Affiliation,
    pub server_config: ServerConfig,
    pub tracks: Vec<TrackEntry>,
    /// ★**견주지 않고 재설정한다** — 스냅샷이다(연§4-6-4 규칙 2).
    pub version: Version,
}

/// `0x0202 ROOM_LEAVE` 요청.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveReq {
    pub room_id: String,
}

/// `0x0202 ROOM_LEAVE` 응답. ★**`version` 이 없다** — 종결이라 재설정할 스냅샷이 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomLeaveRes {
    pub room_id: String,
    pub affiliation: Affiliation,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 입장_기본은_말한다() {
        let r: RoomJoinReq = serde_json::from_str(r#"{"room_id":"r1"}"#).expect("parse");
        assert_eq!(r.select, None);
        assert!(r.select.unwrap_or(true), "★wire 기본은 true 다(SDK 표면의 기본과 반대다)");
    }

    #[test]
    fn 퇴장_응답에는_version_이_없다() {
        let res = RoomLeaveRes {
            room_id: "r1".into(),
            affiliation: Affiliation { sub_rooms: vec![], pub_room: None },
        };
        let json = serde_json::to_string(&res).expect("ser");
        assert!(!json.contains("version"), "★종결이라 재설정할 스냅샷이 없다");
    }

    #[test]
    fn 일pc_번호표는_있을_때만_실린다() {
        let r = RoomJoinReq {
            room_id: "r1".into(),
            role: None,
            select: None,
            extmap: None,
            codecs: None,
        };
        let json = serde_json::to_string(&r).expect("ser");
        assert_eq!(json, r#"{"room_id":"r1"}"#, "★2pc 는 번호표를 안 싣는다");
    }

    #[test]
    fn 서버_설정은_받기_자격을_따로_든다() {
        // ★보내기와 받기 자격을 한 값으로 합치면 재협상에서 ICE 재시작으로 오인된다.
        let c: ServerConfig = serde_json::from_str(
            r#"{"sfu_id":"sfu-1","pc_mode":"2pc",
                "ice":{"ip":"1.2.3.4","port":7000,"publish_ufrag":"a","publish_pwd":"b",
                       "subscribe_ufrag":"c","subscribe_pwd":"d"},
                "dtls":{"fingerprint":"sha-256 AB:CD","setup":"passive"},
                "codecs":[{"kind":"audio","name":"opus","rtcp_fb":["transport-cc"]}],
                "extmap":[{"id":1,"uri":"urn:ietf:params:rtp-hdrext:sdes:mid"}],
                "max_bitrate_bps":800000}"#,
        )
        .expect("parse");
        assert_ne!(c.ice.publish_ufrag, c.ice.subscribe_ufrag);
        assert_eq!(c.dtls.setup, "passive", "★서버는 언제나 passive 다");
        assert!(c.codecs_sub.is_none());
    }
}
