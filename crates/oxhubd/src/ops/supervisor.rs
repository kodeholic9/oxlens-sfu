// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-SUPERVISOR 정:RULE-OPS-SURFACE 정:RULE-OPS-LOGGER 정:RULE-BUS-FATE 정:IMPL-BUS-FAULTS 정:IMPL-CFG-SYSTEM-FILE 정:CAND-OPS-SUPERVISOR-DEFAULTS 정:CAND-OPS-SHUTDOWN-NO-RESTART 정:CAND-OPS-SUPERVISOR-EXTRA-TRANSITIONS 정:CAND-OPS-ZENOH-UNIT-DOWN 운영:RULE-CTL-SUPERVISOR 정:CAND-OPS-UNIT-STATE-LINE · model: claude-opus-5-5
//! supervisor — 유닛 셋 종류(process · remote · inproc) · `UnitState` 여덟 · backoff · 폭주 창 · 기동 핸드셰이크(sfu = Hello · 로거 = `ready` 줄) · 부모 생존 채널(stdin).

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use common::handle::{Client, HelloReq};
use common::system::{UnitKind, UnitRole};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, Notify};

use crate::bus::Bus;
use crate::cfg::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UState {
    Disabled,
    Inactive,
    Starting,
    Running,
    Stopping,
    Stopped,
    Down,
    Blocked,
}

impl UState {
    pub fn name(self) -> &'static str {
        match self {
            UState::Disabled => "Disabled",
            UState::Inactive => "Inactive",
            UState::Starting => "Starting",
            UState::Running => "Running",
            UState::Stopping => "Stopping",
            UState::Stopped => "Stopped",
            UState::Down => "Down",
            UState::Blocked => "Blocked",
        }
    }
}

pub const ZENOH_UNIT: &str = "zenoh";

pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    pub role: Option<UnitRole>,
    pub order: u32,
    pub addr: Option<String>,
    pub cmd: Vec<String>,
    pub state: UState,
    pub restarts: u32,
    pub next_retry: Option<Instant>,
    pub epoch: Option<String>,
    pub client: Option<Client>,
    attempts: VecDeque<Instant>,
    stdin: Option<tokio::process::ChildStdin>,
    kill: Option<Arc<Notify>>,
    started_once: bool,
}

impl Unit {
    fn rank(&self) -> (u8, u32) {
        match self.role {
            Some(UnitRole::Log) => (0, self.order),
            None => (1, self.order),
            Some(UnitRole::Sfu) => (2, self.order),
        }
    }
}

pub struct UnitView {
    pub id: String,
    pub kind: UnitKind,
    pub role: Option<UnitRole>,
    pub order: u32,
    pub addr: Option<String>,
    pub state: UState,
    pub restarts: u32,
    pub next_retry_ms: Option<u64>,
    pub epoch: Option<String>,
}

pub struct Supervisor {
    cfg: Config,
    hub_inst: String,
    units: Mutex<Vec<Unit>>,
    bus: OnceLock<Arc<Bus>>,
    stopping: std::sync::atomic::AtomicBool,
    zenoh_down: Notify,
}

fn transition(u: &mut Unit, to: UState) {
    let from = u.state;
    u.state = to;
    common::ev!(spec = "SV-STATE-UNIT-STATE", "unit={} from={} to={}", u.id, from.name(), to.name());
}

impl Supervisor {
    pub fn new(cfg: &Config, hub_inst: &str) -> Arc<Supervisor> {
        let mut units: Vec<Unit> = cfg
            .sys
            .unit
            .iter()
            .map(|u| Unit {
                id: u.id.clone(),
                kind: u.kind,
                role: Some(u.role),
                order: u.order,
                addr: u.addr.clone(),
                cmd: u.cmd.clone(),
                state: if u.enabled { UState::Inactive } else { UState::Disabled },
                restarts: 0,
                next_retry: None,
                epoch: None,
                client: None,
                attempts: VecDeque::new(),
                stdin: None,
                kill: None,
                started_once: false,
            })
            .collect();
        units.push(Unit {
            id: ZENOH_UNIT.to_string(),
            kind: UnitKind::Inproc,
            role: None,
            order: 0,
            addr: None,
            cmd: Vec::new(),
            state: UState::Inactive,
            restarts: 0,
            next_retry: None,
            epoch: None,
            client: None,
            attempts: VecDeque::new(),
            stdin: None,
            kill: None,
            started_once: false,
        });
        units.sort_by_key(|u| u.rank());
        Arc::new(Supervisor {
            cfg: cfg.clone(),
            hub_inst: hub_inst.to_string(),
            units: Mutex::new(units),
            bus: OnceLock::new(),
            stopping: std::sync::atomic::AtomicBool::new(false),
            zenoh_down: Notify::new(),
        })
    }

