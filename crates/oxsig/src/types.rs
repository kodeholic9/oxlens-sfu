// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§4-1 · §4-3 · §4-4 · §4-4-1 · §4-5 · §4-6 · model: claude-opus-5

//! 공통 형 — 여러 op 이 같은 형을 나른다. ★**한 보관본에 들어가는 것들**이라 형도 하나다.

use serde::{Deserialize, Serialize};

/// ★**무엇이 최신인가**(연§4-6). `epoch` 가 바뀌면 `seq` 를 견주지 않고 재설정한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// 미디어 서버 프로세스 기동 단위 불투명 값 — `server_config.sfu_id` 와 같은 값이다.
    pub epoch: String,
    /// 그 방 공통 스냅샷이 바뀔 때마다 하나씩 오른다.
    pub seq: u64,
}

/// 스트림 종류. ★**바뀌지 않는다** — 회의↔무전 전환은 등록 속성(`duplex`)이 바뀌는 것이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    /// 한 사람의 트랙.
    Individual,
    /// 방 공용 — 여러 사람이 돌려쓰는 한 m-line. 주인이 없다.
    Slot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Audio,
    Video,
}

/// video 개인 스트림의 출처. ★**닫힌 집합이다**(연§4-4-1 사상표 — 모르는 값은 `1002`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Camera,
    Screen,
}

/// ★**배정 층** — 그 스트림의 **내** 자리. 사람마다 다르고 `seq` 대상이 아니다.
///
/// ★**내가 그 방에 입장 중이고 자리가 붙어 있을 때만 온다** — 없으면 필드 자체가 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assign {
    /// 내 받기 m-line 번호 — ★**십진 정수 문자열**이다(정렬은 수치로).
    pub mid: String,
    /// ★**서버가 이 받기 BUNDLE 에 배정한 PT**(7비트) — 발행자 값이 아니다.
    pub pt: u8,
    /// 재전송 PT. ★`rtx_ssrc` 와 짝이다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_pt: Option<u8>,
}

/// ★**받을 스트림 하나**(연§4-1) — 받기 SDP 를 조립하는 재료 전부다.
///
/// ★**세 층 중 둘만 여기 있다** — 스트림(방 전원 같은 값)과 배정(수신자 것).
/// 등록 층(`codec`·`ssrc`·`duplex`·`simulcast`)은 ★**스냅샷 밖**이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackEntry {
    #[serde(rename = "type")]
    pub stream_type: StreamType,
    /// ★이 스트림이 어느 방 것인가. 항상 온다.
    pub room_id: String,
    /// ★식별 키 — 보관본의 키. ★**파싱하지 않는다**(슬롯 이름에 규칙이 있어도).
    pub track_id: String,
    pub kind: Kind,
    /// 시뮬캐스트면 서버가 만든 가상 SSRC · 슬롯이면 슬롯 SSRC · 그 외엔 원본.
    pub ssrc: u32,
    /// 재전송 SSRC. ★`assign.rtx_pt` 와 짝으로 온다 — 하나만 오면 실패로 다룬다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtx_ssrc: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// ★보내는 쪽 협상 **확정본**의 `a=fmtp` 원문(파라미터만, PT 제외). kind 를 가리지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    /// 보내는 사람. ★**슬롯에는 없다** — 주인이 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// video 개인 스트림만.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// `false` 면 지금은 안 온다 — 회의→무전 전환으로 쉬는 individual 스트림.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    /// 그 트랙이 음소거인가 — `TRACK_STATE{muted}` 와 같은 값. 슬롯에는 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// `"L{spatial}T{temporal}"` — 몇 단이 있는가. 시뮬캐스트/SVC 일 때만.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign: Option<Assign>,
}

/// ★**그 사람이 무엇을 할 수 있나**(연§4-4·§4-4-1) — `role` 과 다른 축이다.
///
/// ★**기본과 다를 때만 명단에 실린다** — 없으면 넷 다 허용이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permission {
    pub publish_audio: bool,
    pub publish_video: bool,
    pub publish_screen: bool,
    pub floor_request: bool,
}

impl Default for Permission {
    /// ★**부재 = 전부 허용.** 명단이 안 실었다는 것은 기본값이라는 뜻이다.
    fn default() -> Self {
        Self { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true }
    }
}

