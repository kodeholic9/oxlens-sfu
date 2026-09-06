// author: kodeholic (powered by Claude)
//! 정§16-1 supervisor — 유닛을 띄우고 지켜보고 거둔다.
//!
//! ★가름이 이 파일의 전부다. `Down`(비정상 → backoff 재기동)과 `Stopped`(의도적 정지 → 방치)를
//! 안 가르면 stop/load 반복이 그대로 기동 횟수로 세어져 intensity 폭주 → hub 자폭이 된다.
//!
//! 판단(`UnitFsm`)은 프로세스를 모른다 — 1층이 진짜 자식을 안 띄우고 8상태 전이를 전수로 잰다.
//! 실을 쥔 쪽(`Supervisor`)이 그 판단을 받아 spawn/kill 한다.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::config::{Supervisor as SupervisorCfg, Unit};
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

/// 정§16-1 UnitState 8종.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    /// 등재만 됐다 — 아직 띄운 적 없다.
    Inactive,
    /// 띄웠고 아직 못 붙었다.
    Starting,
    /// 붙었다.
    Live,
    /// ★비정상으로 빠졌다 — 재기동 대상.
    Down,
    /// 재기동까지 기다리는 중.
    Backoff,
    /// graceful 종료를 넣었고 거두는 중.
    Stopping,
    /// ★의도적으로 멈췄다 — 방치한다. 기동 횟수로 세지 않는다.
    Stopped,
    /// ★backoff 폭주 — 더 못 살린다. hub 를 내린다.
    Blocked,
}

/// 정§18-1 `restart` — `no` · `on-failure` · `always`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Never,
    OnFailure,
    Always,
}

impl Restart {
    pub fn parse(s: &str) -> Self {
        match s {
            "always" => Self::Always,
            "no" => Self::Never,
            // ★모르는 값은 가장 조용한 쪽이 아니라 규격 기본값으로 — 오타로 감시가 꺼지면 안 보인다.
            _ => Self::OnFailure,
        }
    }
}

/// 주인이 이 판단을 받아 손을 쓴다. ★콜백을 주입하지 않는다 — 흐름이 여기 다 보인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 할 일 없다.
    Idle,
    /// 지금 띄워라.
    Spawn,
    /// ★hub 를 내려라 — 살릴 수 없는 유닛이 생겼다.
    ShutdownHub,
}

/// 한 유닛의 판단. 시각은 밖에서 받는다 — 그래야 1층이 결정적으로 잰다.
#[derive(Debug)]
pub struct UnitFsm {
    pub state: UnitState,
    restart: Restart,
    /// `cmd` 없음 = 원격 — 우리가 못 띄운다. 살았나 죽었나만 적는다.
    managed: bool,
    backoff_start: Duration,
    backoff_max: Duration,
    backoff: Duration,
    /// 창 안의 기동 시각. ★의도적 정지 뒤의 기동은 여기 안 들어간다.
    starts: Vec<Instant>,
    burst: u32,
    window: Duration,
    resume_at: Option<Instant>,
}

impl UnitFsm {
    pub fn new(unit: &Unit, cfg: &SupervisorCfg) -> Self {
        Self {
            state: UnitState::Inactive,
            restart: Restart::parse(&unit.restart),
            managed: unit.cmd.is_some(),
            backoff_start: Duration::from_millis(cfg.backoff_start_ms),
            backoff_max: Duration::from_millis(cfg.backoff_max_ms),
            backoff: Duration::from_millis(cfg.backoff_start_ms),
            starts: Vec::new(),
            burst: cfg.start_limit_burst,
            window: Duration::from_secs(cfg.start_limit_interval_sec),
            resume_at: None,
        }
    }

    pub fn managed(&self) -> bool {
        self.managed
    }

    /// 띄웠다. ★여기서만 기동 횟수를 센다.
    pub fn started(&mut self, now: Instant) {
        self.starts.retain(|t| now.duration_since(*t) < self.window);
        self.starts.push(now);
        self.state = UnitState::Starting;
        self.resume_at = None;
    }

    /// 붙었다 — 여기서 backoff 가 처음으로 돌아간다(한 번 살면 다음 사고는 새 사고다).
    ///
    /// ★우리가 띄우는 유닛은 `Inactive` 에서 바로 `Live` 로 못 간다(띄운 적 없는 것이 살아 있을 리 없다).
    /// 원격 유닛은 반대다 — 띄우는 단계가 아예 없으니 붙는 것이 곧 사는 것이다.
    pub fn ready(&mut self) {
        let from_scratch = !self.managed && self.state == UnitState::Inactive;
        if from_scratch || matches!(self.state, UnitState::Starting | UnitState::Down | UnitState::Backoff) {
            self.state = UnitState::Live;
            self.backoff = self.backoff_start;
        }
    }

