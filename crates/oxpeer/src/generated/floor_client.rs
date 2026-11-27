// ★생성 파일 — gen/floor_client.mts 가 원천(context/spec/protocol · areas/flr/behavior/client.ts)에서 짓는다. 손으로 고치지 않는다.
// IR sha256 207c0df6ea211dc9 · 3GPP TS 24.380 §6.2.4 floor participant · Rust 판 — 같은 생성기의 TS 판과 한 뿌리(같은 입구 · 같은 칸 · 같은 라벨).
// 집행은 부르는 쪽 몫이다 — out(DC 로 보낼 것) · ack(ACK 으로 답할 것) · fx(마이크 · 화면 · 앱 알림) · start(다시 걸 타이머). armed() 밖의 타이머는 멈춘다.
#![allow(dead_code, unused_variables, unused_mut, unused_parens, non_camel_case_types, clippy::all)]

/// 클라 상태(6.2.4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CState {
    NOPERM,
    PREQ,
    PERM,
    PREL,
    QUEUED,
}
impl CState {
    pub const ALL: &'static [CState] = &[CState::NOPERM, CState::PREQ, CState::PERM, CState::PREL, CState::QUEUED];
    pub fn as_str(self) -> &'static str {
        match self { CState::NOPERM => "NOPERM", CState::PREQ => "PREQ", CState::PERM => "PERM", CState::PREL => "PREL", CState::QUEUED => "QUEUED", }
    }
    pub fn parse(s: &str) -> Option<CState> {
        match s { "NOPERM" => Some(CState::NOPERM), "PREQ" => Some(CState::PREQ), "PERM" => Some(CState::PERM), "PREL" => Some(CState::PREL), "QUEUED" => Some(CState::QUEUED), _ => None }
    }
}

/// queued 세부
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CSub {
    WAIT,
    QUERY,
    GRANTED,
}
impl CSub {
    pub const ALL: &'static [CSub] = &[CSub::WAIT, CSub::QUERY, CSub::GRANTED];
    pub fn as_str(self) -> &'static str {
        match self { CSub::WAIT => "WAIT", CSub::QUERY => "QUERY", CSub::GRANTED => "GRANTED", }
    }
    pub fn parse(s: &str) -> Option<CSub> {
        match s { "WAIT" => Some(CSub::WAIT), "QUERY" => Some(CSub::QUERY), "GRANTED" => Some(CSub::GRANTED), _ => None }
    }
}

/// 효과(마이크 · 화면 · 앱 알림)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fx {
    #[cfg_attr(feature = "serde", serde(rename = "mic.close"))] mic_close,
    #[cfg_attr(feature = "serde", serde(rename = "mic.open"))] mic_open,
    #[cfg_attr(feature = "serde", serde(rename = "media.render"))] media_render,
    #[cfg_attr(feature = "serde", serde(rename = "app.speaker_none"))] app_speaker_none,
    #[cfg_attr(feature = "serde", serde(rename = "app.floor_unreliable"))] app_floor_unreliable,
    #[cfg_attr(feature = "serde", serde(rename = "app.floor_reliable"))] app_floor_reliable,
    #[cfg_attr(feature = "serde", serde(rename = "app.speaker"))] app_speaker,
    #[cfg_attr(feature = "serde", serde(rename = "app.granted"))] app_granted,
    #[cfg_attr(feature = "serde", serde(rename = "app.denied"))] app_denied,
    #[cfg_attr(feature = "serde", serde(rename = "app.queued"))] app_queued,
    #[cfg_attr(feature = "serde", serde(rename = "app.revoked"))] app_revoked,
    #[cfg_attr(feature = "serde", serde(rename = "app.granted_queued"))] app_granted_queued,
    #[cfg_attr(feature = "serde", serde(rename = "app.request_timeout"))] app_request_timeout,
    #[cfg_attr(feature = "serde", serde(rename = "app.queue_timeout"))] app_queue_timeout,
    #[cfg_attr(feature = "serde", serde(rename = "app.grant_expired"))] app_grant_expired,
}
impl Fx {
    pub const ALL: &'static [Fx] = &[Fx::mic_close, Fx::mic_open, Fx::media_render, Fx::app_speaker_none, Fx::app_floor_unreliable, Fx::app_floor_reliable, Fx::app_speaker, Fx::app_granted, Fx::app_denied, Fx::app_queued, Fx::app_revoked, Fx::app_granted_queued, Fx::app_request_timeout, Fx::app_queue_timeout, Fx::app_grant_expired];
    pub fn as_str(self) -> &'static str {
        match self { Fx::mic_close => "mic.close", Fx::mic_open => "mic.open", Fx::media_render => "media.render", Fx::app_speaker_none => "app.speaker_none", Fx::app_floor_unreliable => "app.floor_unreliable", Fx::app_floor_reliable => "app.floor_reliable", Fx::app_speaker => "app.speaker", Fx::app_granted => "app.granted", Fx::app_denied => "app.denied", Fx::app_queued => "app.queued", Fx::app_revoked => "app.revoked", Fx::app_granted_queued => "app.granted_queued", Fx::app_request_timeout => "app.request_timeout", Fx::app_queue_timeout => "app.queue_timeout", Fx::app_grant_expired => "app.grant_expired", }
    }
    pub fn parse(s: &str) -> Option<Fx> {
        match s { "mic.close" => Some(Fx::mic_close), "mic.open" => Some(Fx::mic_open), "media.render" => Some(Fx::media_render), "app.speaker_none" => Some(Fx::app_speaker_none), "app.floor_unreliable" => Some(Fx::app_floor_unreliable), "app.floor_reliable" => Some(Fx::app_floor_reliable), "app.speaker" => Some(Fx::app_speaker), "app.granted" => Some(Fx::app_granted), "app.denied" => Some(Fx::app_denied), "app.queued" => Some(Fx::app_queued), "app.revoked" => Some(Fx::app_revoked), "app.granted_queued" => Some(Fx::app_granted_queued), "app.request_timeout" => Some(Fx::app_request_timeout), "app.queue_timeout" => Some(Fx::app_queue_timeout), "app.grant_expired" => Some(Fx::app_grant_expired), _ => None }
    }
}