    pub fn set_bus(&self, bus: Arc<Bus>) {
        let _ = self.bus.set(bus);
    }

    pub async fn zenoh_running(&self) {
        let mut g = self.units.lock().await;
        if let Some(u) = g.iter_mut().find(|u| u.id == ZENOH_UNIT) {
            transition(u, UState::Starting);
            transition(u, UState::Running);
            u.epoch = Some(self.hub_inst.clone());
        }
    }

    pub async fn zenoh_down_wait(&self) {
        self.zenoh_down.notified().await;
    }

    pub fn stopping(&self) -> bool {
        self.stopping.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn views(&self) -> Vec<UnitView> {
        self.units
            .lock()
            .await
            .iter()
            .map(|u| UnitView {
                id: u.id.clone(),
                kind: u.kind,
                role: u.role,
                order: u.order,
                addr: u.addr.clone(),
                state: u.state,
                restarts: u.restarts,
                next_retry_ms: if u.state == UState::Down {
                    u.next_retry.map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u64)
                } else {
                    None
                },
                epoch: if u.state == UState::Running && u.kind != UnitKind::Inproc { u.epoch.clone() } else { None },
            })
            .collect()
    }

    pub fn has_epoch(&self, inst: &str) -> bool {
        self.units.try_lock().map(|g| g.iter().any(|u| u.epoch.as_deref() == Some(inst) && u.role == Some(UnitRole::Sfu))).unwrap_or(false)
    }

    pub async fn sfu_client(&self, inst: Option<&str>) -> Option<Client> {
        let g = self.units.lock().await;
        g.iter()
            .filter(|u| u.role == Some(UnitRole::Sfu) && u.state == UState::Running)
            .find(|u| inst.is_none() || u.epoch.as_deref() == inst)
            .and_then(|u| u.client.clone())
    }

    pub async fn sfu_clients(&self) -> Vec<(String, Option<String>, Option<Client>)> {
        let g = self.units.lock().await;
        g.iter()
            .filter(|u| u.role == Some(UnitRole::Sfu))
            .map(|u| (u.id.clone(), u.epoch.clone(), if u.state == UState::Running { u.client.clone() } else { None }))
            .collect()
    }

    pub async fn ready(&self) -> (bool, Vec<String>) {
        let g = self.units.lock().await;
        let down: Vec<String> = g.iter().filter(|u| u.state != UState::Disabled && u.state != UState::Running).map(|u| u.id.clone()).collect();
        (down.is_empty(), down)
    }

    pub async fn start_role(self: &Arc<Self>, role: Option<UnitRole>) {
        let ids: Vec<String> = self.units.lock().await.iter().filter(|u| u.role == role && u.state == UState::Inactive).map(|u| u.id.clone()).collect();
        for id in ids {
            self.clone().launch(id, true).await;
        }
    }

    async fn wait_port_free(&self, addr: &str) {
        let t0 = Instant::now();
        loop {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => {
                    drop(l);
                    return;
                }
                Err(_) if t0.elapsed() < Duration::from_secs(30) => {
                    tracing::info!("waiting for previous unit to release {addr}");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => {
                    tracing::warn!("port {addr} still held after 30s: {e}");
                    return;
                }
            }
        }
    }

