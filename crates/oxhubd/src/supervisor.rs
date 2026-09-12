// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§16-1 · §16-1-4 · §15-6 · §15-7 · model: claude-opus-5

//! supervisor — ★**가름이 이 파일의 전부다.**
//!
//! `Down`(비정상 → backoff 재기동)과 `Stopped`(의도적 정지 → 방치)를 가르지 않으면
//! `stop`/`load` 반복이 intensity 폭주로 읽혀 ★**hub 가 자폭한다.**
//!
//! ★**이 모듈은 판정만 한다** — 프로세스를 띄우고 거두는 것은 주인이 한다.
//! 그래야 시계를 주입해 전이를 1층에서 전수로 시험할 수 있고, 흐름이 한 자리에 다 보인다
//! (★콜백을 주입하지 않는다 — 흐름이 런타임에만 드러나면 추적이 불가능해진다).

use common::system::{Unit, UnitKind};

/// 정§16-1-4 — ★**사유가 여덟이라 여덟이다.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    /// 설정이 끈 유닛 — 띄우지도 세지도 않는다.
    Disabled,
    /// 쥐었으나 아직 안 띄웠다.
    Inactive,
    /// 띄웠다. ★**아직 준비 신호가 없다.**
    Starting,
    /// 준비됐다. ★★**유닛 `live` 가 이 상태 하나다.**
    Running,
    /// graceful 종료 중.
    Stopping,
    /// ★**의도적 정지 — 방치한다.**
    Stopped,
    /// ★**비정상 종료 — backoff 뒤 재기동한다.**
    Down,
    /// ★**backoff 폭주 — 더 못 살린다.**
    Blocked,
}

impl UnitState {
    /// ★**유닛 `live` 는 이 하나다**(정§16-1 — 노드 축과 다른 물음).
    pub fn is_live(self) -> bool {
        self == UnitState::Running
    }
}

/// 주인이 받아 손을 쓰는 판단. ★**콜백이 아니라 값이다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 아무것도 안 한다.
    None,
    /// 이 유닛을 띄운다.
    Spawn(String),
    /// 이 유닛에 종료 신호를 보낸다.
    Signal(String),
    /// ★**hub 를 내려라** — 살릴 수 없는 유닛이 생겼다(zenoh 면 노드가 통째로 내려간다).
    ShutdownHub(String),
}

/// backoff 사다리. 창 안의 기동이 이 길이를 넘으면 `Blocked` 다.
const BACKOFF_MS: &[u64] = &[500, 1_000, 2_000, 4_000, 8_000];
/// intensity 창 — 이 시간 안의 기동만 센다.
const WINDOW_MS: u64 = 60_000;

#[derive(Debug, Clone)]
pub struct UnitSup {
    pub id: String,
    pub kind: UnitKind,
    pub order: u8,
    pub state: UnitState,
    /// ★**창 안의 기동 시각**만 쌓는다. `Stopped` 뒤의 기동은 여기 안 들어간다.
    starts: Vec<u64>,
    /// 총 재기동 횟수 — 관측용(창과 다른 축).
    pub restarts: u32,
    /// backoff 중이면 그 만료 시각.
    retry_at: Option<u64>,
    /// ★**기동 신원**(`epoch` = `sfu_id` = §15-1 `{inst}`) — 유닛이 등록할 때 받아 든다.
    pub epoch: Option<String>,
}

impl UnitSup {
    pub fn new(u: &Unit) -> Self {
        Self {
            id: u.id.clone(),
            kind: u.kind,
            order: u.order,
            state: if u.enabled { UnitState::Inactive } else { UnitState::Disabled },
            starts: Vec::new(),
            restarts: 0,
            retry_at: None,
            epoch: None,
        }
    }

    /// backoff 잔량 — `/admin/supervisor/status` 가 낸다. ★**backoff 중에만 값이 있다.**
    pub fn next_retry_ms(&self, now: u64) -> Option<u64> {
        self.retry_at.map(|t| t.saturating_sub(now))
    }