/// 타이머
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimerName {
    T101,
    T100,
    T104,
    T132,
}
impl TimerName {
    pub const ALL: &'static [TimerName] = &[TimerName::T101, TimerName::T100, TimerName::T104, TimerName::T132];
    pub fn as_str(self) -> &'static str {
        match self { TimerName::T101 => "T101", TimerName::T100 => "T100", TimerName::T104 => "T104", TimerName::T132 => "T132", }
    }
    pub fn parse(s: &str) -> Option<TimerName> {
        match s { "T101" => Some(TimerName::T101), "T100" => Some(TimerName::T100), "T104" => Some(TimerName::T104), "T132" => Some(TimerName::T132), _ => None }
    }
}

/// ACK 으로 답하는 메시지
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AckOf {
    TAKEN,
    GRANTED,
    DENY,
}
impl AckOf {
    pub const ALL: &'static [AckOf] = &[AckOf::TAKEN, AckOf::GRANTED, AckOf::DENY];
    pub fn as_str(self) -> &'static str {
        match self { AckOf::TAKEN => "TAKEN", AckOf::GRANTED => "GRANTED", AckOf::DENY => "DENY", }
    }
    pub fn parse(s: &str) -> Option<AckOf> {
        match s { "TAKEN" => Some(AckOf::TAKEN), "GRANTED" => Some(AckOf::GRANTED), "DENY" => Some(AckOf::DENY), _ => None }
    }
}

