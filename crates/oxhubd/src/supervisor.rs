// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-OPS-SUPERVISOR 정:RULE-OPS-SURFACE 정:RULE-BUS-FATE 정:IMPL-CFG-SYSTEM-FILE · 운영 규격서 §4-1 · model: claude-opus-5-5
//! supervisor — 상태 여덟 · 종류 셋(process · remote · inproc) · 부모 생존 채널 · 기동 핸드셰이크(Hello) · backoff · 폭주.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::rpc::UnitClient;
use common::system::{System, UnitKind};
use oxsig::Code;
use oxsig::bplane::{UnitReq, UnitRes};
use serde::Serialize;
use tokio::sync::Notify;
use tonic::transport::Channel;

const BACKOFF_CAP_MS: u64 = 30_000;
const HELLO_POLL_MS: u64 = 200;
const REMOTE_PROBE_MS: u64 = 1_000;
const STOP_GRACE_MS: u64 = 5_000;
pub const ZENOH_UNIT: &str = "zenoh";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    None,
    Stop,
    Load,
    Kill,
}

pub struct Slot {
    pub id: String,
    pub kind: UnitKind,
    pub order: u32,
    pub role: String,
    pub addr: Option<String>,
    pub cmd: Vec<String>,
    pub state: UnitState,
    pub epoch: Option<String>,
    pub restarts: u32,
    pub next_retry: Option<Instant>,
    pub pid: Option<u32>,
    pub starts: VecDeque<Instant>,
    pub fails: u32,
    pub client: Option<UnitClient<Channel>>,
    cmd_req: Cmd,
}

pub struct Supervisor {
    pub slots: Arc<Mutex<Vec<Slot>>>,
    wake: Arc<Vec<Notify>>,
    backoff_ms: u64,
    intensity_max: u32,
    intensity_window: Duration,
    hello_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitView {
    pub id: String,
    pub kind: &'static str,
    pub order: u32,
    pub role: String,
    pub addr: Option<String>,
    pub state: UnitState,
    pub restarts: u32,
    pub next_retry_ms: Option<u64>,
    pub epoch: Option<String>,
}

pub struct StartCtx {
    pub node: String,
    pub build_label: Option<String>,
    pub zenoh_namespace: String,
    pub zenoh_connect: Option<String>,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
}

fn kind_name(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Process => "process",
        UnitKind::Remote => "remote",
        UnitKind::Inproc => "inproc",
    }
}

impl Supervisor {
    pub fn new(system: &System) -> Supervisor {
        let mut slots = vec![Slot {
            id: ZENOH_UNIT.into(),
            kind: UnitKind::Inproc,
            order: 0,
            role: "bus".into(),
            addr: None,
            cmd: vec![],
            state: UnitState::Inactive,
            epoch: None,
            restarts: 0,
            next_retry: None,
            pid: None,
            starts: VecDeque::new(),
            fails: 0,
            client: None,
            cmd_req: Cmd::None,
        }];
        for u in &system.unit {
            slots.push(Slot {
                id: u.id.clone(),
                kind: u.kind,
                order: u.order.max(1),
                role: u.role.clone(),
                addr: u.addr.clone(),
                cmd: u.cmd.clone(),
                state: if u.enabled { UnitState::Inactive } else { UnitState::Disabled },
                epoch: None,
                restarts: 0,
                next_retry: None,
                pid: None,
                starts: VecDeque::new(),
                fails: 0,
                client: None,
                cmd_req: Cmd::None,
            });
        }
        slots.sort_by_key(|s| s.order);
        let wake = Arc::new((0..slots.len()).map(|_| Notify::new()).collect());
        Supervisor {
            slots: Arc::new(Mutex::new(slots)),
            wake,
            backoff_ms: system.supervisor.backoff_ms,
            intensity_max: system.supervisor.intensity_max,
            intensity_window: Duration::from_secs(system.supervisor.intensity_window_secs),
            hello_timeout: Duration::from_millis(system.supervisor.hello_timeout_ms),
        }
    }

