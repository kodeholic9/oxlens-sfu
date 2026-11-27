// ★생성 파일 — spec/sdk/gen/binding.py 가 SDK 정의서 원천(areas/pub 의 입구 표 · 효과 표)에서 짓는다. 손으로 고치지 않는다.
// 위(연동규격서)가 지은 PUB 클라 상태기 Rust 판을 SDK 표면에 잇는다 — TS 판 결합과 같은 표 · 위가 효과를 바꾸면 아래 match 가 깨져 짓기가 멈춘다.
// 집행은 부르는 쪽이 plan() 의 목록대로 직접 한다 — 콜백을 주입하지 않는다.
#![allow(dead_code)]
use super::publish_client as F;

/// SDK 가 앱에 내는 이벤트(효과 표의 app.* 행)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PubEvent { DuplexApplied, DuplexFailed, Ended, PublishFailed, Published, }
impl PubEvent {
    /// 표면 이름(SDK 정의서 효과 표 그대로)
    pub fn as_str(self) -> &'static str {
        match self { PubEvent::DuplexApplied => "duplexApplied", PubEvent::DuplexFailed => "duplexFailed", PubEvent::Ended => "ended", PubEvent::PublishFailed => "publishFailed", PubEvent::Published => "published", }
    }
}

/// SDK 가 하는 동작(효과 표의 그 밖 행)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PubAction { GateClose, GateOpen, TransceiverInactive, }
impl PubAction {
    /// 표면 이름(SDK 정의서 효과 표 그대로)
    pub fn as_str(self) -> &'static str {
        match self { PubAction::GateClose => "gate.close", PubAction::GateOpen => "gate.open", PubAction::TransceiverInactive => "transceiver.inactive", }
    }
}

/// 위 입구 중 SDK 가 넣는 것(입구 표) — 위 입구 이름 그대로(Rust 판 함수는 snake_case)
pub const FED: &[&str] = &["on", "nego", "pubRes", "attach", "off", "revoked", "reregister", "floorOpen", "floorClose", "resumeCheck", "dropServer", "roomGone", "moveOut", "moveIn", "setDuplex", "reqExpired", "setDuplexRes"];
/// 위 입구 중 넣지 않는 것(입구 표 셋째 칸 · 판정표에 사유)
pub const NOT_FED: &[&str] = &[];

/// 위 효과 하나의 갈 곳 — 이벤트 · 동작 · (위 이음매면) 받는 영역
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    pub event: Option<PubEvent>,
    pub action: Option<PubAction>,
    pub seam: Option<&'static str>,
}
/// 위 효과 하나 → SDK 가 낼 이벤트 · 할 동작(효과 표) — 위 효과가 늘거나 줄면 여기서 깨진다
pub fn effect(f: F::Fx) -> Effect {
    match f {
        F::Fx::app_publishing => Effect { event: Some(PubEvent::Published), action: None, seam: None },
        F::Fx::app_publish_failed => Effect { event: Some(PubEvent::PublishFailed), action: None, seam: None },
        F::Fx::app_revoked => Effect { event: Some(PubEvent::Ended), action: None, seam: None },
        F::Fx::app_duplex => Effect { event: Some(PubEvent::DuplexApplied), action: None, seam: None },
        F::Fx::app_duplex_failed => Effect { event: Some(PubEvent::DuplexFailed), action: None, seam: None },
        F::Fx::track_attach => Effect { event: None, action: Some(PubAction::GateOpen), seam: None },
        F::Fx::track_detach => Effect { event: None, action: Some(PubAction::GateClose), seam: None },
        F::Fx::transceiver_inactive => Effect { event: None, action: Some(PubAction::TransceiverInactive), seam: None },
    }
}

/// 한 걸음의 집행 목록 — 보낼 것 · 걸 타이머 · 이음매(받는 곳은 위가 정한다)는 위가 정한 그대로, 이벤트 · 동작은 효과 표대로
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub state: F::St,
    pub label: &'static str,
    pub send: Vec<F::Tx>,
    pub seams: Vec<F::Seam>,
    pub timers: Vec<F::TimerName>,
    pub events: Vec<PubEvent>,
    pub actions: Vec<PubAction>,
}
pub fn plan(step: F::Step) -> Plan {
    let mut events = Vec::new();
    let mut actions = Vec::new();
    for f in &step.fx {
        let e = effect(*f);
        events.extend(e.event);
        actions.extend(e.action);
    }
    Plan { state: step.s, label: step.label, send: step.out, seams: step.seams, timers: step.start, events, actions }
}