    fn launch(self: Arc<Self>, id: String, first: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            if self.stopping() {
                return;
            }
            let (kind, role, addr, cmd) = {
                let g = self.units.lock().await;
                let Some(u) = g.iter().find(|u| u.id == id) else {
                    return;
                };
                (u.kind, u.role, u.addr.clone(), u.cmd.clone())
            };
            if first
                && role == Some(UnitRole::Sfu)
                && kind == UnitKind::Process
                && let Some(a) = &addr
            {
                self.wait_port_free(a).await;
            }
            {
                let mut g = self.units.lock().await;
                let Some(u) = g.iter_mut().find(|u| u.id == id) else {
                    return;
                };
                let now = Instant::now();
                let win = Duration::from_secs(self.cfg.sys.supervisor.intensity_window_secs);
                while u.attempts.front().is_some_and(|t| now.duration_since(*t) > win) {
                    u.attempts.pop_front();
                }
                u.attempts.push_back(now);
                if u.started_once {
                    u.restarts += 1;
                }
                u.started_once = true;
                u.next_retry = None;
                transition(u, UState::Starting);
            }
            match kind {
                UnitKind::Process => self.spawn_process(&id, role, addr, cmd).await,
                UnitKind::Remote => self.attach_remote(&id, addr).await,
                UnitKind::Inproc => {}
            }
        })
    }

    async fn spawn_process(self: &Arc<Self>, id: &str, role: Option<UnitRole>, addr: Option<String>, cmd: Vec<String>) {
        let Some((prog, args)) = cmd.split_first() else {
            self.clone().down(id.to_string()).await;
            return;
        };
        let mut c = tokio::process::Command::new(prog);
        c.args(args);
        c.arg("--id").arg(&self.cfg.node_id);
        if let Some(b) = self.cfg.build.split_once('+').map(|(_, l)| l) {
            c.arg("--build").arg(b);
        }
        c.arg("--zenoh-namespace").arg(&self.cfg.sys.zenoh.namespace);
        if let Some(l) = self.cfg.sys.zenoh.listen.first() {
            c.arg("--zenoh-connect").arg(l);
        }
        if let Some(d) = &self.cfg.log_dir {
            c.arg("--log-dir").arg(d);
        }
        if let Some(s) = &self.cfg.log_socket {
            c.arg("--log-socket").arg(s);
        }
        c.stdin(Stdio::piped());
        c.stdout(if role == Some(UnitRole::Log) { Stdio::piped() } else { Stdio::inherit() });
        c.stderr(Stdio::inherit());
        c.kill_on_drop(false);
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                tracing::warn!("unit {id} exec failed: {e}");
                self.clone().down(id.to_string()).await;
                return;
            }
        };
        let stdin = child.stdin.take();
        if role == Some(UnitRole::Log)
            && let Some(out) = child.stdout.take()
        {
            self.take_log_stdout(id.to_string(), out);
        }
        let killer = Arc::new(Notify::new());
        {
            let mut g = self.units.lock().await;
            if let Some(u) = g.iter_mut().find(|u| u.id == id) {
                u.stdin = stdin;
                u.kill = Some(killer.clone());
                u.epoch = None;
                u.client = None;
            }
        }
        let me = self.clone();
        let idw = id.to_string();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = killer.notified() => {
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
            };
            tracing::info!("unit {idw} exited {status:?}");
            me.exited(idw).await;
        });
        match role {
            Some(UnitRole::Log) => self.wait_log_ready(id).await,
            _ => self.hello(id, addr).await,
        }
    }

    async fn wait_log_ready(self: &Arc<Self>, id: &str) {
        let deadline = Duration::from_millis(self.cfg.sys.supervisor.hello_timeout_ms);
        let t0 = Instant::now();
        loop {
            if t0.elapsed() > deadline {
                tracing::warn!("unit {id} gave no ready line");
                self.kill_now(id).await;
                return;
            }
            let ready = {
                let g = self.units.lock().await;
                g.iter().find(|u| u.id == id).map(|u| u.epoch.is_some() || u.state != UState::Starting).unwrap_or(true)
            };
            if ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn take_log_stdout(self: &Arc<Self>, id: String, out: tokio::process::ChildStdout) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(e) = l.strip_prefix("ready epoch=") {
                    let mut g = me.units.lock().await;
                    if let Some(u) = g.iter_mut().find(|u| u.id == id)
                        && u.state == UState::Starting
                    {
                        u.epoch = Some(e.trim().to_string());
                        transition(u, UState::Running);
                    }
                }
            }
        });
    }

    async fn hello(self: &Arc<Self>, id: &str, addr: Option<String>) {
        let Some(addr) = addr else {
            self.kill_now(id).await;
            return;
        };
        let deadline = Duration::from_millis(self.cfg.sys.supervisor.hello_timeout_ms);
        let t0 = Instant::now();
        loop {
            {
                let g = self.units.lock().await;
                if g.iter().find(|u| u.id == id).is_none_or(|u| u.state != UState::Starting) {
                    return;
                }
            }
            if t0.elapsed() > deadline {
                tracing::warn!("unit {id} hello timed out");
                self.kill_now(id).await;
                return;
            }
            if let Ok(mut c) = common::handle::connect(&addr, Duration::from_millis(500)).await {
                let req = HelloReq { node: self.cfg.node_id.clone(), hub_inst: self.hub_inst.clone() };
                if let Ok(r) = c.hello(req).await {
                    let r = r.into_inner();
                    let first_sfu = {
                        let mut g = self.units.lock().await;
                        let Some(u) = g.iter_mut().find(|u| u.id == id) else {
                            return;
                        };
                        if u.state != UState::Starting {
                            return;
                        }
                        u.epoch = Some(r.epoch.clone());
                        u.client = Some(c);
                        transition(u, UState::Running);
                        tracing::info!("unit {id} running epoch={} build={}", r.epoch, r.build);
                        u.role == Some(UnitRole::Sfu)
                    };
                    if first_sfu
                        && let Some(b) = self.bus.get()
                        && let Err(e) = b.declare_node().await
                    {
                        tracing::warn!("node token: {e}");
                    }
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn attach_remote(self: &Arc<Self>, id: &str, addr: Option<String>) {
        self.hello(id, addr).await;
    }

    async fn kill_now(&self, id: &str) {
        let k = self.units.lock().await.iter().find(|u| u.id == id).and_then(|u| u.kill.clone());
        if let Some(k) = k {
            k.notify_one();
        }
    }

    async fn exited(self: Arc<Self>, id: String) {
        let stopped = {
            let mut g = self.units.lock().await;
            let Some(u) = g.iter_mut().find(|u| u.id == id) else {
                return;
            };
            u.stdin = None;
            u.kill = None;
            u.client = None;
            u.epoch = None;
            if u.state == UState::Stopping || u.state == UState::Stopped {
                if u.state == UState::Stopping {
                    transition(u, UState::Stopped);
                }
                true
            } else {
                transition(u, UState::Down);
                false
            }
        };
        if !stopped {
            self.down(id).await;
        }
    }

    fn down(self: Arc<Self>, id: String) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let wait = {
                let mut g = self.units.lock().await;
                let Some(u) = g.iter_mut().find(|u| u.id == id) else {
                    return;
                };
                if u.state != UState::Down {
                    transition(u, UState::Down);
                }
                if self.stopping() {
                    return;
                }
                let sup = &self.cfg.sys.supervisor;
                let now = Instant::now();
                let win = Duration::from_secs(sup.intensity_window_secs);
                while u.attempts.front().is_some_and(|t| now.duration_since(*t) > win) {
                    u.attempts.pop_front();
                }
                if u.attempts.len() as u32 >= sup.intensity_max {
                    transition(u, UState::Blocked);
                    if u.id == ZENOH_UNIT {
                        self.zenoh_down.notify_waiters();
                    }
                    return;
                }
                let idx = (u.attempts.len().saturating_sub(1)).min(sup.backoff_ms.len().saturating_sub(1));
                let ms = sup.backoff_ms.get(idx).copied().unwrap_or(1000);
                u.next_retry = Some(now + Duration::from_millis(ms));
                Duration::from_millis(ms)
            };
            tokio::time::sleep(wait).await;
            let still = self.units.lock().await.iter().find(|u| u.id == id).is_some_and(|u| u.state == UState::Down);
            if still && !self.stopping() {
                self.clone().launch(id, false).await;
            }
        })
    }

    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<(UState, UState), oxsig::Code> {
        if id == ZENOH_UNIT {
            let from = self.state_of(id).await.ok_or(oxsig::Code::RoomNotFound)?;
            self.zenoh_down.notify_waiters();
            return Ok((from, UState::Stopped));
        }
        let mut g = self.units.lock().await;
        let u = g.iter_mut().find(|u| u.id == id).ok_or(oxsig::Code::RoomNotFound)?;
        let from = u.state;
        match u.state {
            UState::Running | UState::Starting => {
                transition(u, UState::Stopping);
                u.stdin = None;
                let k = u.kill.clone();
                drop(g);
                if let Some(k) = k {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        k.notify_one();
                    });
                }
                let until = Instant::now() + Duration::from_secs(7);
                while Instant::now() < until && self.state_of(id).await == Some(UState::Stopping) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            UState::Down | UState::Blocked | UState::Inactive => transition(u, UState::Stopped),
            _ => {}
        }
        Ok((from, UState::Stopped))
    }

    pub async fn load(self: &Arc<Self>, id: &str) -> Result<UState, oxsig::Code> {
        let from = {
            let mut g = self.units.lock().await;
            let u = g.iter_mut().find(|u| u.id == id).ok_or(oxsig::Code::RoomNotFound)?;
            if u.state != UState::Stopped && u.state != UState::Blocked {
                return Err(oxsig::Code::PreconditionFailed);
            }
            u.attempts.clear();
            u.state
        };
        self.clone().launch(id.to_string(), false).await;
        Ok(from)
    }

    pub async fn kill(self: &Arc<Self>, id: &str, if_epoch: &str) -> Result<UState, (oxsig::Code, Option<String>)> {
        let (from, epoch, k) = {
            let g = self.units.lock().await;
            let u = g.iter().find(|u| u.id == id).ok_or((oxsig::Code::RoomNotFound, None))?;
            (u.state, u.epoch.clone(), u.kill.clone())
        };
        if epoch.as_deref() != Some(if_epoch) {
            return Err((oxsig::Code::PreconditionFailed, epoch));
        }
        if id == ZENOH_UNIT {
            self.zenoh_down.notify_waiters();
            return Ok(from);
        }
        if let Some(k) = k {
            k.notify_one();
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            let gone = self.units.lock().await.iter().find(|u| u.id == id).is_none_or(|u| u.epoch.as_deref() != Some(if_epoch));
            if gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(from)
    }

    pub async fn state_of(&self, id: &str) -> Option<UState> {
        self.units.lock().await.iter().find(|u| u.id == id).map(|u| u.state)
    }

    pub async fn shutdown_units(self: &Arc<Self>) {
        self.stopping.store(true, std::sync::atomic::Ordering::Relaxed);
        let ids: Vec<(String, Option<UnitRole>)> = {
            let g = self.units.lock().await;
            g.iter().rev().filter(|u| u.kind != UnitKind::Inproc).map(|u| (u.id.clone(), u.role)).collect()
        };
        for (id, role) in ids {
            if role == Some(UnitRole::Log) {
                continue;
            }
            self.stop_and_wait(&id).await;
        }
    }

    pub async fn stop_logger(self: &Arc<Self>) {
        let ids: Vec<String> = self.units.lock().await.iter().filter(|u| u.role == Some(UnitRole::Log)).map(|u| u.id.clone()).collect();
        for id in ids {
            self.stop_and_wait(&id).await;
        }
    }

    async fn stop_and_wait(self: &Arc<Self>, id: &str) {
        let _ = self.stop(id).await;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(6) {
            match self.state_of(id).await {
                Some(UState::Stopping) | Some(UState::Starting) => tokio::time::sleep(Duration::from_millis(50)).await,
                _ => return,
            }
        }
    }
}
