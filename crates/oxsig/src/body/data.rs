// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6-4 · §6-5 · §6-6 · model: claude-opus-5

//! 소속·데이터·진단 — `AFFILIATION` · `MESSAGE` · `TASK`.

use serde::{Deserialize, Serialize};

use crate::types::Affiliation;

/// `0x0401 AFFILIATION` 요청 — ★**어디로 발행할까**. 둘 다 생략하면 조회다.
///
/// ★적용 순서는 `pub_deselect` → `pub_select` 다(같은 요청에 둘 다 와도).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffiliationReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_select: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_deselect: Option<String>,
    /// 내가 붙인 번호 — 응답에 되돌아온다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

/// 소속이 왜 바뀌었나. `AFFILIATION` 응답과 `ROOM_EVENT` 가 같은 어휘를 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffiliationCause {
    /// ★`AFFILIATION` 응답의 **유일한** 값 — 내가 이 요청으로 바꿨다(연§6-4).
    ///
    /// ★입·퇴장이 곁들여 바꾼 것(`ROOM_JOIN`/`ROOM_LEAVE` 응답)은 ★**`cause` 필드가 아예 없다** —
    /// 연§6-4 가 그것을 *implicit* 이라 부르지만 ★**wire 값이 아니다.** 값을 두면
    /// 그 자리에 무엇을 넣을지가 매번 갈린다.
    User,
    /// 강퇴 — 운영 경로.
    Kick,
    /// 방이 사라졌다.
    RoomClosed,
    /// 미디어가 죽어 서버가 거뒀다.
    MediaLost,
}

/// `0x0401 AFFILIATION` 응답 — ★**스냅샷이다**(요청 반영이 아니다).
///
/// ★**`version` 이 없다** — 내 세션 것이라 방 공통 스냅샷 밖이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffiliationRes {
    #[serde(flatten)]
    pub affiliation: Affiliation,
    pub cause: AffiliationCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_id: Option<String>,
}

/// `0x0501 MESSAGE` — 보낼 때.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSend {
    pub room_id: String,
    pub content: String,
}

/// `0x0501 MESSAGE` 응답.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRes {
    pub msg_id: String,
}

/// `0x0501 MESSAGE` — 받을 때. ★**보낸 사람에게는 에코하지 않는다**(자기 것은 `pid` 로 안다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRecv {
    pub room_id: String,
    pub user_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPhase {
    Request,
    Report,
}

/// `0x0601 TASK` — 같은 형이 양방향이다. ★`report` 는 **새 `00` 프레임**이고 자기 `pid` 를 가진다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub phase: TaskPhase,
    pub req_id: u64,
    /// ★**모르는 값을 조용히 버리지 않는다** — `1002` 로 답한다(버리면 발제자가 타임아웃까지 기다린다).
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// ★**수집 자체가 실패했을 때 이것만** 보낸다. 정상이면 없다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 소속_응답은_version_을_안_싣는다() {
        let res = AffiliationRes {
            affiliation: Affiliation { sub_rooms: vec!["r1".into()], pub_room: Some("r1".into()) },
            cause: AffiliationCause::User,
            change_id: None,
        };
        let json = serde_json::to_string(&res).expect("ser");
        assert!(!json.contains("version"), "★내 세션 것이라 방 공통 스냅샷 밖이다");
        // ★응답의 `cause` 는 이 값 하나다 — 입·퇴장이 곁들여 바꾼 것은 `cause` 필드가 없다.
        assert!(json.contains(r#""cause":"user""#), "{json}");
        assert!(json.contains("sub_rooms"), "★평평하게 편다 — 중첩하면 형이 둘이 된다");
    }

    #[test]
    fn 받은_메시지는_보낸_사람을_싣는다() {
        let m: MessageRecv =
            serde_json::from_str(r#"{"room_id":"r1","user_id":"u2","content":"안녕"}"#).expect("parse");
        assert_eq!(m.user_id, "u2");
    }

    #[test]
    fn task_는_같은_형이_양방향이다() {
        let req: Task =
            serde_json::from_str(r#"{"phase":"request","req_id":7,"type":"probe","params":{}}"#)
                .expect("parse");
        assert_eq!(req.phase, TaskPhase::Request);
        assert!(req.result.is_none());
        let rep: Task =
            serde_json::from_str(r#"{"phase":"report","req_id":7,"type":"probe","result":{}}"#)
                .expect("parse");
        assert_eq!(rep.req_id, req.req_id, "★req_id 가 둘을 잇는다");
    }

    #[test]
    fn 수집_실패는_error_만_싣는다() {
        let t = Task {
            phase: TaskPhase::Report,
            req_id: 1,
            task_type: "probe".into(),
            params: None,
            result: None,
            error: Some("denied".into()),
        };
        let json = serde_json::to_string(&t).expect("ser");
        assert!(!json.contains("result"), "★정상이 아니면 result 자리를 비운다");
    }
}
