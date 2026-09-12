// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6-1 · model: claude-opus-5

//! 세션 축 — `BIND` · `RESUME` · `HEARTBEAT` · `LEAVE`.

use serde::{Deserialize, Serialize};

use crate::types::{MemberInfo, TrackEntry, Version};

/// `0x0101 BIND` 요청. ★토큰은 **body 로** 보낸다 — 접속 주소 질의값에 안 싣는다(액세스 로그에 남는다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindReq {
    pub token: String,
    /// ★재접속 이어받기용 — 끊기기 전 세션의 것.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 클라 프로토콜 세대. 안 보내면 `1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ver: Option<u32>,
    /// ★**세션 단위**다. 안 보내면 `"2pc"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_mode: Option<PcMode>,
}

/// 이 규격서의 세대. `client_ver` 부재는 이 값으로 본다.
pub const CLIENT_VER: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PcMode {
    #[serde(rename = "1pc")]
    One,
    #[serde(rename = "2pc")]
    Two,
}

impl Default for PcMode {
    /// ★안 보내면 `2pc` — 규격이 정한 부재값이다.
    fn default() -> Self {
        PcMode::Two
    }
}

/// `0x0101 BIND` 응답.
///
/// ★**`session_id` 로 이어받기 성부를 안다** — 보낸 것과 같으면 세션이 살아 있고, 다르면 새 세션이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindRes {
    pub user_id: String,
    pub server_ver: u32,
    /// ms — 이 주기로 `HEARTBEAT` 를 보낸다.
    pub heartbeat_interval: u64,
    pub session_id: String,
    /// 이 시간 안에 돌아오면 이어받을 수 있다.
    pub resume_window_ms: u64,
    /// ★**확정값 에코** — 이것이 없으면 클라가 자기 모드를 서버에게 확인할 길이 없다.
    pub pc_mode: PcMode,
}

/// `0x0102 RESUME` 요청은 ★**빈 body** 다 — 신고할 것이 없다(서버가 아는 것을 다 낸다).
///
/// 형을 두지 않는 것이 계약이라 타입도 두지 않는다.
pub struct ResumeReq;

/// 한 방의 스냅샷 — `RESUME` 응답이 방마다 하나씩 든다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSnapshot {
    pub participants: Vec<MemberInfo>,
    pub tracks: Vec<TrackEntry>,
    pub version: Version,
}

/// ★**등록 층**(연§4-1) — 서버가 아는 내 등록. 스냅샷(스트림 층) 밖이라 따로 온다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    pub track_id: String,
    pub room_id: String,
    pub kind: crate::types::Kind,
    pub duplex: Duplex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmtp: Option<String>,
    #[serde(default)]
    pub simulcast: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Duplex {
    /// 회의 — 상시 흐른다.
    Full,
    /// 무전 — 발언권이 있을 때만 흐른다.
    Half,
}

/// `0x0102 RESUME` 응답.
///
/// ★**둘뿐이다** — `resumed[]`·`failed[]`·`reason{}`·`publish_failed[]` 는 없다.
/// 방 단위 판정을 두면 클라 신고가 필요해지고 무증상 실패가 생긴다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRes {
    /// 그 세션이 아는 방 전부 — 키가 `room_id` 다.
    pub snapshot: std::collections::BTreeMap<String, RoomSnapshot>,
    pub publications: Vec<Publication>,
}

/// `0x0104 LEAVE` — S→C body. ★`Failure` 에서 **`permanent` 를 뺀 것**이다.
///
/// 사유가 닫힌 집합이라 *"영구인가"* 를 물을 자리가 없다 — 종료는 되돌릴 대상이 아니다.
/// ★**C→S 는 빈 body** 다(*"자원을 남기고 나간다"* 는 뜻을 실을 자리를 두지 않는다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaveNotice {
    pub code: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl LeaveNotice {
    pub fn new(code: crate::Code) -> Self {
        Self { code: code.as_u16(), name: Some(code.name().to_string()), message: None, details: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Code;

    #[test]
    fn 토큰은_body_로_간다() {
        let r: BindReq = serde_json::from_str(r#"{"token":"jwt"}"#).expect("parse");
        assert_eq!(r.token, "jwt");
        assert!(r.pc_mode.is_none(), "★부재는 부재로 둔다 — 기본값을 미리 채우면 '안 보냈다'를 잃는다");
        assert_eq!(r.pc_mode.unwrap_or_default(), PcMode::Two);
    }

    #[test]
    fn pc_mode_는_wire_이름_그대로다() {
        assert_eq!(serde_json::to_string(&PcMode::One).expect("ser"), r#""1pc""#);
    }

    #[test]
    fn resume_응답에_부분_실패_형이_없다() {
        // ★형이 없다는 것이 계약이다 — 있으면 서버가 언젠가 쓴다.
        let json = serde_json::to_string(&ResumeRes {
            snapshot: Default::default(),
            publications: vec![],
        })
        .expect("ser");
        for 없어야 in ["resumed", "failed", "reason", "publish_failed"] {
            assert!(!json.contains(없어야), "★{없어야} 는 없는 형이다");
        }
    }

    #[test]
    fn leave_에는_permanent_가_없다() {
        let n = LeaveNotice::new(Code::ServerShutdown);
        let json = serde_json::to_string(&n).expect("ser");
        assert!(!json.contains("permanent"), "★종료는 되돌릴 대상이 아니다");
        assert!(json.contains("5004"));
    }
}
