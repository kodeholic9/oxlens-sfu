// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:CAND-OPS-SUPERVISOR-EXTRA-TRANSITIONS · 정:CAND-OPS-SHUTDOWN-NO-RESTART · 정:CAND-OPS-SUPERVISOR-DEFAULTS · model: claude-opus-5-5
//! supervisor 판정(순수) — 상태 여덟 · 기동 시도 세기(exec 실패 포함 · Stopped 뒤 기동 제외) · backoff · 폭주 → Blocked.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    Disabled,
    Inactive,
    Starting,
    Running,
    Stopping,
    Stopped,
    Down,
    Blocked,
}

impl UnitState {
    pub fn name(self) -> &'static str {
        match self {
            UnitState::Disabled => "Disabled",
            UnitState::Inactive => "Inactive",
            UnitState::Starting => "Starting",
            UnitState::Running => "Running",
            UnitState::Stopping => "Stopping",
            UnitState::Stopped => "Stopped",
            UnitState::Down => "Down",
            UnitState::Blocked => "Blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Boot,
    Ready,
    Exited,
    Stop,
    StopDone,
    Load,
    BackoffDue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Spawn,
    Terminate,
    Retry(u64),
    Refuse(&'static str),
    None,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub backoff_ms: Vec<u64>,
    pub intensity_max: u32,
    pub window_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Core {
    pub state: UnitState,
    pub restarts: u32,
    starts: VecDeque<u64>,
    backoff_idx: usize,
    pub next_retry_at: Option<u64>,
    pub shutting: bool,
}

impl Core {
    pub fn new(enabled: bool) -> Core {
        Core { state: if enabled { UnitState::Inactive } else { UnitState::Disabled }, restarts: 0, starts: VecDeque::new(), backoff_idx: 0, next_retry_at: None, shutting: false }
    }

    fn count_start(&mut self, now: u64, lim: &Limits) -> bool {
        self.starts.retain(|t| now.saturating_sub(*t) < lim.window_ms);
        self.starts.push_back(now);
        self.starts.len() as u32 > lim.intensity_max
    }

    pub fn on(&mut self, ev: Event, now: u64, lim: &Limits) -> Effect {
        use UnitState as S;
        match (self.state, ev) {
            (S::Disabled, Event::Load) => Effect::Refuse("Disabled — 설정이 끈 유닛이다(enabled=false)"),
            (S::Disabled, _) => Effect::None,
            (S::Inactive, Event::Boot) => {
                if self.count_start(now, lim) {
                    self.state = S::Blocked;
                    return Effect::None;
                }
                self.state = S::Starting;
                Effect::Spawn
            }
            (S::Starting, Event::Ready) => {
                self.state = S::Running;
                self.backoff_idx = 0;
                Effect::None
            }
            (S::Starting | S::Running, Event::Exited) => self.down(now, lim),
            (S::Starting | S::Running, Event::Stop) => {
                self.state = S::Stopping;
                Effect::Terminate
            }
            (S::Stopping, Event::Exited | Event::StopDone) => {
                self.state = S::Stopped;
                self.next_retry_at = None;
                Effect::None
            }
            (S::Stopped, Event::Load) => {
                self.state = S::Starting;
                Effect::Spawn
            }
            (S::Down, Event::BackoffDue) => {
                if self.shutting {
                    return Effect::None;
                }
                self.restarts += 1;
                if self.count_start(now, lim) {
                    self.state = S::Blocked;
                    self.next_retry_at = None;
                    return Effect::None;
                }
                self.state = S::Starting;
                Effect::Spawn
            }
            (S::Down | S::Blocked, Event::Stop) => {
                self.state = S::Stopped;
                self.next_retry_at = None;
                Effect::None
            }
            (S::Blocked, Event::Load) => {
                self.starts.clear();
                self.backoff_idx = 0;
                self.state = S::Starting;
                Effect::Spawn
            }
            (_, Event::Load) => Effect::Refuse("이미 떠 있거나 내려가는 중이다"),
            _ => Effect::None,
        }
    }

    fn down(&mut self, now: u64, lim: &Limits) -> Effect {
        self.state = UnitState::Down;
        if self.shutting {
            self.next_retry_at = None;
            return Effect::None;
        }
        let wait = lim.backoff_ms.get(self.backoff_idx).or(lim.backoff_ms.last()).map(u64::to_owned).unwrap_or(1000);
        self.backoff_idx = (self.backoff_idx + 1).min(lim.backoff_ms.len().saturating_sub(1));
        self.next_retry_at = Some(now + wait);
        Effect::Retry(wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim() -> Limits {
        Limits { backoff_ms: vec![200, 500], intensity_max: 3, window_ms: 60_000 }
    }

    #[test]
    fn crash_loop_blocks_but_stop_load_does_not() {
        let l = lim();
        let mut c = Core::new(true);
        assert_eq!(c.on(Event::Boot, 0, &l), Effect::Spawn);
        assert_eq!(c.on(Event::Exited, 10, &l), Effect::Retry(200), "S-sym-RULE-OPS-SUPERVISOR-4 exec 실패도 기동 시도로 센다");
        assert_eq!(c.on(Event::BackoffDue, 210, &l), Effect::Spawn);
        assert_eq!(c.on(Event::Exited, 220, &l), Effect::Retry(500));
        assert_eq!(c.on(Event::BackoffDue, 720, &l), Effect::Spawn);
        c.on(Event::Exited, 730, &l);
        assert_eq!(c.on(Event::BackoffDue, 1300, &l), Effect::None);
        assert_eq!(c.state, UnitState::Blocked, "S-sym-RULE-OPS-SUPERVISOR-4 무한 재시도 0");
        let mut d = Core::new(true);
        d.on(Event::Boot, 0, &l);
        d.on(Event::Ready, 1, &l);
        for i in 0..10u64 {
            assert_eq!(d.on(Event::Stop, 10 + i * 10, &l), Effect::Terminate);
            d.on(Event::StopDone, 11 + i * 10, &l);
            assert_eq!(d.state, UnitState::Stopped, "S-sym-RULE-OPS-SUPERVISOR-1 Stopped 에서 재기동 0");
            assert_eq!(d.on(Event::Load, 12 + i * 10, &l), Effect::Spawn);
            d.on(Event::Ready, 13 + i * 10, &l);
        }
        assert_eq!(d.state, UnitState::Running, "S-sym-RULE-OPS-SUPERVISOR-1 Stopped 뒤 기동은 창에 안 쌓인다");
    }

    #[test]
    fn disabled_and_inactive_never_jump_to_running() {
        let l = lim();
        let mut c = Core::new(false);
        assert!(matches!(c.on(Event::Load, 0, &l), Effect::Refuse(_)), "S-sym-RULE-OPS-SUPERVISOR-3 Disabled 는 load 가 안 먹는다");
        let mut i = Core::new(true);
        i.on(Event::Ready, 0, &l);
        assert_eq!(i.state, UnitState::Inactive, "S-sym-RULE-OPS-SUPERVISOR-2 Inactive → Running 직행 0");
        let mut s = Core::new(true);
        s.shutting = true;
        s.on(Event::Boot, 0, &l);
        assert_eq!(s.on(Event::Exited, 1, &l), Effect::None, "종료가 시작되면 되살리지 않는다");
    }
}