    fn prune(&mut self, now: u64) {
        self.starts.retain(|t| now.saturating_sub(*t) <= WINDOW_MS);
    }

    /// ★★**무엇을 기동 시도로 세나 — 못 띄운 것도 센다.**
    ///
    /// 안 세면 intensity 가 안 올라 backoff 도 `Blocked` 도 안 걸리고
    /// ★**없는 실행파일에 무한 재시도**를 돈다.
    fn count_start(&mut self, now: u64) {
        self.prune(now);
        self.starts.push(now);
        self.restarts += 1;
    }

    fn backoff_for(&self, n: usize) -> Option<u64> {
        BACKOFF_MS.get(n.saturating_sub(1)).copied()
    }
}

/// 유닛 묶음의 판정기.
#[derive(Debug, Clone, Default)]
pub struct Supervisor {
    pub units: Vec<UnitSup>,
}

impl Supervisor {
    pub fn new(units: &[Unit]) -> Self {
        let mut v: Vec<UnitSup> = units.iter().map(UnitSup::new).collect();
        v.sort_by(|a, b| (a.order, a.id.as_str()).cmp(&(b.order, b.id.as_str())));
        Self { units: v }
    }

    pub fn get(&self, id: &str) -> Option<&UnitSup> {
        self.units.iter().find(|u| u.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut UnitSup> {
        self.units.iter_mut().find(|u| u.id == id)
    }

    /// 기동 — ★**순서 속성대로**(정지는 역순이다).
    pub fn start_all(&mut self, now: u64) -> Vec<Action> {
        let ids: Vec<String> = self
            .units
            .iter()
            .filter(|u| u.state == UnitState::Inactive)
            .map(|u| u.id.clone())
            .collect();
        ids.into_iter().map(|id| self.spawn(&id, now)).collect()
    }

    fn spawn(&mut self, id: &str, now: u64) -> Action {
        match self.get_mut(id) {
            Some(u) => {
                u.count_start(now);
                u.state = UnitState::Starting;
                u.retry_at = None;
                Action::Spawn(u.id.clone())
            }
            None => Action::None,
        }
    }

    /// 유닛이 준비됐다고 알려 왔다 — ★**기동 신원을 여기서 받아 든다.**
    ///
    /// ★**`Inactive` 에서 바로 오면 무시한다** — 띄운 적 없는 것이 살아 있을 수 없다.
    pub fn on_ready(&mut self, id: &str, epoch: String) -> Action {
        if let Some(u) = self.get_mut(id)
            && u.state == UnitState::Starting
        {
            u.state = UnitState::Running;
            u.epoch = Some(epoch);
        }
        Action::None
    }

    /// 자식이 끝났다. ★`Stopping` 뒤였으면 의도적 정지고, 그 밖은 `Down` 이다.
    pub fn on_exit(&mut self, id: &str, now: u64) -> Action {
        let (state, n) = match self.get_mut(id) {
            Some(u) => {
                u.epoch = None;
                if u.state == UnitState::Stopping {
                    u.state = UnitState::Stopped;
                    return Action::None;
                }
                u.state = UnitState::Down;
                u.prune(now);
                (u.state, u.starts.len())
            }
            None => return Action::None,
        };
        debug_assert_eq!(state, UnitState::Down);
        match self.get(id).and_then(|u| u.backoff_for(n)) {
            Some(ms) => {
                if let Some(u) = self.get_mut(id) {
                    u.retry_at = Some(now + ms);
                }
                Action::None
            }
            // ★사다리를 다 썼다 — 더 못 살린다.
            None => {
                if let Some(u) = self.get_mut(id) {
                    u.state = UnitState::Blocked;
                    u.retry_at = None;
                }
                Action::ShutdownHub(id.to_string())
            }
        }
    }

    /// ★**exec 가 실패했다 — 이것도 기동 시도다.** 안 세면 무한 재시도를 돈다.
    pub fn on_spawn_failed(&mut self, id: &str, now: u64) -> Action {
        self.on_exit(id, now)
    }

    /// 시계가 흘렀다 — backoff 가 만료된 유닛을 다시 띄운다.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        let due: Vec<String> = self
            .units
            .iter()
            .filter(|u| u.state == UnitState::Down && u.retry_at.is_some_and(|t| t <= now))
            .map(|u| u.id.clone())
            .collect();
        due.into_iter().map(|id| self.spawn(&id, now)).collect()
    }

    /// 운영자가 내렸다 — ★**`Stopped` 로 적고 재기동하지 않는다.**
    pub fn stop(&mut self, id: &str) -> Option<(UnitState, UnitState)> {
        let u = self.get_mut(id)?;
        let from = u.state;
        if from == UnitState::Disabled {
            return Some((from, from));
        }
        u.state = UnitState::Stopping;
        u.retry_at = None;
        Some((from, UnitState::Stopping))
    }

    /// 운영자가 다시 올렸다. ★**`Stopped` 뒤의 기동은 창에 안 쌓는다** — 열 번 껐다 켠 것이
    /// 폭주로 세이면 hub 가 자폭한다.
    pub fn load(&mut self, id: &str, now: u64) -> Option<(UnitState, UnitState, Action)> {
        let u = self.get_mut(id)?;
        let from = u.state;
        // ★`Disabled` 는 설정이 끈 것이라 `load` 가 먹지 않는다 — `Stopped` 와 다른 값인 이유다.
        if from == UnitState::Disabled {
            return Some((from, from, Action::None));
        }
        u.state = UnitState::Starting;
        u.retry_at = None;
        let action = Action::Spawn(u.id.clone());
        let _ = now;
        Some((from, UnitState::Starting, action))
    }

    /// 종료 — ★**정지는 기동의 역순이다**(§15-6: zenoh 는 나중에 닫힌다).
    pub fn stop_order(&self) -> Vec<&UnitSup> {
        let mut v: Vec<&UnitSup> = self.units.iter().filter(|u| u.state != UnitState::Disabled).collect();
        v.sort_by(|a, b| (b.order, b.id.as_str()).cmp(&(a.order, a.id.as_str())));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str, order: u8, enabled: bool) -> Unit {
        Unit {
            id: id.into(),
            kind: UnitKind::Process,
            order,
            role: String::new(),
            cmd: vec!["x".into()],
            addr: String::new(),
            enabled,
        }
    }

    fn sup() -> Supervisor {
        Supervisor::new(&[unit("zenoh", 1, true), unit("sfu-1", 2, true), unit("off", 3, false)])
    }

    #[test]
    fn 설정이_끈_유닛은_다른_값이다() {
        let s = sup();
        assert_eq!(s.get("off").expect("u").state, UnitState::Disabled);
        assert_ne!(UnitState::Disabled, UnitState::Stopped, "★합치면 load 가 먹는 줄 안다");
    }

    #[test]
    fn 기동은_순서대로_정지는_역순이다() {
        let mut s = sup();
        let acts = s.start_all(0);
        assert_eq!(acts[0], Action::Spawn("zenoh".into()), "★zenoh 가 먼저 선다");
        let ids: Vec<&str> = s.stop_order().iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["sfu-1", "zenoh"], "★나중에 닫힌다");
    }