/// 명단 한 줄(연§4-4). ★**투명(`hidden`) 참가자는 여기 안 오른다.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub user_id: String,
    /// `u8`, 기본 `255`. 뜻은 앱이 준다.
    pub role: u8,
    /// ★**입장 시점의 의도** — `true` 참여 / `false` 청취. 입장 뒤 바뀌지 않는다.
    pub select: bool,
    /// ★**서버가 토큰에서 채운다** — 클라 신고값이 아니다.
    pub participant_type: u8,
    /// 토큰에 있을 때만.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// ★**기본과 다를 때만.** `None` 이면 `Permission::default()` 다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

impl MemberInfo {
    /// ★부재를 기본값으로 펴 준다 — 소비처마다 다시 펴면 그때마다 갈린다.
    pub fn permission(&self) -> Permission {
        self.permission.unwrap_or_default()
    }
}

/// ★**어느 방을 듣고 어디로 말하나**(연§4-3). ★**S→C 전용** — 요청 형상이 아니다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Affiliation {
    /// 내가 듣는 방들 = 그 서버에 입장한 방들(입장이 곧 구독).
    pub sub_rooms: Vec<String>,
    /// ★**하나뿐이라 단수다.** 없으면 `null`.
    pub pub_room: Option<String>,
}

/// 실패 응답 body(연§4-5). ★`flags=10` 프레임의 body 는 언제나 이것이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub code: u16,
    pub name: String,
    /// ★`LEAVE` 사유로 쓰인 코드에는 이 축이 없다 — 그때는 싣지 않는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permanent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Failure {
    /// 코드 하나로 짓는다 — `name`·`permanent` 는 ★**표가 정본**이라 지어내지 않는다.
    pub fn new(code: crate::Code) -> Self {
        Self {
            code: code.as_u16(),
            name: code.name().to_string(),
            permanent: code.permanent(),
            message: None,
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Code;

    #[test]
    fn 권한_부재는_전부_허용이다() {
        let m: MemberInfo = serde_json::from_str(
            r#"{"user_id":"u1","role":255,"select":false,"participant_type":0}"#,
        )
        .expect("parse");
        assert_eq!(m.permission(), Permission::default());
        assert!(m.permission().floor_request);
    }

    #[test]
    fn 기본_권한은_명단에_안_실린다() {
        let m = MemberInfo {
            user_id: "u1".into(),
            role: 255,
            select: false,
            participant_type: 0,
            metadata: None,
            permission: None,
        };
        let s = serde_json::to_string(&m).expect("ser");
        assert!(!s.contains("permission"), "★기본값은 싣지 않는다 — 정원 1000 의 명단이 통째로 나간다");
    }

    #[test]
    fn 슬롯에는_주인이_없다() {
        let e: TrackEntry = serde_json::from_str(
            r#"{"type":"slot","room_id":"r1","track_id":"ptt-r1-audio","kind":"audio","ssrc":7}"#,
        )
        .expect("parse");
        assert_eq!(e.stream_type, StreamType::Slot);
        assert!(e.user_id.is_none());
        assert!(e.assign.is_none(), "★미배정이면 필드 자체가 없다");
    }

    #[test]
    fn 배정은_십진_문자열_mid_다() {
        let e: TrackEntry = serde_json::from_str(
            r#"{"type":"individual","room_id":"r1","track_id":"t1","kind":"video","ssrc":9,
                "user_id":"u2","assign":{"mid":"34","pt":96}}"#,
        )
        .expect("parse");
        let a = e.assign.expect("assign");
        assert_eq!(a.mid, "34");
        assert_eq!(a.rtx_pt, None);
    }

    #[test]
    fn 모르는_필드는_무시한다() {
        // ★서버가 필드를 늘려도 구버전이 안 깨진다(연§3-1).
        let e: TrackEntry = serde_json::from_str(
            r#"{"type":"individual","room_id":"r1","track_id":"t1","kind":"audio","ssrc":1,
                "user_id":"u1","무엇인가":true}"#,
        )
        .expect("parse");
        assert_eq!(e.track_id, "t1");
    }

    #[test]
    fn 실패는_표에서_짓는다() {
        let f = Failure::new(Code::RoomNotFound);
        assert_eq!(f.name, "ROOM_NOT_FOUND");
        assert_eq!(f.permanent, Some(false));
        let leave = Failure::new(Code::ServerShutdown);
        assert_eq!(leave.permanent, None, "★LEAVE 사유에는 재시도 축이 없다");
    }

    #[test]
    fn 발행_방은_단수이고_없을_수_있다() {
        let a: Affiliation =
            serde_json::from_str(r#"{"sub_rooms":["r1","r2"],"pub_room":null}"#).expect("parse");
        assert_eq!(a.sub_rooms.len(), 2);
        assert!(a.pub_room.is_none());
    }
}