/// 클라 상태 — 원천 Cl(areas/flr/model/state.ts)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cl {
    /// 클라 상태(6.2.4)
    pub cs: CState,
    /// queued 세부
    pub sub: CSub,
    /// 사용자가 말하려 한다
    pub want: bool,
    /// T101(REQUEST 재전송) 횟수
    pub c101: i64,
    /// T100(RELEASE 재전송) 횟수
    pub c100: i64,
    /// T104(순번 질의 재전송) 횟수
    pub c104: i64,
    /// 누름 횟수 — 검사 상한(MaxPress)용
    pub presses: i64,
    /// 요청에 실은 우선순위
    pub prio: i64,
    /// 허가된 시간(초)
    pub grant: i64,
    /// 허가된 우선순위 — 받은 GRANTED 의 값(연:RULE-FLR-PROC-GRANTED 1 · 연:RULE-FLR-PROC-PERM-GRANTED 2)
    pub gprio: i64,
    /// 이 방이 발행 방인가(전환 응답을 받았다)
    #[cfg_attr(feature = "serde", serde(rename = "isPub"))]
    pub is_pub: bool,
    /// 방을 나갔다
    pub left: bool,
    /// 화자 표시 — 화면에 보이는 화자(내가 말하는 중이면 없음)
    pub shown: Option<String>,
}
pub fn initial() -> Cl {
    Cl { cs: CState::NOPERM, sub: CSub::WAIT, want: false, c101: 0, c100: 0, c104: 0, presses: 0, prio: 0, grant: 0, gprio: 0, is_pub: false, left: false, shown: None }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxTaken {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub speaker: String,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub sources: Option<Vec<i64>>,
    pub seq: i64,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub talk: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxIdle {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub prev: Option<String>,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxRevoke {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub cause: i64,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxGranted {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub priority: i64,
    pub duration: i64,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub talk: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxDeny {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub cause: i64,
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxQinfoInfo {
    pub position: i64,
    pub priority: i64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxQinfo {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub info: RxQinfoInfo,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RxAck {
    /// 머리의 A 비트 — 서야 ACK 으로 답한다
    #[cfg_attr(feature = "serde", serde(default))]
    pub ack: bool,
    pub of: i64,
}

/// 서버 → 클라 메시지(wire 이름 · 방 칸은 입구가 거른다)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "t"))]
pub enum Rx {
    #[cfg_attr(feature = "serde", serde(rename = "TAKEN"))]
    Taken(RxTaken),
    #[cfg_attr(feature = "serde", serde(rename = "IDLE"))]
    Idle(RxIdle),
    #[cfg_attr(feature = "serde", serde(rename = "REVOKE"))]
    Revoke(RxRevoke),
    #[cfg_attr(feature = "serde", serde(rename = "GRANTED"))]
    Granted(RxGranted),
    #[cfg_attr(feature = "serde", serde(rename = "DENY"))]
    Deny(RxDeny),
    #[cfg_attr(feature = "serde", serde(rename = "QINFO"))]
    Qinfo(RxQinfo),
    #[cfg_attr(feature = "serde", serde(rename = "ACK"))]
    Ack(RxAck),
}
/// 클라 → 서버 메시지(이 상태기가 내는 것)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Tx {
    Request { #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))] priority: Option<i64> },
    Release,
    QposReq,
}
impl Tx {
    pub fn op(&self) -> &'static str {
        match self {
            Tx::Request { .. } => "REQUEST",
            Tx::Release => "RELEASE",
            Tx::QposReq => "QPOS_REQ",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ack {
    pub of: AckOf,
}

/// 한 걸음의 결과 — label 은 원천의 동작 라벨(연동규격서 FLR.C.…)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Step {
    pub s: Cl,
    pub label: &'static str,
    pub out: Vec<Tx>,
    pub ack: Vec<Ack>,
    pub fx: Vec<Fx>,
    pub start: Vec<TimerName>,
}

/// 원장 — 타이머(ms)
pub fn timer_ms(t: TimerName) -> i64 {
    match t { TimerName::T101 => 500, TimerName::T100 => 500, TimerName::T104 => 500, TimerName::T132 => 2000, }
}
/// 원장 — 재전송 횟수
#[allow(non_upper_case_globals)]
pub mod counts {
    pub const C101: i64 = 3;
    pub const C100: i64 = 3;
    pub const C104: i64 = 3;
}

fn done(prev: &Cl, s: Cl, label: &'static str, out: Vec<Tx>, ack: Vec<Ack>, fx: Vec<Fx>, mut start: Vec<TimerName>) -> Step {
    let before = armed(prev);
    for t in armed(&s) {
        if !before.contains(&t) && !start.contains(&t) {
            start.push(t);
        }
    }
    Step { s, label, out, ack, fx, start }
}

/// 지금 돌아야 하는 타이머 — 원천 timer(…).armed
pub fn armed(s: &Cl) -> Vec<TimerName> {
    let mut on = Vec::new();
    if s.cs == CState::PREQ {
        on.push(TimerName::T101);
    }
    if s.cs == CState::PREL {
        on.push(TimerName::T100);
    }
    if s.cs == CState::QUEUED && s.sub == CSub::QUERY {
        on.push(TimerName::T104);
    }
    if s.cs == CState::QUEUED && s.sub == CSub::GRANTED {
        on.push(TimerName::T132);
    }
    on
}

/// 원천 절차 → 입구 이름 — 다른 영역의 이음매가 부르는 입구를 찾을 때(위 IR seams 의 react 는 절차 이름이다)
pub const PROC_ENTRY: &[(&str, &str)] = &[("CPress", "press"), ("CRelease", "release"), ("CIntent", "intent"), ("CQpos", "queue_position"), ("CRtpTx", "rtp_tx"), ("CRtpRx", "rtp_rx"), ("CPubSel", "pub_select"), ("CAway", "pub_away"), ("CLeave", "leave"), ("CJoin", "join"), ("CDown", "down"), ("CDcDown", "dc_down"), ("CDcUp", "dc_up")];

/// 누름
pub fn press(s: &Cl, p: i64) -> Step {
    if s.cs == CState::NOPERM {
        return done(s, Cl { cs: CState::PREQ, want: true, presses: s.presses + 1, prio: p, c101: 1, ..s.clone() }, "FLR.C.NOPERM.press", vec![Tx::Request { priority: Some(p) }], vec![], vec![], vec![TimerName::T101]);
    }
    done(s, s.clone(), "FLR.C.any.press_drop", vec![], vec![], vec![], vec![])
}

/// 뗌 — 받는 상태 밖이면 None(사건이 없던 것)
pub fn release(s: &Cl) -> Option<Step> {
    if !(s.cs == CState::PREQ || s.cs == CState::PERM || s.cs == CState::QUEUED) {
        return None;
    }
    if s.cs == CState::PREQ {
        return Some(done(s, Cl { cs: CState::PREL, want: false, c100: 1, ..s.clone() }, "FLR.C.PREQ.release", vec![Tx::Release], vec![], vec![], vec![TimerName::T100]));
    }
    if s.cs == CState::PERM {
        return Some(done(s, Cl { cs: CState::PREL, want: false, c100: 1, ..s.clone() }, "FLR.C.PERM.release", vec![Tx::Release], vec![], vec![Fx::mic_close], vec![TimerName::T100]));
    }
    Some(done(s, Cl { cs: CState::PREL, want: false, c100: 1, ..s.clone() }, "FLR.C.QUEUED.release", vec![Tx::Release], vec![], vec![], vec![TimerName::T100]))
}

/// 승계 허가 뒤 발언 의지 표시 — 받는 상태 밖이면 None(사건이 없던 것)
pub fn intent(s: &Cl) -> Option<Step> {
    if !(s.cs == CState::QUEUED && s.sub == CSub::GRANTED) {
        return None;
    }
    Some(done(s, Cl { cs: CState::PERM, shown: None, ..s.clone() }, "FLR.C.QUEUED.intent", vec![], vec![], vec![Fx::mic_open], vec![]))
}

/// 앱이 순번을 묻는다 — 받는 상태 밖이면 None(사건이 없던 것)
pub fn queue_position(s: &Cl) -> Option<Step> {
    if !(s.cs == CState::QUEUED && s.sub == CSub::WAIT) {
        return None;
    }
    Some(done(s, Cl { sub: CSub::QUERY, c104: 1, ..s.clone() }, "FLR.C.QUEUED.qpos", vec![Tx::QposReq], vec![], vec![], vec![TimerName::T104]))
}

/// 보낸다
pub fn rtp_tx(s: &Cl) -> Step {
    done(s, s.clone(), "FLR.C.PERM.rtp_tx", vec![], vec![], vec![], vec![])
}

/// 남의 목소리가 닿는다
pub fn rtp_rx(s: &Cl) -> Step {
    if s.cs == CState::NOPERM {
        return done(s, s.clone(), "FLR.C.NOPERM.rtp_rx", vec![], vec![], vec![Fx::media_render], vec![]);
    }
    if s.cs == CState::PREQ {
        return done(s, s.clone(), "FLR.C.PREQ.rtp_rx", vec![], vec![], vec![Fx::media_render], vec![]);
    }
    if s.cs == CState::PERM {
        return done(s, s.clone(), "FLR.C.PERM.rtp_rx", vec![], vec![], vec![Fx::media_render], vec![]);
    }
    if s.cs == CState::PREL {
        return done(s, s.clone(), "FLR.C.PREL.rtp_rx", vec![], vec![], vec![Fx::media_render], vec![]);
    }
    done(s, s.clone(), "FLR.C.QUEUED.rtp_rx", vec![], vec![], vec![Fx::media_render], vec![])
}

/// WS AFFILIATION{pub_select}
pub fn pub_select(s: &Cl) -> Step {
    done(s, Cl { is_pub: true, ..s.clone() }, "FLR.C.any.pub_select", vec![], vec![], vec![], vec![])
}

/// 발행 방을 떠난다
pub fn pub_away(s: &Cl) -> Step {
    done(s, Cl { cs: CState::NOPERM, want: false, is_pub: false, ..s.clone() }, "FLR.C.any.pub_away", vec![], vec![], vec![Fx::mic_close], vec![])
}

/// WS ROOM_LEAVE — 방을 떠난다
pub fn leave(s: &Cl) -> Step {
    done(s, Cl { cs: CState::NOPERM, want: false, is_pub: false, shown: None, left: true, ..s.clone() }, "FLR.C.any.leave", vec![], vec![], vec![Fx::mic_close], vec![])
}

/// WS ROOM_JOIN — 방에 든다
pub fn join(s: &Cl) -> Step {
    done(s, Cl { sub: CSub::WAIT, want: false, prio: 0, grant: 0, gprio: 0, is_pub: false, shown: None, left: false, ..s.clone() }, "FLR.C.any.join", vec![], vec![], vec![], vec![])
}

/// 재구축으로 그 방 발언권을 내린다(세션 전체 · 그 서버)
pub fn down(s: &Cl) -> Step {
    done(s, Cl { cs: CState::NOPERM, want: false, is_pub: false, shown: None, ..s.clone() }, "FLR.C.any.down", vec![], vec![], vec![Fx::app_speaker_none, Fx::mic_close], vec![])
}

/// DC 만 끊김
pub fn dc_down(s: &Cl) -> Step {
    done(s, s.clone(), "FLR.C.any.dc_down", vec![], vec![], vec![Fx::app_floor_unreliable], vec![])
}

/// DC 다시 열림
pub fn dc_up(s: &Cl) -> Step {
    done(s, s.clone(), "FLR.C.any.dc_reopen", vec![], vec![], vec![Fx::app_floor_reliable], vec![])
}

/// 타이머 만료 — 돌고 있지 않은 타이머면 None(부르는 쪽이 멈췄어야 한다)
pub fn expire(s: &Cl, t: TimerName) -> Option<Step> {
    if !armed(s).contains(&t) {
        return None;
    }
    match t {
        TimerName::T101 => {
            if s.c101 < counts::C101 {
                return Some(done(s, Cl { c101: s.c101 + 1, ..s.clone() }, "FLR.C.PREQ.T101", vec![Tx::Request { priority: Some(s.prio) }], vec![], vec![], vec![]));
            }
            Some(done(s, Cl { cs: CState::NOPERM, ..s.clone() }, "FLR.C.PREQ.T101_exhaust", vec![], vec![], vec![Fx::app_request_timeout], vec![]))
        }
        TimerName::T100 => {
            if s.c100 < counts::C100 {
                return Some(done(s, Cl { c100: s.c100 + 1, ..s.clone() }, "FLR.C.PREL.T100", vec![Tx::Release], vec![], vec![], vec![]));
            }
            Some(done(s, Cl { cs: CState::NOPERM, ..s.clone() }, "FLR.C.PREL.T100_exhaust", vec![], vec![], vec![], vec![]))
        }
        TimerName::T104 => {
            if s.c104 < counts::C104 {
                return Some(done(s, Cl { c104: s.c104 + 1, ..s.clone() }, "FLR.C.QUEUED.T104", vec![Tx::QposReq], vec![], vec![], vec![]));
            }
            Some(done(s, Cl { cs: CState::PREL, c100: 1, ..s.clone() }, "FLR.C.QUEUED.T104_exhaust", vec![Tx::Release], vec![], vec![Fx::app_queue_timeout], vec![TimerName::T100]))
        }
        TimerName::T132 => {
            Some(done(s, Cl { cs: CState::NOPERM, want: false, ..s.clone() }, "FLR.C.QUEUED.T132", vec![Tx::Release], vec![], vec![Fx::app_grant_expired], vec![]))
        }
    }
}

/// 서버 → 클라 사건 — 상태 × 사건 표(원문 6.2.4) · 표에 없는 칸은 other(6.2.4.1)
pub fn receive(s: &Cl, msg: &Rx) -> Step {
    match s.cs {
        CState::NOPERM => match msg {
            Rx::Taken(m) => {
                done(s, Cl { shown: Some(m.speaker.clone()), ..s.clone() }, "FLR.C.NOPERM.rx_TAKEN", vec![], if m.ack { vec![Ack { of: AckOf::TAKEN }] } else { vec![] }, vec![Fx::app_speaker], vec![])
            }
            Rx::Idle(m) => {
                done(s, Cl { shown: None, ..s.clone() }, "FLR.C.NOPERM.rx_IDLE", vec![], vec![], vec![Fx::app_speaker_none], vec![])
            }
            Rx::Revoke(m) => {
                if m.cause == 3 {
                    return done(s, s.clone(), "FLR.C.NOPERM.rx_REVOKE_release", vec![Tx::Release], vec![], vec![], vec![]);
                }
                done(s, s.clone(), "FLR.C.NOPERM.rx_REVOKE_other", vec![], vec![], vec![], vec![])
            }
            _ => {
                done(s, s.clone(), "FLR.C.NOPERM.rx_other", vec![], vec![], vec![], vec![])
            }
        },
        CState::PREQ => match msg {
            Rx::Granted(m) => {
                done(s, Cl { cs: CState::PERM, grant: m.duration, gprio: m.priority, shown: None, ..s.clone() }, "FLR.C.PREQ.rx_GRANTED", vec![], if m.ack { vec![Ack { of: AckOf::GRANTED }] } else { vec![] }, vec![Fx::app_granted, Fx::app_speaker_none, Fx::mic_open], vec![])
            }
            Rx::Deny(m) => {
                done(s, Cl { cs: CState::NOPERM, want: false, ..s.clone() }, "FLR.C.PREQ.rx_DENY", vec![], if m.ack { vec![Ack { of: AckOf::DENY }] } else { vec![] }, vec![Fx::app_denied], vec![])
            }
            Rx::Qinfo(m) => {
                done(s, Cl { cs: CState::QUEUED, sub: CSub::WAIT, ..s.clone() }, "FLR.C.PREQ.rx_QINFO", vec![], vec![], vec![Fx::app_queued], vec![])
            }
            Rx::Taken(m) => {
                done(s, Cl { shown: Some(m.speaker.clone()), ..s.clone() }, "FLR.C.PREQ.rx_TAKEN", vec![], if m.ack { vec![Ack { of: AckOf::TAKEN }] } else { vec![] }, vec![Fx::app_speaker], vec![])
            }
            Rx::Idle(m) => {
                done(s, Cl { shown: None, ..s.clone() }, "FLR.C.PREQ.rx_IDLE", vec![], vec![], vec![Fx::app_speaker_none], vec![])
            }
            _ => {
                done(s, s.clone(), "FLR.C.PREQ.rx_other", vec![], vec![], vec![], vec![])
            }
        },
        CState::PERM => match msg {
            Rx::Revoke(m) => {
                done(s, Cl { cs: CState::PREL, want: false, c100: 1, ..s.clone() }, "FLR.C.PERM.rx_REVOKE", vec![Tx::Release], vec![], vec![Fx::mic_close, Fx::app_revoked], vec![TimerName::T100])
            }
            Rx::Granted(m) => {
                done(s, Cl { grant: m.duration, gprio: m.priority, ..s.clone() }, "FLR.C.PERM.rx_GRANTED", vec![], if m.ack { vec![Ack { of: AckOf::GRANTED }] } else { vec![] }, vec![Fx::app_granted], vec![])
            }
            Rx::Idle(m) => {
                done(s, s.clone(), "FLR.C.PERM.rx_IDLE", vec![], vec![], vec![], vec![])
            }
            Rx::Taken(m) => {
                done(s, s.clone(), "FLR.C.PERM.rx_TAKEN", vec![], vec![], vec![], vec![])
            }
            _ => {
                done(s, s.clone(), "FLR.C.PERM.rx_other", vec![], vec![], vec![], vec![])
            }
        },
        CState::PREL => match msg {
            Rx::Idle(m) => {
                done(s, Cl { cs: CState::NOPERM, shown: None, ..s.clone() }, "FLR.C.PREL.rx_IDLE", vec![], vec![], vec![Fx::app_speaker_none], vec![])
            }
            Rx::Ack(m) => {
                if m.of == 4 {
                    return done(s, Cl { cs: CState::NOPERM, ..s.clone() }, "FLR.C.PREL.rx_ACK_release", vec![], vec![], vec![], vec![]);
                }
                done(s, s.clone(), "FLR.C.PREL.rx_ACK_other", vec![], vec![], vec![], vec![])
            }
            Rx::Taken(m) => {
                done(s, Cl { cs: CState::NOPERM, shown: Some(m.speaker.clone()), ..s.clone() }, "FLR.C.PREL.rx_TAKEN", vec![], if m.ack { vec![Ack { of: AckOf::TAKEN }] } else { vec![] }, vec![Fx::app_speaker], vec![])
            }
            Rx::Revoke(m) => {
                done(s, s.clone(), "FLR.C.PREL.rx_REVOKE", vec![], vec![], vec![Fx::app_revoked], vec![])
            }
            Rx::Granted(m) => {
                done(s, Cl { shown: None, ..s.clone() }, "FLR.C.PREL.rx_GRANTED", vec![], if m.ack { vec![Ack { of: AckOf::GRANTED }] } else { vec![] }, vec![Fx::app_speaker_none], vec![])
            }
            _ => {
                done(s, s.clone(), "FLR.C.PREL.rx_other", vec![], vec![], vec![], vec![])
            }
        },
        CState::QUEUED => match msg {
            Rx::Taken(m) => {
                done(s, Cl { shown: Some(m.speaker.clone()), ..s.clone() }, "FLR.C.QUEUED.rx_TAKEN", vec![], if m.ack { vec![Ack { of: AckOf::TAKEN }] } else { vec![] }, vec![Fx::app_speaker], vec![])
            }
            Rx::Granted(m) => {
                if s.sub != CSub::GRANTED {
                    return done(s, Cl { sub: CSub::GRANTED, grant: m.duration, gprio: m.priority, shown: None, ..s.clone() }, "FLR.C.QUEUED.rx_GRANTED", vec![], if m.ack { vec![Ack { of: AckOf::GRANTED }] } else { vec![] }, vec![Fx::app_granted_queued, Fx::app_speaker_none], vec![]);
                }
                done(s, s.clone(), "FLR.C.QUEUED.rx_GRANTED_again", vec![], if m.ack { vec![Ack { of: AckOf::GRANTED }] } else { vec![] }, vec![], vec![])
            }
            Rx::Deny(m) => {
                done(s, Cl { cs: CState::NOPERM, want: false, ..s.clone() }, "FLR.C.QUEUED.rx_DENY", vec![], if m.ack { vec![Ack { of: AckOf::DENY }] } else { vec![] }, vec![Fx::app_denied], vec![])
            }
            Rx::Idle(m) => {
                done(s, Cl { cs: CState::NOPERM, shown: None, ..s.clone() }, "FLR.C.QUEUED.rx_IDLE", vec![], vec![], vec![Fx::app_speaker_none], vec![])
            }
            Rx::Qinfo(m) => {
                if s.sub == CSub::QUERY {
                    return done(s, Cl { sub: CSub::WAIT, ..s.clone() }, "FLR.C.QUEUED.rx_QINFO_answer", vec![], vec![], vec![Fx::app_queued], vec![]);
                }
                done(s, s.clone(), "FLR.C.QUEUED.rx_QINFO", vec![], vec![], vec![Fx::app_queued], vec![])
            }
            _ => {
                done(s, s.clone(), "FLR.C.QUEUED.rx_other", vec![], vec![], vec![], vec![])
            }
        },
    }
}

/// MBCP 메시지(종류 · A 비트 · TLV) → (방, 받은 메시지) — 방(관문 칸)이 없거나 · 필수 칸이 없거나 · 클라가 받지 않는 종류면 None(받는 쪽은 조용히 버린다)
pub fn rx_from_tlv(msg_type: u8, ack: bool, tlvs: &[(u8, Vec<u8>)]) -> Option<(String, Rx)> {
    let get = |id: u8| tlvs.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_slice());
    match msg_type {
        2 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let speaker = get(4).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let sources = get(31).and_then(|v| Some(v.chunks_exact(4).map(|c| i64::from(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))).collect::<Vec<i64>>()));
            let seq = get(8).and_then(|v| (if v.len() >= 2 { Some(i64::from(u16::from_be_bytes([v[0], v[1]]))) } else { None }))?;
            let talk = get(32).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()));
            Some((room, Rx::Taken(RxTaken { ack, speaker, sources, seq, talk })))
        }
        5 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let prev = get(26).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()));
            let seq = get(8).and_then(|v| (if v.len() >= 2 { Some(i64::from(u16::from_be_bytes([v[0], v[1]]))) } else { None }))?;
            Some((room, Rx::Idle(RxIdle { ack, prev, seq })))
        }
        6 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let cause = get(2).and_then(|v| v.first().map(|x| i64::from(*x)))?;
            let reason = get(27).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()));
            Some((room, Rx::Revoke(RxRevoke { ack, cause, reason })))
        }
        1 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let priority = get(0).and_then(|v| (if v.len() == 2 { Some(i64::from(v[0])) } else { None }))?;
            let duration = get(1).and_then(|v| (if v.len() >= 2 { Some(i64::from(u16::from_be_bytes([v[0], v[1]]))) } else { None }))?;
            let talk = get(32).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()));
            Some((room, Rx::Granted(RxGranted { ack, priority, duration, talk })))
        }
        3 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let cause = get(2).and_then(|v| v.first().map(|x| i64::from(*x)))?;
            let reason = get(27).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()));
            Some((room, Rx::Deny(RxDeny { ack, cause, reason })))
        }
        9 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let info = get(3).and_then(|v| (if v.len() >= 2 { Some(RxQinfoInfo { position: i64::from(v[0]), priority: i64::from(v[1]) }) } else { None }))?;
            Some((room, Rx::Qinfo(RxQinfo { ack, info })))
        }
        10 => {
            let room = get(29).and_then(|v| Some(String::from_utf8_lossy(v).into_owned()))?;
            let of = get(12).and_then(|v| (if v.len() == 2 { Some(i64::from(v[0])) } else { None }))?;
            Some((room, Rx::Ack(RxAck { ack, of })))
        }
        _ => None,
    }
}