    #[test]
    fn 띄운_적_없으면_살아_있을_수_없다() {
        let mut s = sup();
        s.on_ready("sfu-1", "e1".into());
        // ★`Inactive`→`Running` 직행이 되면 healthz/ready 가 200 을 낸다.
        assert_eq!(s.get("sfu-1").expect("u").state, UnitState::Inactive);
        assert!(!s.get("sfu-1").expect("u").state.is_live());
    }

    #[test]
    fn 준비_신호가_기동_신원을_들여온다() {
        let mut s = sup();
        s.start_all(0);
        s.on_ready("sfu-1", "sfu-7f3a".into());
        let u = s.get("sfu-1").expect("u");
        assert_eq!(u.state, UnitState::Running);
        assert!(u.state.is_live(), "★유닛 live 는 Running 하나다");
        assert_eq!(u.epoch.as_deref(), Some("sfu-7f3a"), "★kill 의 확인값이 이 값이다");
    }

    #[test]
    fn 의도적_정지는_방치한다() {
        let mut s = sup();
        s.start_all(0);
        s.on_ready("sfu-1", "e".into());
        assert_eq!(s.stop("sfu-1"), Some((UnitState::Running, UnitState::Stopping)));
        s.on_exit("sfu-1", 10);
        assert_eq!(s.get("sfu-1").expect("u").state, UnitState::Stopped);
        // ★backoff 로 되살아나면 안 된다.
        assert!(s.tick(999_999).is_empty(), "★Stopped 는 방치한다");
    }

