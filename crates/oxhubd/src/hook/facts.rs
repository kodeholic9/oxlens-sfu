// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-EVENTS · 정:RULE-HOOK-FACTS-SUB · model: claude-opus-5-5
//! 사실 짓기의 판단 — 순수 함수 + 사유(원천 `server/areas/hook/behavior/facts.ts` 의 거울 · 벡터 `hook.json` 이 맞댄다): 방 통지 가르기(재료 표지면 맡은 hub 만 짓고 · A 평면 통지면 배달하고 맡았으면 사실 하나 더) · A 평면 통지 → 카탈로그 이름 · 사실 id 도장(받은 차례로 커진다 · 벽시계 역행이면 앞 ms · 카운터 4 자리를 다 쓰면 ms + 1) · 사실 id 꼴 · 계정 `facts` 고르기 · 사실 봉투(주인 — node 넘기기 · 운반이 이것을 쓴다).

use common::envelope::{Material, Notice, OP_FACT_MATERIAL};
use oxsig::op::Op;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub t: u64,
    pub room_id: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Deliver(Option<&'static str>),
    Build(Material),
    NotOwner,
    Malformed,
}

impl Route {
    pub fn reason(&self) -> &'static str {
        match self {
            Route::Deliver(Some(_)) => "deliver and build",
            Route::Deliver(None) => "deliver",
            Route::Build(_) => "material",
            Route::NotOwner => "not owner",
            Route::Malformed => "material unreadable",
        }
    }
}

pub fn route(n: &Notice, owned: bool) -> Route {
    if n.op == OP_FACT_MATERIAL {
        if !owned {
            return Route::NotOwner;
        }
        return Material::of_notice(n).map_or(Route::Malformed, Route::Build);
    }
    Route::Deliver(if owned { notice_kind(n) } else { None })
}

pub fn of_notice(op: &str, sub: Option<&str>) -> Option<&'static str> {
    match (op, sub) {
        ("PARTICIPANT_EVENT", Some("joined")) => Some("participant_joined"),
        ("PARTICIPANT_EVENT", Some("left")) => Some("participant_left"),
        ("TRACK_EVENT", Some("add")) => Some("track_published"),
        ("TRACK_EVENT", Some("remove")) => Some("track_unpublished"),
        ("TRACK_STATE", Some("muted")) => Some("track_state"),
        ("PARTICIPANT_STATE", Some("permission")) => Some("permission_changed"),
        _ => None,
    }
}

pub fn notice_kind(n: &Notice) -> Option<&'static str> {
    let op = Op::from_u16(n.op)?;
    let field = match op {
        Op::TrackEvent => "action",
        Op::ParticipantEvent | Op::TrackState | Op::ParticipantState => "type",
        _ => return None,
    };
    of_notice(op.name(), n.body.get(field).and_then(|v| v.as_str()))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stamp {
    last: Option<(u64, u16)>,
}

impl Stamp {
    pub fn at(ms: u64, counter: u16) -> Stamp {
        Stamp { last: Some((ms, counter)) }
    }

    pub fn next(&mut self, now: u64) -> (u64, u16) {
        let out = match self.last {
            None => (now, 0),
            Some((ms, _)) if now > ms => (now, 0),
            Some((ms, c)) if c == u16::MAX => (ms + 1, 0),
            Some((ms, c)) => (ms, c + 1),
        };
        self.last = Some(out);
        out
    }
}

pub fn fact_id(ms: u64, counter: u16, inst: &str) -> String {
    format!("{ms:012x}{counter:04x}{inst}")
}

pub fn wants(facts: &[String], kind: &str) -> bool {
    facts.iter().any(|f| f == "*" || f == kind)
}