/// 보낼 메시지 → MBCP(종류 · A 비트 · TLV) — 방(관문 칸)을 싣는다 · 필수 칸 차례대로 · 선택 칸은 있을 때만 뒤에
pub fn tx_to_tlv(tx: &Tx, room: &str) -> (u8, bool, Vec<(u8, Vec<u8>)>) {
    match tx {
        Tx::Request { priority } => {
            let mut t = vec![(29, room.as_bytes().to_vec())];
            if let Some(x) = priority {
                t.push((0, vec![*x as u8, 0]));
            }
            (0, false, t)
        }
        Tx::Release => {
            let t = vec![(29, room.as_bytes().to_vec())];
            (4, true, t)
        }
        Tx::QposReq => {
            let t = vec![(29, room.as_bytes().to_vec())];
            (8, false, t)
        }
    }
}

/// ACK 으로 답한다 → MBCP — of 의 종류 번호 · 방
pub fn ack_to_tlv(a: &Ack, room: &str) -> (u8, bool, Vec<(u8, Vec<u8>)>) {
    let of: i64 = match a.of { AckOf::TAKEN => 2, AckOf::GRANTED => 1, AckOf::DENY => 3, };
    (10, false, vec![(12, vec![of as u8, 0]), (29, room.as_bytes().to_vec())])
}