    #[test]
    fn 급사는_backoff_로_되살린다() {
        let mut s = sup();
        s.start_all(0);
        s.on_ready("sfu-1", "e".into());
        s.on_exit("sfu-1", 100);
        let u = s.get("sfu-1").expect("u");
        assert_eq!(u.state, UnitState::Down);
        assert_eq!(u.next_retry_ms(100), Some(500));
        assert!(s.tick(200).is_empty(), "★만료 전엔 안 띄운다");
        assert_eq!(s.tick(600), vec![Action::Spawn("sfu-1".into())]);
    }

    #[test]
    fn 못_띄운_것도_기동_시도다() {
        // ★안 세면 없는 실행파일에 무한 재시도를 돈다.
        let mut s = sup();
        let mut now = 0;
        // 첫 기동 — 없는 실행파일이라 exec 가 실패한다.
        s.start_all(now);
        let mut last = s.on_spawn_failed("sfu-1", now);
        // ★사다리를 타고 다시 띄우기를 되풀이한다 — 매번 "기동 시도"로 세야 한다.
        for _ in 0..BACKOFF_MS.len() {
            assert_eq!(last, Action::None, "아직 사다리가 남았다");
            let wait = s.get("sfu-1").expect("u").next_retry_ms(now).expect("backoff");
            now += wait;
            assert_eq!(s.tick(now), vec![Action::Spawn("sfu-1".into())]);
            last = s.on_spawn_failed("sfu-1", now);
        }
        assert_eq!(last, Action::ShutdownHub("sfu-1".into()));
        assert_eq!(s.get("sfu-1").expect("u").state, UnitState::Blocked);
        // ★더는 안 띄운다 — 무한 재시도가 여기서 멎는다.
        assert!(s.tick(now + 999_999).is_empty());
    }

    #[test]
    fn 껐다_켜기를_되풀이해도_자폭하지_않는다() {
        // ★`Stopped` 뒤의 기동은 창에 안 쌓는다 — 열 번 stop/load 가 폭주로 세이면 hub 가 죽는다.
        let mut s = sup();
        s.start_all(0);
        s.on_ready("sfu-1", "e".into());
        for i in 0..10 {
            let now = 1_000 + i * 10;
            s.stop("sfu-1");
            s.on_exit("sfu-1", now);
            let (_, to, _) = s.load("sfu-1", now).expect("load");
            assert_eq!(to, UnitState::Starting);
            s.on_ready("sfu-1", "e".into());
        }
        assert_eq!(s.get("sfu-1").expect("u").state, UnitState::Running);
        assert_ne!(s.get("sfu-1").expect("u").state, UnitState::Blocked);
    }

    #[test]
    fn 꺼진_유닛은_load_가_안_먹는다() {
        let mut s = sup();
        let (from, to, act) = s.load("off", 0).expect("load");
        assert_eq!(from, UnitState::Disabled);
        assert_eq!(to, UnitState::Disabled);
        assert_eq!(act, Action::None);
    }
}
