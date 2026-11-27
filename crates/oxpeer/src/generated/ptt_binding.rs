// ★생성 파일 — spec/sdk/gen/binding.py 가 SDK 정의서 원천(areas/flr 의 입구 표 · 효과 표)에서 짓는다. 손으로 고치지 않는다.
// 위(연동규격서)가 지은 FLR 클라 상태기 Rust 판을 SDK 표면에 잇는다 — TS 판 결합과 같은 표 · 위가 효과를 바꾸면 아래 match 가 깨져 짓기가 멈춘다.
// 집행은 부르는 쪽이 plan() 의 목록대로 직접 한다 — 콜백을 주입하지 않는다.
#![allow(dead_code)]
use super::floor_client as F;

/// SDK 가 앱에 내는 이벤트(효과 표의 app.* 행)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PttEvent { Denied, Granted, Queued, Revoked, Speaker, State, }
impl PttEvent {
    /// 표면 이름(SDK 정의서 효과 표 그대로)
    pub fn as_str(self) -> &'static str {
        match self { PttEvent::Denied => "denied", PttEvent::Granted => "granted", PttEvent::Queued => "queued", PttEvent::Revoked => "revoked", PttEvent::Speaker => "speaker", PttEvent::State => "state", }
    }
}

/// SDK 가 하는 동작(효과 표의 그 밖 행)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PttAction { PlaybackRender, }
impl PttAction {
    /// 표면 이름(SDK 정의서 효과 표 그대로)
    pub fn as_str(self) -> &'static str {
        match self { PttAction::PlaybackRender => "playback.render", }
    }
}

/// 위 입구 중 SDK 가 넣는 것(입구 표) — 위 입구 이름 그대로(Rust 판 함수는 snake_case)
pub const FED: &[&str] = &["press", "release", "intent", "queuePosition", "rtpTx", "rtpRx", "pubSelect", "pubAway", "leave", "join", "down", "dcDown", "dcUp", "expire", "receive"];
/// 위 입구 중 넣지 않는 것(입구 표 셋째 칸 · 판정표에 사유)
pub const NOT_FED: &[&str] = &[];

/// 위 효과 하나의 갈 곳 — 이벤트 · 동작 · (위 이음매면) 받는 영역
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    pub event: Option<PttEvent>,
    pub action: Option<PttAction>,
    pub seam: Option<&'static str>,
}
/// 위 효과 하나 → SDK 가 낼 이벤트 · 할 동작(효과 표) — 위 효과가 늘거나 줄면 여기서 깨진다
pub fn effect(f: F::Fx) -> Effect {
    match f {
        F::Fx::media_render => Effect { event: None, action: Some(PttAction::PlaybackRender), seam: None },
        F::Fx::app_granted => Effect { event: Some(PttEvent::Granted), action: None, seam: None },
        F::Fx::app_granted_queued => Effect { event: Some(PttEvent::Granted), action: None, seam: None },
        F::Fx::app_denied => Effect { event: Some(PttEvent::Denied), action: None, seam: None },
        F::Fx::app_queued => Effect { event: Some(PttEvent::Queued), action: None, seam: None },
        F::Fx::app_revoked => Effect { event: Some(PttEvent::Revoked), action: None, seam: None },
        F::Fx::app_speaker => Effect { event: Some(PttEvent::Speaker), action: None, seam: None },
        F::Fx::app_speaker_none => Effect { event: Some(PttEvent::Speaker), action: None, seam: None },
        F::Fx::app_request_timeout => Effect { event: Some(PttEvent::State), action: None, seam: None },
        F::Fx::app_queue_timeout => Effect { event: Some(PttEvent::State), action: None, seam: None },
        F::Fx::app_grant_expired => Effect { event: Some(PttEvent::State), action: None, seam: None },
        F::Fx::app_floor_unreliable => Effect { event: Some(PttEvent::State), action: None, seam: None },
        F::Fx::app_floor_reliable => Effect { event: Some(PttEvent::State), action: None, seam: None },
        F::Fx::mic_close => Effect { event: None, action: None, seam: Some("PUB") },   // 위 이음매 — 받는 곳은 위가 정했다
        F::Fx::mic_open => Effect { event: None, action: None, seam: Some("PUB") },   // 위 이음매 — 받는 곳은 위가 정했다
    }
}

/// 한 걸음의 집행 목록 — 보낼 것 · 걸 타이머 · ACK는 위가 정한 그대로, 이벤트 · 동작은 효과 표대로
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub state: F::Cl,
    pub label: &'static str,
    pub send: Vec<F::Tx>,
    pub ack: Vec<F::Ack>,
    /// 위 이음매로 넘길 효과(위 IR seams)
    pub routed: Vec<F::Fx>,
    pub timers: Vec<F::TimerName>,
    pub events: Vec<PttEvent>,
    pub actions: Vec<PttAction>,
}
pub fn plan(step: F::Step) -> Plan {
    let mut events = Vec::new();
    let mut actions = Vec::new();
    let mut routed = Vec::new();
    for f in &step.fx {
        let e = effect(*f);
        if e.seam.is_some() {
            routed.push(*f);
        }
        events.extend(e.event);
        actions.extend(e.action);
    }
    Plan { state: step.s, label: step.label, send: step.out, ack: step.ack, routed, timers: step.start, events, actions }
}