/// 벡터 시험 — TS 판 입구 이름과 JSON 인자로 부른다(oxpeer 1층 · TS 판 재생이 찾은 호출을 Rust 판에 넣어 맞댄다)
#[cfg(feature = "serde")]
pub fn call_json(entry: &str, s: &Cl, a: &[serde_json::Value]) -> Result<Option<Step>, String> {
    fn arg<T: serde::de::DeserializeOwned>(a: &[serde_json::Value], i: usize) -> Result<T, String> {
        serde_json::from_value(a.get(i).cloned().unwrap_or(serde_json::Value::Null)).map_err(|e| format!("인자 {i}: {e}"))
    }
    Ok(match entry {
        "press" => Some(press(s, arg::<i64>(a, 0)?)),
        "release" => release(s),
        "intent" => intent(s),
        "queuePosition" => queue_position(s),
        "rtpTx" => Some(rtp_tx(s)),
        "rtpRx" => Some(rtp_rx(s)),
        "pubSelect" => Some(pub_select(s)),
        "pubAway" => Some(pub_away(s)),
        "leave" => Some(leave(s)),
        "join" => Some(join(s)),
        "down" => Some(down(s)),
        "dcDown" => Some(dc_down(s)),
        "dcUp" => Some(dc_up(s)),
        "expire" => expire(s, arg::<TimerName>(a, 0)?),
        "receive" => Some(receive(s, &arg::<Rx>(a, 0)?)),
        _ => return Err(format!("모르는 입구 {entry}")),
    })
}
