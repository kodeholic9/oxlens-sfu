// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:RULE-BUS-FATE · 정:IMPL-CFG-SYSTEM-FILE · 정:IMPL-BUS-FAULTS · 운영:RULE-CTL-SUPERVISOR · model: claude-opus-5-5
//! supervisor — `UnitState` 여덟 · 못 띄운 것도 기동 시도로 센다 · `Stopped` 뒤 기동은 창에 안 쌓는다 · 준비 = Hello 성공 · 부모 생존 채널(stdin) · 종료 중엔 되살리지 않는다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::system::{Unit, UnitKind};
use tokio::sync::{mpsc, oneshot};

use crate::bus::{Bus, Local};

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
    pub fn as_str(self) -> &'static str {
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
pub enum Role {
    Log,
    Zenoh,
    Sfu,
}

pub struct UnitRec {
    pub state: UnitState,
    pub restarts: u32,
    pub next_retry_at: Option<Instant>,
    pub epoch: Option<String>,
    pub build: Option<String>,
    attempts: VecDeque<Instant>,
}

pub enum Cmd {
    Load(oneshot::Sender<Result<(UnitState, UnitState), String>>),
    Stop(oneshot::Sender<Result<(UnitState, UnitState), String>>),
    Kill(oneshot::Sender<Result<(UnitState, UnitState), String>>),
}

pub struct UnitHandle {
    pub id: String,
    pub kind: UnitKind,
    pub role: Role,
    pub order: i32,
    pub addr: Option<String>,
    pub cfg: Unit,
    pub rec: Mutex<UnitRec>,
    pub tx: mpsc::UnboundedSender<Cmd>,
    rx: Mutex<Option<mpsc::UnboundedReceiver<Cmd>>>,
}

pub struct SupCfg {
    pub node: String,
    pub hub_inst: String,
    pub build_label: Option<String>,
    pub backoff_ms: Vec<u64>,
    pub intensity_max: u32,
    pub intensity_window: Duration,
    pub hello_timeout: Duration,
    pub extra_args: Vec<String>,
}

pub struct Supervisor {
    pub units: Vec<Arc<UnitHandle>>,
    pub cfg: SupCfg,
    pub stopping: AtomicBool,
    pub node_declared: AtomicBool,
    pub fatal: mpsc::UnboundedSender<String>,
}

fn line(unit: &str, from: UnitState, to: UnitState, cause: &str) {
    let t = common::log::Transition {
        area: "OPS",
        key: unit,
        spec: Some("SV-STATE-UNIT-STATE"),
        from: from.as_str(),
        to: Some(to.as_str()),
        cause,
        inputs: &[],
    };
    common::log::emit(t.line().key("unit", unit));
}

impl UnitHandle {
    pub fn state(&self) -> UnitState {
        self.rec.lock().map(|r| r.state).unwrap_or(UnitState::Down)
    }

    pub fn epoch(&self) -> Option<String> {
        self.rec.lock().ok().and_then(|r| if r.state == UnitState::Running { r.epoch.clone() } else { None })
    }

    fn set(&self, to: UnitState, cause: &str) -> UnitState {
        let from = match self.rec.lock() {
            Ok(mut r) => {
                let f = r.state;
                r.state = to;
                if to != UnitState::Down {
                    r.next_retry_at = None;
                }
                f
            }
            Err(_) => return to,
        };
        if from != to {
            line(&self.id, from, to, cause);
        }
        from
    }
}