    /// 못 붙는다 / 끊겼다 — 원격 유닛은 이 길로만 상태가 움직인다.
    pub fn unreachable(&mut self) {
        if self.state == UnitState::Live {
            self.state = UnitState::Down;
        }
    }

    /// graceful 종료를 넣었다.
    pub fn stopping(&mut self) {
        self.state = UnitState::Stopping;
    }

    /// 자식이 끝났다. ★`Stopping` 뒤였거나 깨끗이 끝났으면 의도적 정지다.
    pub fn exited(&mut self, success: bool) {
        self.state = if self.state == UnitState::Stopping || (success && self.restart != Restart::Always) {
            UnitState::Stopped
        } else {
            UnitState::Down
        };
    }

    /// 주기적으로 묻는다 — 지금 무엇을 할 때인가.
    pub fn poll(&mut self, now: Instant) -> Decision {
        match self.state {
            UnitState::Down => {
                if !self.managed || self.restart == Restart::Never {
                    return Decision::Idle; // 방치 — 준비 평면이 대신 말한다.
                }
                self.state = UnitState::Backoff;
                self.resume_at = Some(now + self.backoff);
                self.backoff = (self.backoff * 2).min(self.backoff_max);
                Decision::Idle
            }
            UnitState::Backoff => {
                if self.resume_at.is_some_and(|t| now < t) {
                    return Decision::Idle;
                }
                if self.over_limit(now) {
                    self.state = UnitState::Blocked;
                    return Decision::ShutdownHub;
                }
                Decision::Spawn
            }
            _ => Decision::Idle,
        }
    }

    /// ★창 안의 기동만 센다 — `Stopped` 로 간 정지는 애초에 여기 안 쌓인다.
    fn over_limit(&self, now: Instant) -> bool {
        let recent = self.starts.iter().filter(|t| now.duration_since(**t) < self.window).count();
        u32::try_from(recent).unwrap_or(u32::MAX) >= self.burst
    }
}

// ───────────── 실을 쥔 쪽 ─────────────

pub struct ManagedUnit {
    pub unit: Unit,
    pub fsm: UnitFsm,
    child: Option<Child>,
}

pub struct Supervisor {
    pub units: Vec<ManagedUnit>,
    cfg: SupervisorCfg,
}

impl Supervisor {
    /// `enabled=false` 면 빈 것을 돌려준다 — 부르는 쪽이 분기를 안 갖게.
    pub fn new(cfg: &SupervisorCfg, units: &[Unit]) -> Self {
        let units = if cfg.enabled {
            units.iter().map(|u| ManagedUnit { unit: u.clone(), fsm: UnitFsm::new(u, cfg), child: None }).collect()
        } else {
            Vec::new()
        };
        Self { units, cfg: cfg.clone() }
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// 처음 한 번 — 우리가 띄울 수 있는 것만 띄운다.
    pub fn start_all(&mut self, now: Instant) {
        for m in &mut self.units {
            if m.fsm.managed() {
                spawn(m, now);
            }
        }
    }

    /// 한 바퀴. `probe` 는 그 `node_id` 가 지금 붙느냐 — 주인이 훑어서 넣어 준다.
    /// ★hub 를 내려야 하면 그 유닛 id 를 돌려준다.
    pub fn tick(&mut self, now: Instant, probe: &dyn Fn(&str) -> bool) -> Option<String> {
        let mut blocked = None;
        for m in &mut self.units {
            reap(m);
            if probe(&m.unit.id) {
                m.fsm.ready();
            } else {
                m.fsm.unreachable();
            }
            match m.fsm.poll(now) {
                Decision::Spawn => spawn(m, now),
                Decision::ShutdownHub => {
                    error!(unit = %m.unit.id, "unit blocked — 재기동이 폭주했다, hub 를 내린다");
                    blocked.get_or_insert_with(|| m.unit.id.clone());
                }
                Decision::Idle => {}
            }
        }
        blocked
    }

    /// 정§16-1 종료 순서 ② — 유닛 graceful. 클라 Close 는 이보다 **먼저** 나가 있어야 한다.
    pub async fn stop_all(&mut self) {
        for m in &mut self.units {
            let Some(child) = m.child.as_mut() else { continue };
            m.fsm.stopping();
            let grace = Duration::from_secs(m.unit.timeout_stop_sec);
            // SIGTERM — 자식이 스스로 거둘 기회를 준다. ★`kill()` 은 SIGKILL 이라 여기 못 쓴다.
            if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
                let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGTERM);
            }
            match tokio::time::timeout(grace, child.wait()).await {
                Ok(_) => info!(unit = %m.unit.id, "unit stopped"),
                Err(_) => {
                    warn!(unit = %m.unit.id, secs = m.unit.timeout_stop_sec, "graceful 시간이 지났다 — 강제 종료");
                    let _ = child.kill().await;
                }
            }
            m.fsm.exited(true);
            m.child = None;
        }
    }

    pub fn states(&self) -> Vec<(String, UnitState)> {
        self.units.iter().map(|m| (m.unit.id.clone(), m.fsm.state)).collect()
    }

    pub fn cfg(&self) -> &SupervisorCfg {
        &self.cfg
    }
}

