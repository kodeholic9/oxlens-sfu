// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-AFF-SERVER-SCOPE · 정:RULE-AFF-ONE-SET · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-FWD-ROOM · 연:AFF.op.AFFILIATION · 연:RULE-AFF-PROC · 연:RULE-AFF-RES-FORM · 연:AFF.S · model: claude-opus-5-5
//! 소속 — 입장 방 집합(= 구독 집합) 하나와 `pub_room`(RCU 값) 하나. 발행 RTP 는 매 패킷 이 값으로 방을 정한다.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use arc_swap::ArcSwapOption;
use oxsig::types::Affiliation;

use crate::fwd::hot::RoomHot;

pub struct Hot {
    pub user: Arc<str>,
    pub pub_room: ArcSwapOption<RoomHot>,
    pub rtp_notify_ms: AtomicU64,
}

impl Hot {
    pub fn new(user: &str) -> Hot {
        Hot { user: Arc::from(user), pub_room: ArcSwapOption::empty(), rtp_notify_ms: AtomicU64::new(0) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Reject,
    RejectPerm,
    Select,
    SelectMoved,
    Away,
    Same,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Reject => "AFF.S.reject",
            Step::RejectPerm => "AFF.S.reject_perm",
            Step::Select => "AFF.S.select",
            Step::SelectMoved => "AFF.S.select_moved",
            Step::Away => "AFF.S.away",
            Step::Same => "AFF.S.same",
        }
    }
}

pub fn decide(pub_room: Option<&str>, sel: Option<&str>, desel: Option<&str>, codec_bad: bool, perm_bad: bool) -> Step {
    if codec_bad {
        return Step::Reject;
    }
    if perm_bad {
        return Step::RejectPerm;
    }
    match (sel, pub_room) {
        (Some(_), None) => Step::Select,
        (Some(s), Some(p)) if s != p => Step::SelectMoved,
        (None, Some(p)) if desel == Some(p) => Step::Away,
        _ => Step::Same,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinStep {
    Select,
    SelectMoved,
    Listen,
}

impl JoinStep {
    pub fn label(self) -> &'static str {
        match self {
            JoinStep::Select => "AFF.S.join_select",
            JoinStep::SelectMoved => "AFF.S.join_select_moved",
            JoinStep::Listen => "AFF.S.join_listen",
        }
    }
}

pub fn decide_join(pub_room: Option<&str>, room: &str, select: bool) -> JoinStep {
    match (select, pub_room) {
        (true, None) => JoinStep::Select,
        (true, Some(p)) if p != room => JoinStep::SelectMoved,
        _ => JoinStep::Listen,
    }
}

pub fn affiliation(sub: &BTreeSet<String>, pub_room: Option<&str>) -> Affiliation {
    Affiliation { sub_rooms: sub.iter().cloned().collect(), pub_room: pub_room.map(str::to_string) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_matrix() {
        assert_eq!(decide(None, Some("r1"), None, false, false), Step::Select);
        assert_eq!(decide(Some("r1"), Some("r3"), Some("r1"), false, false), Step::SelectMoved);
        assert_eq!(decide(Some("r1"), None, Some("r1"), false, false), Step::Away);
        assert_eq!(decide(Some("r1"), None, Some("r9"), false, false), Step::Same);
        assert_eq!(decide(Some("r1"), Some("r1"), None, false, false), Step::Same);
        assert_eq!(decide(None, Some("r1"), None, true, false), Step::Reject);
        assert_eq!(decide_join(Some("r1"), "r2", true), JoinStep::SelectMoved);
        assert_eq!(decide_join(Some("r1"), "r1", true), JoinStep::Listen);
    }
}