impl Supervisor {
    pub fn new(units_cfg: &[Unit], cfg: SupCfg, fatal: mpsc::UnboundedSender<String>) -> Arc<Supervisor> {
        let mut units: Vec<Arc<UnitHandle>> = Vec::new();
        let mk = |u: Unit, role: Role| {
            let (tx, rx) = mpsc::unbounded_channel();
            Arc::new(UnitHandle {
                id: u.id.clone(),
                kind: u.kind,
                role,
                order: u.order,
                addr: u.addr.clone(),
                rec: Mutex::new(UnitRec {
                    state: if u.enabled { UnitState::Inactive } else { UnitState::Disabled },
                    restarts: 0,
                    next_retry_at: None,
                    epoch: None,
                    build: None,
                    attempts: VecDeque::new(),
                }),
                cfg: u,
                tx,
                rx: Mutex::new(Some(rx)),
            })
        };
        for u in units_cfg.iter().filter(|u| u.role == "log") {
            units.push(mk(u.clone(), Role::Log));
        }
        let z = Unit {
            id: "zenoh".into(),
            kind: UnitKind::Inproc,
            order: 0,
            role: "zenoh".into(),
            addr: None,
            cmd: Vec::new(),
            enabled: true,
        };
        units.push(mk(z, Role::Zenoh));
        let mut rest: Vec<&Unit> = units_cfg.iter().filter(|u| u.role != "log").collect();
        rest.sort_by_key(|u| u.order);
        for u in rest {
            units.push(mk(u.clone(), Role::Sfu));
        }
        Arc::new(Supervisor {
            units,
            cfg,
            stopping: AtomicBool::new(false),
            node_declared: AtomicBool::new(false),
            fatal,
        })
    }

    pub fn unit(&self, id: &str) -> Option<Arc<UnitHandle>> {
        self.units.iter().find(|u| u.id == id).cloned()
    }

    pub fn sfu_units(&self) -> Vec<Arc<UnitHandle>> {
        self.units.iter().filter(|u| u.role == Role::Sfu).cloned().collect()
    }

    pub fn down_units(&self) -> Vec<String> {
        self.units
            .iter()
            .filter(|u| !matches!(u.state(), UnitState::Running | UnitState::Disabled))
            .map(|u| u.id.clone())
            .collect()
    }

    pub fn mark_running(&self, role: Role, cause: &str) {
        for u in self.units.iter().filter(|u| u.role == role) {
            u.set(UnitState::Starting, cause);
            u.set(UnitState::Running, cause);
        }
    }

    pub fn mark_state(&self, role: Role, to: UnitState, cause: &str) {
        for u in self.units.iter().filter(|u| u.role == role) {
            u.set(to, cause);
        }
    }

    pub fn start_sfu(self: &Arc<Self>, bus: Arc<Bus>) {
        for u in self.sfu_units() {
            if u.state() == UnitState::Disabled {
                continue;
            }
            let Some(rx) = u.rx.lock().ok().and_then(|mut g| g.take()) else { continue };
            let me = self.clone();
            let bus = bus.clone();
            tokio::spawn(async move { me.run_unit(u, rx, bus).await });
        }
    }

    fn backoff(&self, restarts: u32) -> Duration {
        let b = &self.cfg.backoff_ms;
        let i = (restarts as usize).min(b.len().saturating_sub(1));
        Duration::from_millis(b.get(i).copied().unwrap_or(1000))
    }

    fn note_attempt(&self, u: &UnitHandle, counted: bool) -> bool {
        let Ok(mut r) = u.rec.lock() else { return false };
        if !counted {
            return false;
        }
        let now = Instant::now();
        r.attempts.push_back(now);
        while let Some(f) = r.attempts.front() {
            if now.duration_since(*f) > self.cfg.intensity_window {
                r.attempts.pop_front();
            } else {
                break;
            }
        }
        r.attempts.len() as u32 > self.cfg.intensity_max
    }