    pub fn views(&self) -> Vec<UnitView> {
        let now = Instant::now();
        self.slots
            .lock()
            .map(|s| {
                s.iter()
                    .map(|u| UnitView {
                        id: u.id.clone(),
                        kind: kind_name(u.kind),
                        order: u.order,
                        role: u.role.clone(),
                        addr: u.addr.clone(),
                        state: u.state,
                        restarts: u.restarts,
                        next_retry_ms: u.next_retry.map(|t| t.saturating_duration_since(now).as_millis() as u64),
                        epoch: u.epoch.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_state(&self, id: &str, st: UnitState) {
        if let Ok(mut s) = self.slots.lock()
            && let Some(u) = s.iter_mut().find(|u| u.id == id) {
                u.state = st;
            }
    }

    pub fn state_of(&self, id: &str) -> Option<UnitState> {
        self.slots.lock().ok()?.iter().find(|u| u.id == id).map(|u| u.state)
    }

    pub fn sfu_id(&self) -> Option<String> {
        self.slots.lock().ok()?.iter().find(|u| u.role == "sfu").map(|u| u.id.clone())
    }

    pub fn sfu_epoch(&self) -> Option<String> {
        self.slots.lock().ok()?.iter().find(|u| u.role == "sfu" && u.state == UnitState::Running).and_then(|u| u.epoch.clone())
    }

    pub fn down_units(&self) -> Vec<String> {
        self.slots
            .lock()
            .map(|s| s.iter().filter(|u| u.state != UnitState::Disabled && u.state != UnitState::Running).map(|u| u.id.clone()).collect())
            .unwrap_or_default()
    }

    pub fn supervising(&self) -> bool {
        self.slots.lock().map(|s| s.iter().any(|u| u.kind != UnitKind::Inproc && u.state != UnitState::Disabled)).unwrap_or(false)
    }

    pub async fn sfu_call(&self, req: &UnitReq, timeout: Duration) -> Result<UnitRes, Code> {
        let (client, id) = {
            let s = self.slots.lock().map_err(|_| Code::INTERNAL_ERROR)?;
            let u = s.iter().find(|u| u.role == "sfu").ok_or(Code::SFU_UNAVAILABLE)?;
            if u.state != UnitState::Running {
                return Err(Code::SFU_UNAVAILABLE);
            }
            (u.client.clone(), u.id.clone())
        };
        self.call_slot(&id, client, req, timeout).await
    }

    pub async fn call_unit(&self, id: &str, req: &UnitReq, timeout: Duration) -> Result<UnitRes, Code> {
        let client = {
            let s = self.slots.lock().map_err(|_| Code::INTERNAL_ERROR)?;
            let u = s.iter().find(|u| u.id == id).ok_or(Code::SFU_UNAVAILABLE)?;
            if u.state != UnitState::Running {
                return Err(Code::SFU_UNAVAILABLE);
            }
            u.client.clone()
        };
        self.call_slot(id, client, req, timeout).await
    }

    async fn call_slot(
        &self,
        id: &str,
        client: Option<UnitClient<Channel>>,
        req: &UnitReq,
        timeout: Duration,
    ) -> Result<UnitRes, Code> {
        let mut c = match client {
            Some(c) => c,
            None => {
                let addr = self.slots.lock().ok().and_then(|s| s.iter().find(|u| u.id == id).and_then(|u| u.addr.clone()));
                let addr = addr.ok_or(Code::SFU_UNAVAILABLE)?;
                let c = common::rpc::connect(&addr).await.map_err(|_| Code::SFU_UNAVAILABLE)?;
                if let Ok(mut s) = self.slots.lock()
                    && let Some(u) = s.iter_mut().find(|u| u.id == id) {
                        u.client = Some(c.clone());
                    }
                c
            }
        };
        match common::rpc::call(&mut c, req, timeout).await {
            Ok(r) => Ok(r),
            Err(e) if e == "timeout" => Err(Code::SFU_ERROR),
            Err(_) => Err(Code::SFU_UNAVAILABLE),
        }
    }

    fn command(&self, id: &str, cmd: Cmd) -> Result<(UnitState, usize), Code> {
        let mut s = self.slots.lock().map_err(|_| Code::INTERNAL_ERROR)?;
        let idx = s.iter().position(|u| u.id == id).ok_or(Code::SFU_UNAVAILABLE)?;
        let from = s[idx].state;
        s[idx].cmd_req = cmd;
        Ok((from, idx))
    }

    pub fn stop(&self, id: &str) -> Result<UnitState, Code> {
        let (from, idx) = self.command(id, Cmd::Stop)?;
        self.wake[idx].notify_one();
        Ok(from)
    }

    pub fn load(&self, id: &str) -> Result<UnitState, Code> {
        let (from, idx) = self.command(id, Cmd::Load)?;
        self.wake[idx].notify_one();
        Ok(from)
    }

    pub fn kill(&self, id: &str, if_epoch: &str) -> Result<UnitState, Code> {
        {
            let s = self.slots.lock().map_err(|_| Code::INTERNAL_ERROR)?;
            let u = s.iter().find(|u| u.id == id).ok_or(Code::SFU_UNAVAILABLE)?;
            if u.epoch.as_deref() != Some(if_epoch) {
                return Err(Code::PRECONDITION_FAILED);
            }
        }
        let (from, idx) = self.command(id, Cmd::Kill)?;
        self.wake[idx].notify_one();
        Ok(from)
    }

    pub fn epoch_of(&self, id: &str) -> Option<String> {
        self.slots.lock().ok()?.iter().find(|u| u.id == id).and_then(|u| u.epoch.clone())
    }

    pub fn start(self: &Arc<Self>, ctx: Arc<StartCtx>, on_running: Arc<dyn Fn(String, String) + Send + Sync>) {
        let n = self.slots.lock().map(|s| s.len()).unwrap_or(0);
        for idx in 0..n {
            let (kind, state) = match self.slots.lock() {
                Ok(s) => (s[idx].kind, s[idx].state),
                Err(_) => continue,
            };
            if kind == UnitKind::Inproc || state == UnitState::Disabled {
                continue;
            }
            let me = self.clone();
            let ctx = ctx.clone();
            let cb = on_running.clone();
            tokio::spawn(async move {
                match kind {
                    UnitKind::Process => me.run_process(idx, ctx, cb).await,
                    UnitKind::Remote => me.run_remote(idx, cb).await,
                    UnitKind::Inproc => {}
                }
            });
        }
    }

    fn take_cmd(&self, idx: usize) -> Cmd {
        match self.slots.lock() {
            Ok(mut s) => std::mem::replace(&mut s[idx].cmd_req, Cmd::None),
            Err(_) => Cmd::None,
        }
    }

    fn with<R>(&self, idx: usize, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
        self.slots.lock().ok().map(|mut s| f(&mut s[idx]))
    }

    fn record_start(&self, idx: usize, counted: bool) -> bool {
        let window = self.intensity_window;
        let max = self.intensity_max;
        self.with(idx, |u| {
            let now = Instant::now();
            while u.starts.front().is_some_and(|t| now.duration_since(*t) > window) {
                u.starts.pop_front();
            }
            if counted {
                u.starts.push_back(now);
            }
            u.starts.len() as u32 > max
        })
        .unwrap_or(false)
    }

    async fn hello(&self, idx: usize) -> Option<String> {
        let addr = self.with(idx, |u| u.addr.clone()).flatten()?;
        let deadline = Instant::now() + self.hello_timeout;
        while Instant::now() < deadline {
            if let Ok(mut c) = common::rpc::connect(&addr).await
                && let Ok(UnitRes::Hello(h)) = common::rpc::call(&mut c, &UnitReq::Hello, Duration::from_millis(1_000)).await {
                    self.with(idx, |u| u.client = Some(c.clone()));
                    return Some(h.epoch);
                }
            tokio::time::sleep(Duration::from_millis(HELLO_POLL_MS)).await;
        }
        None
    }

    async fn wait_cmd(&self, idx: usize, want: &[Cmd]) -> Cmd {
        loop {
            let c = self.take_cmd(idx);
            if want.contains(&c) {
                return c;
            }
            self.wake[idx].notified().await;
        }
    }

    async fn run_process(self: Arc<Self>, idx: usize, ctx: Arc<StartCtx>, on_running: Arc<dyn Fn(String, String) + Send + Sync>) {
        let mut counted = true;
        loop {
            let (id, cmd) = match self.with(idx, |u| (u.id.clone(), u.cmd.clone())) {
                Some(v) => v,
                None => return,
            };
            if self.record_start(idx, counted) {
                self.with(idx, |u| u.state = UnitState::Blocked);
                tracing::warn!(kind = "ev", spec = "SV-STATE-BLOCKED", unit = id.as_str(), "unit blocked");
                self.wait_cmd(idx, &[Cmd::Load]).await;
                self.with(idx, |u| u.starts.clear());
                counted = false;
                continue;
            }
            counted = true;
            self.with(idx, |u| {
                u.state = UnitState::Starting;
                u.epoch = None;
                u.client = None;
                u.next_retry = None;
            });
            let mut c = tokio::process::Command::new(cmd.first().cloned().unwrap_or_default());
            c.args(cmd.iter().skip(1));
            c.arg("--id").arg(&ctx.node);
            if let Some(l) = &ctx.build_label {
                c.arg("--build").arg(l);
            }
            c.arg("--zenoh-namespace").arg(&ctx.zenoh_namespace);
            if let Some(z) = &ctx.zenoh_connect {
                c.arg("--zenoh-connect").arg(z);
            }
            if let Some(d) = &ctx.log_dir {
                c.arg("--log-dir").arg(d);
            }
            if let Some(s) = &ctx.log_socket {
                c.arg("--log-socket").arg(s);
            }
            c.stdin(std::process::Stdio::piped());
            let mut child = match c.spawn() {
                Ok(ch) => ch,
                Err(e) => {
                    tracing::warn!(kind = "diag", unit = id.as_str(), "spawn failed: {e}");
                    self.down_and_backoff(idx).await;
                    continue;
                }
            };
            let _liveness = child.stdin.take();
            self.with(idx, |u| u.pid = child.id());
            let epoch = tokio::select! {
                e = self.hello(idx) => e,
                _ = child.wait() => None,
            };
            let Some(epoch) = epoch else {
                let _ = child.start_kill();
                let _ = child.wait().await;
                self.down_and_backoff(idx).await;
                continue;
            };
            self.with(idx, |u| {
                u.state = UnitState::Running;
                u.epoch = Some(epoch.clone());
                u.fails = 0;
            });
            tracing::info!(kind = "ev", spec = "SV-STATE-RUNNING", unit = id.as_str(), epoch = epoch.as_str(), "unit running");
            on_running(id.clone(), epoch);
            loop {
                tokio::select! {
                    _ = child.wait() => break,
                    _ = self.wake[idx].notified() => {
                        match self.take_cmd(idx) {
                            Cmd::Stop => {
                                self.with(idx, |u| u.state = UnitState::Stopping);
                                signal(child.id(), nix::sys::signal::Signal::SIGTERM);
                                if tokio::time::timeout(Duration::from_millis(STOP_GRACE_MS), child.wait()).await.is_err() {
                                    let _ = child.start_kill();
                                    let _ = child.wait().await;
                                }
                                break;
                            }
                            Cmd::Kill => {
                                let _ = child.start_kill();
                                let _ = child.wait().await;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            let stopping = self.with(idx, |u| u.state == UnitState::Stopping).unwrap_or(false);
            self.with(idx, |u| {
                u.pid = None;
                u.client = None;
            });
            if stopping {
                self.with(idx, |u| u.state = UnitState::Stopped);
                tracing::info!(kind = "ev", spec = "SV-STATE-STOPPED", unit = id.as_str(), "unit stopped");
                self.wait_cmd(idx, &[Cmd::Load]).await;
                counted = false;
                continue;
            }
            tracing::warn!(kind = "ev", spec = "SV-STATE-DOWN", unit = id.as_str(), "unit down");
            self.with(idx, |u| u.restarts = u.restarts.saturating_add(1));
            self.down_and_backoff(idx).await;
        }
    }

    async fn down_and_backoff(&self, idx: usize) {
        let base = self.backoff_ms.max(1);
        let wait = self
            .with(idx, |u| {
                u.state = UnitState::Down;
                u.fails = u.fails.saturating_add(1);
                let shift = u.fails.saturating_sub(1).min(16);
                let w = base.saturating_mul(1u64 << shift).min(BACKOFF_CAP_MS);
                u.next_retry = Some(Instant::now() + Duration::from_millis(w));
                w
            })
            .unwrap_or(base);
        let sleep = tokio::time::sleep(Duration::from_millis(wait));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => return,
                _ = self.wake[idx].notified() => {
                    if self.take_cmd(idx) == Cmd::Stop {
                        self.with(idx, |u| u.state = UnitState::Stopped);
                        self.wait_cmd(idx, &[Cmd::Load]).await;
                        return;
                    }
                }
            }
        }
    }

    async fn run_remote(self: Arc<Self>, idx: usize, on_running: Arc<dyn Fn(String, String) + Send + Sync>) {
        loop {
            self.with(idx, |u| u.state = UnitState::Starting);
            let Some(epoch) = self.hello(idx).await else {
                self.with(idx, |u| u.state = UnitState::Down);
                tokio::time::sleep(Duration::from_millis(REMOTE_PROBE_MS)).await;
                continue;
            };
            let id = self.with(idx, |u| {
                u.state = UnitState::Running;
                u.epoch = Some(epoch.clone());
                u.id.clone()
            });
            if let Some(id) = id {
                on_running(id, epoch.clone());
            }
            loop {
                tokio::time::sleep(Duration::from_millis(REMOTE_PROBE_MS)).await;
                let client = self.with(idx, |u| u.client.clone()).flatten();
                let ok = match client {
                    Some(mut c) => matches!(
                        common::rpc::call(&mut c, &UnitReq::Hello, Duration::from_millis(1_000)).await,
                        Ok(UnitRes::Hello(h)) if h.epoch == epoch
                    ),
                    None => false,
                };
                if !ok {
                    self.with(idx, |u| {
                        u.state = UnitState::Down;
                        u.client = None;
                    });
                    break;
                }
            }
        }
    }

    pub async fn shutdown_units(&self) {
        let pids: Vec<(String, Option<u32>)> = self
            .slots
            .lock()
            .map(|s| s.iter().rev().filter(|u| u.kind == UnitKind::Process).map(|u| (u.id.clone(), u.pid)).collect())
            .unwrap_or_default();
        for (id, pid) in pids {
            self.set_state(&id, UnitState::Stopping);
            signal(pid, nix::sys::signal::Signal::SIGTERM);
            let deadline = Instant::now() + Duration::from_millis(STOP_GRACE_MS);
            while Instant::now() < deadline && pid.is_some_and(alive) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if pid.is_some_and(alive) {
                signal(pid, nix::sys::signal::Signal::SIGKILL);
            }
            self.set_state(&id, UnitState::Stopped);
        }
    }
}

fn signal(pid: Option<u32>, sig: nix::sys::signal::Signal) {
    if let Some(p) = pid {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), sig);
    }
}

fn alive(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}