fn reap(m: &mut ManagedUnit) {
    let Some(child) = m.child.as_mut() else { return };
    match child.try_wait() {
        Ok(Some(status)) => {
            warn!(unit = %m.unit.id, code = status.code(), "unit exited");
            m.fsm.exited(status.success());
            m.child = None;
        }
        Ok(None) => {}
        Err(e) => {
            error!(unit = %m.unit.id, error = %e, "자식 상태를 못 읽었다");
            m.fsm.exited(false);
            m.child = None;
        }
    }
}

fn spawn(m: &mut ManagedUnit, now: Instant) {
    let Some(cmd) = m.unit.cmd.as_deref() else { return };
    match Command::new(cmd).args(&m.unit.args).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn() {
        Ok(child) => {
            info!(unit = %m.unit.id, cmd, pid = child.id(), "unit started");
            m.child = Some(child);
            m.fsm.started(now);
        }
        Err(e) => {
            // ★못 띄운 것도 기동 시도다 — 안 세면 없는 실행파일로 무한 재시도를 돈다.
            error!(unit = %m.unit.id, cmd, error = %e, "unit spawn failed");
            m.fsm.started(now);
            m.fsm.exited(false);
        }
    }
}

/// 정§16-1 종료 순서 ① — 클라 전원에게 `4006 SERVER_SHUTDOWN`(연§10-3 백오프 재접속 안내).
pub async fn announce_shutdown(hub: &Arc<crate::ws::Hub>) {
    let n = hub.close_all(oxsig::CloseCode::ServerShutdown);
    info!(conns = n, "server shutdown announced to clients");
    // 소켓이 Close 를 실제로 흘려보낼 짬 — 여기서 안 주면 ②가 먼저 끊는다.
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(cmd: Option<&str>, restart: &str) -> Unit {
        Unit {
            role: "sfu".into(),
            id: "u1".into(),
            addr: "127.0.0.1:1".into(),
            cmd: cmd.map(str::to_owned),
            args: Vec::new(),
            restart: restart.into(),
            timeout_stop_sec: 1,
        }
    }
    fn cfg() -> SupervisorCfg {
        SupervisorCfg { enabled: true, backoff_start_ms: 100, backoff_max_ms: 400, start_limit_burst: 3, start_limit_interval_sec: 60 }
    }

    #[test]
    fn we_never_call_a_unit_we_manage_live_before_we_start_it() {
        let mut f = UnitFsm::new(&unit(Some("x"), "always"), &cfg());
        f.ready();
        assert_eq!(f.state, UnitState::Inactive, "우리가 띄우는 것은 띄운 뒤에만 살 수 있다 — 남의 포트를 우리 것으로 읽지 않는다");
    }

    #[test]
    fn a_crash_is_down_and_a_clean_exit_is_stopped() {
        let mut f = UnitFsm::new(&unit(Some("x"), "on-failure"), &cfg());
        let t = Instant::now();
        f.started(t);
        f.ready();
        f.exited(false);
        assert_eq!(f.state, UnitState::Down, "비정상 종료는 살려야 한다");

        let mut g = UnitFsm::new(&unit(Some("x"), "on-failure"), &cfg());
        g.started(t);
        g.ready();
        g.exited(true);
        assert_eq!(g.state, UnitState::Stopped, "깨끗이 끝난 것은 방치한다");
    }

    #[test]
    fn a_requested_stop_never_becomes_down() {
        let mut f = UnitFsm::new(&unit(Some("x"), "always"), &cfg());
        f.started(Instant::now());
        f.ready();
        f.stopping();
        f.exited(false); // 시간이 지나 강제 종료 — 성공이 아니다
        assert_eq!(f.state, UnitState::Stopped, "★내가 세운 것을 사고로 세면 stop/load 가 intensity 를 태운다");
        assert_eq!(f.poll(Instant::now()), Decision::Idle);
    }

    #[test]
    fn down_waits_out_a_growing_backoff_before_respawning() {
        let mut f = UnitFsm::new(&unit(Some("x"), "on-failure"), &cfg());
        let t0 = Instant::now();
        f.started(t0);
        f.exited(false);
        assert_eq!(f.poll(t0), Decision::Idle);
        assert_eq!(f.state, UnitState::Backoff);
        assert_eq!(f.poll(t0 + Duration::from_millis(99)), Decision::Idle, "아직 이르다");
        assert_eq!(f.poll(t0 + Duration::from_millis(100)), Decision::Spawn);

        // 두 번째 사고 — 기다림은 그 사고 시점부터 200ms 다(첫 기동 시점부터가 아니다).
        let t1 = t0 + Duration::from_millis(100);
        f.started(t1);
        f.exited(false);
        f.poll(t1);
        assert_eq!(f.poll(t1 + Duration::from_millis(199)), Decision::Idle, "두 번째는 200ms 다");
        assert_eq!(f.poll(t1 + Duration::from_millis(200)), Decision::Spawn);
    }

    #[test]
    fn a_burst_of_restarts_blocks_the_unit_and_takes_the_hub_down() {
        let mut f = UnitFsm::new(&unit(Some("x"), "always"), &cfg());
        let mut t = Instant::now();
        for _ in 0..3 {
            f.started(t);
            f.exited(false);
            f.poll(t);
            t += Duration::from_secs(1);
        }
        assert_eq!(f.poll(t), Decision::ShutdownHub, "창 안 3회면 폭주다");
        assert_eq!(f.state, UnitState::Blocked);
        assert_eq!(f.poll(t + Duration::from_secs(1)), Decision::Idle, "Blocked 는 끝이다 — 두 번 내리지 않는다");
    }

    #[test]
    fn a_stop_load_cycle_does_not_burn_the_limit() {
        let mut f = UnitFsm::new(&unit(Some("x"), "on-failure"), &cfg());
        let mut t = Instant::now();
        for _ in 0..10 {
            f.started(t);
            f.ready();
            f.stopping();
            f.exited(true);
            assert_eq!(f.state, UnitState::Stopped);
            t += Duration::from_secs(1);
        }
        // 그러고 진짜 사고가 한 번 나면 그때는 살려야 한다.
        f.started(t);
        f.exited(false);
        assert_eq!(f.poll(t), Decision::Idle);
        assert_eq!(f.state, UnitState::Backoff, "★열 번 껐다 켠 것이 폭주로 세이면 hub 가 자폭한다");
    }

    #[test]
    fn a_remote_unit_is_watched_but_never_spawned() {
        let mut f = UnitFsm::new(&unit(None, "always"), &cfg());
        assert!(!f.managed());
        f.ready();
        assert_eq!(f.state, UnitState::Live, "★남의 기계는 띄우는 단계가 없다 — 붙으면 그것이 사는 것이다");
        f.unreachable();
        assert_eq!(f.state, UnitState::Down);
        assert_eq!(f.poll(Instant::now()), Decision::Idle, "남의 기계는 우리가 못 띄운다");
        assert_eq!(f.state, UnitState::Down, "Backoff 로도 안 간다 — 기다림을 흉내 내지 않는다");
    }

    #[test]
    fn restart_never_leaves_a_crash_alone() {
        let mut f = UnitFsm::new(&unit(Some("x"), "no"), &cfg());
        f.started(Instant::now());
        f.exited(false);
        assert_eq!(f.state, UnitState::Down);
        assert_eq!(f.poll(Instant::now()), Decision::Idle);
        assert_eq!(f.state, UnitState::Down, "★Stopped 로 바꿔 적지 않는다 — 사고를 정상으로 적으면 준비 평면이 거짓말한다");
    }

    #[test]
    fn coming_back_up_resets_the_backoff() {
        let mut f = UnitFsm::new(&unit(Some("x"), "on-failure"), &cfg());
        let t0 = Instant::now();
        f.started(t0);
        f.exited(false);
        f.poll(t0); // backoff 100 → 200
        f.poll(t0 + Duration::from_millis(100));
        f.started(t0 + Duration::from_millis(100));
        f.ready();
        f.exited(false);
        f.poll(t0 + Duration::from_secs(10));
        assert_eq!(f.poll(t0 + Duration::from_secs(10) + Duration::from_millis(100)), Decision::Spawn,
            "한 번 살았으면 다음 사고는 처음부터 센다");
    }

    #[test]
    fn a_disabled_supervisor_holds_nothing() {
        let cfg = SupervisorCfg { enabled: false, ..cfg() };
        let s = Supervisor::new(&cfg, &[unit(Some("x"), "always")]);
        assert!(s.is_empty(), "꺼져 있으면 유닛을 쥐지 않는다 — 부르는 쪽에 분기를 안 만든다");
    }

    #[test]
    fn unknown_restart_words_fall_back_to_the_spec_default() {
        assert_eq!(Restart::parse("on-failure"), Restart::OnFailure);
        assert_eq!(Restart::parse("always"), Restart::Always);
        assert_eq!(Restart::parse("no"), Restart::Never);
        assert_eq!(Restart::parse("On-Failure"), Restart::OnFailure, "오타로 감시가 조용히 꺼지지 않는다");
    }
}