    async fn wait_port_free(&self, u: &UnitHandle) {
        let Some(addr) = &u.addr else { return };
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), tokio::net::TcpStream::connect(addr)).await {
                Ok(Ok(_)) => tokio::time::sleep(Duration::from_millis(100)).await,
                _ => return,
            }
        }
    }

    fn command(&self, u: &UnitHandle) -> Option<tokio::process::Command> {
        let (prog, args) = u.cfg.cmd.split_first()?;
        let mut c = tokio::process::Command::new(prog);
        c.args(args);
        c.args(["--id", &self.cfg.node]);
        if let Some(b) = &self.cfg.build_label {
            c.args(["--build", b]);
        }
        c.args(&self.cfg.extra_args);
        c.stdin(std::process::Stdio::piped());
        c.kill_on_drop(true);
        Some(c)
    }

    async fn hello(&self, u: &UnitHandle) -> Option<(Local, String)> {
        let addr = u.addr.clone()?;
        let deadline = Instant::now() + self.cfg.hello_timeout;
        while Instant::now() < deadline {
            if let Ok((client, epoch, build)) = Bus::hello(&addr, &self.cfg.node, &self.cfg.hub_inst).await {
                return Some((Local { client, epoch, unit: u.id.clone() }, build));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    async fn became_running(&self, u: &UnitHandle, bus: &Bus, local: Local, build: String) {
        if let Ok(mut r) = u.rec.lock() {
            r.epoch = Some(local.epoch.clone());
            r.build = Some(build);
        }
        bus.set_local(Some(local));
        u.set(UnitState::Running, "hello");
        if !self.node_declared.swap(true, Ordering::SeqCst) {
            if let Err(e) = bus.declare_self().await {
                tracing::warn!(kind = "diag", unit = %u.id, "node token: {e}");
            } else {
                common::log::emit(common::log::Line::new(common::log::Kind::Diag).key("unit", &u.id).msg("node token declared"));
            }
        }
    }

    async fn run_unit(self: Arc<Self>, u: Arc<UnitHandle>, mut rx: mpsc::UnboundedReceiver<Cmd>, bus: Arc<Bus>) {
        let mut counted = true;
        let mut first = true;
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            let st = u.state();
            if matches!(st, UnitState::Stopped | UnitState::Blocked | UnitState::Disabled) {
                match rx.recv().await {
                    Some(Cmd::Load(ack)) => {
                        let from = u.set(UnitState::Starting, "load");
                        counted = false;
                        let _ = ack.send(Ok((from, UnitState::Starting)));
                    }
                    Some(Cmd::Stop(ack)) => {
                        let from = u.set(UnitState::Stopped, "stop");
                        let _ = ack.send(Ok((from, UnitState::Stopped)));
                        continue;
                    }
                    Some(Cmd::Kill(ack)) => {
                        let _ = ack.send(Err("not running".into()));
                        continue;
                    }
                    None => return,
                }
            } else if st == UnitState::Down {
                let wait = u.rec.lock().ok().and_then(|r| r.next_retry_at).map(|t| t.saturating_duration_since(Instant::now()));
                let sleep = tokio::time::sleep(wait.unwrap_or_default());
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => {}
                    c = rx.recv() => match c {
                        Some(Cmd::Stop(ack)) => { let from = u.set(UnitState::Stopped, "stop"); let _ = ack.send(Ok((from, UnitState::Stopped))); continue; }
                        Some(Cmd::Load(ack)) => { let _ = ack.send(Err("unit is down — restarting".into())); continue; }
                        Some(Cmd::Kill(ack)) => { let _ = ack.send(Err("not running".into())); continue; }
                        None => return,
                    }
                }
                u.set(UnitState::Starting, "backoff");
            } else {
                u.set(UnitState::Starting, "start");
            }
            if first {
                self.wait_port_free(&u).await;
                first = false;
            }
            let blocked = self.note_attempt(&u, counted);
            counted = true;
            if blocked {
                u.set(UnitState::Blocked, "intensity");
                continue;
            }
            let child = match u.kind {
                UnitKind::Process => match self.command(&u).and_then(|mut c| c.spawn().ok()) {
                    Some(c) => Some(c),
                    None => {
                        self.go_down(&u, "exec failed");
                        continue;
                    }
                },
                _ => None,
            };
            let Some((local, build)) = self.hello(&u).await else {
                if let Some(mut c) = child {
                    let _ = c.start_kill();
                    let _ = c.wait().await;
                }
                self.go_down(&u, "hello timeout");
                continue;
            };
            self.became_running(&u, &bus, local, build).await;
            let outcome = self.supervise(&u, child, &mut rx).await;
            bus.set_local(None);
            if let Ok(mut r) = u.rec.lock() {
                r.epoch = None;
            }
            match outcome {
                Outcome::Exited | Outcome::Killed => {
                    if self.stopping.load(Ordering::SeqCst) {
                        u.set(UnitState::Stopped, "shutdown");
                        return;
                    }
                    self.go_down(&u, if matches!(outcome, Outcome::Killed) { "kill" } else { "exit" });
                }
                Outcome::Stopped => {}
                Outcome::Gone => return,
            }
        }
    }

    fn go_down(&self, u: &UnitHandle, cause: &str) {
        let restarts = u.rec.lock().map(|r| r.restarts).unwrap_or(0);
        let b = self.backoff(restarts);
        if let Ok(mut r) = u.rec.lock() {
            r.restarts += 1;
        }
        u.set(UnitState::Down, cause);
        if let Ok(mut r) = u.rec.lock() {
            r.next_retry_at = Some(Instant::now() + b);
        }
    }

    async fn supervise(&self, u: &UnitHandle, child: Option<tokio::process::Child>, rx: &mut mpsc::UnboundedReceiver<Cmd>) -> Outcome {
        let Some(mut child) = child else {
            loop {
                match rx.recv().await {
                    Some(Cmd::Stop(ack)) => {
                        let from = u.set(UnitState::Stopped, "stop");
                        let _ = ack.send(Ok((from, UnitState::Stopped)));
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Kill(ack)) => {
                        let _ = ack.send(Ok((UnitState::Running, UnitState::Down)));
                        return Outcome::Killed;
                    }
                    Some(Cmd::Load(ack)) => {
                        let _ = ack.send(Err("already running".into()));
                    }
                    None => return Outcome::Gone,
                }
            }
        };
        let _parent_channel = child.stdin.take();
        loop {
            tokio::select! {
                _ = child.wait() => return Outcome::Exited,
                c = rx.recv() => match c {
                    Some(Cmd::Stop(ack)) => {
                        let from = u.set(UnitState::Stopping, "stop");
                        terminate(&mut child, Duration::from_secs(5)).await;
                        u.set(UnitState::Stopped, "stop");
                        let _ = ack.send(Ok((from, UnitState::Stopped)));
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Kill(ack)) => {
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                        let _ = ack.send(Ok((UnitState::Running, UnitState::Down)));
                        return Outcome::Killed;
                    }
                    Some(Cmd::Load(ack)) => { let _ = ack.send(Err("already running".into())); }
                    None => {
                        terminate(&mut child, Duration::from_secs(5)).await;
                        return Outcome::Gone;
                    }
                }
            }
        }
    }

    pub async fn request(&self, id: &str, which: &str) -> Result<(UnitState, UnitState), String> {
        let u = self.unit(id).ok_or_else(|| "no unit".to_string())?;
        if u.role == Role::Zenoh || u.role == Role::Log {
            if which == "load" {
                return Err("inproc unit".into());
            }
            let from = u.state();
            if u.role == Role::Zenoh {
                let _ = self.fatal.send(format!("zenoh unit {which}"));
            }
            return Ok((from, if which == "kill" { UnitState::Down } else { UnitState::Stopped }));
        }
        let (tx, rx) = oneshot::channel();
        let cmd = match which {
            "load" => Cmd::Load(tx),
            "stop" => Cmd::Stop(tx),
            _ => Cmd::Kill(tx),
        };
        u.tx.send(cmd).map_err(|_| "unit loop gone".to_string())?;
        rx.await.map_err(|_| "no answer".to_string())?
    }

    pub async fn stop_all(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let mut units: Vec<Arc<UnitHandle>> = self.sfu_units();
        units.reverse();
        for u in units {
            if u.state() == UnitState::Running {
                let _ = self.request(&u.id, "stop").await;
            }
        }
    }
}

enum Outcome {
    Exited,
    Killed,
    Stopped,
    Gone,
}

async fn terminate(child: &mut tokio::process::Child, grace: Duration) {
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
    }
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}
