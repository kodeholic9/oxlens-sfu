// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:IMPL-BUS-FAULTS · 정:IMPL-CFG-SYSTEM-FILE · 정:CAND-BUS-HANDLE-SHAPE · 정:CAND-OPS-SHUTDOWN-NO-RESTART · 운영 규격서 §4-1 · model: claude-opus-5-5
//! supervisor — 유닛 셋 종류(process · remote · inproc). 준비 = 로거는 stdout `ready epoch=` · sfu 는 gRPC Hello · zenoh 는 세션 열림. 부모 생존 채널은 자식 stdin.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::bus::grpc::handle_client::HandleClient;
use common::bus::grpc::HelloReq;
use common::system::{Unit as UnitCfg, UnitKind, UnitRole};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tonic::transport::Channel;

use super::sup_core::{Core, Effect, Event, Limits, UnitState};

pub const STOP_GRACE: Duration = Duration::from_secs(5);
const PROBE_EVERY: Duration = Duration::from_millis(200);
pub const ZENOH_UNIT: &str = "zenoh";

#[derive(Debug, Clone)]
pub enum UnitEvent {
    Ready { id: String, role: UnitRole, epoch: String, client: Option<Box<HandleClient<Channel>>> },
    Down { id: String, role: UnitRole },
}

pub enum Ctl {
    Boot,
    Stop(oneshot::Sender<(String, String)>),
    Load(oneshot::Sender<Result<(String, String), String>>),
    Kill(oneshot::Sender<(String, String)>),
}

pub struct Unit {
    pub cfg: UnitCfg,
    pub core: Mutex<Core>,
    pub epoch: Mutex<Option<String>>,
    pub pid: AtomicU32,
    ctl: mpsc::UnboundedSender<Ctl>,
}

impl Unit {
    pub fn state(&self) -> UnitState {
        self.core.lock().map(|c| c.state).unwrap_or(UnitState::Down)
    }

    pub fn epoch(&self) -> Option<String> {
        self.epoch.lock().ok().and_then(|e| e.clone())
    }

    pub fn status(&self, now_ms: u64) -> serde_json::Value {
        let c = self.core.lock().map(|c| c.clone()).unwrap_or_else(|_| Core::new(true));
        let mut v = serde_json::json!({
            "id": self.cfg.id, "kind": kind_name(self.cfg.kind), "order": self.cfg.order, "role": role_name(self.cfg.role),
            "state": c.state.name(), "restarts": c.restarts,
            "next_retry_ms": c.next_retry_at.map(|t| t.saturating_sub(now_ms)).unwrap_or(0),
        });
        if let Some(e) = self.epoch() {
            v["epoch"] = serde_json::Value::String(e);
        }
        v
    }

    pub fn send(&self, c: Ctl) {
        let _ = self.ctl.send(c);
    }
}

pub fn kind_name(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Process => "process",
        UnitKind::Remote => "remote",
        UnitKind::Inproc => "inproc",
    }
}

pub fn role_name(r: UnitRole) -> &'static str {
    match r {
        UnitRole::Sfu => "sfu",
        UnitRole::Log => "log",
    }
}

pub struct Shared {
    pub node: String,
    pub hub_inst: String,
    pub extra: Vec<String>,
    pub limits: Limits,
    pub hello_timeout: Duration,
    pub shutting: Arc<AtomicBool>,
    pub events: mpsc::UnboundedSender<UnitEvent>,
}

pub struct Supervisor {
    pub units: Vec<Arc<Unit>>,
    pub zenoh_state: Mutex<UnitState>,
}

impl Supervisor {
    pub fn new(cfgs: &[UnitCfg], shared: Shared) -> Supervisor {
        let shared = Arc::new(shared);
        let mut units = Vec::new();
        let mut sorted: Vec<UnitCfg> = cfgs.to_vec();
        sorted.sort_by_key(|u| (u.role != UnitRole::Log, u.order));
        for cfg in sorted {
            let (tx, rx) = mpsc::unbounded_channel();
            let unit = Arc::new(Unit { core: Mutex::new(Core::new(cfg.enabled)), epoch: Mutex::new(None), pid: AtomicU32::new(0), ctl: tx, cfg });
            tokio::spawn(run(unit.clone(), shared.clone(), rx));
            units.push(unit);
        }
        Supervisor { units, zenoh_state: Mutex::new(UnitState::Inactive) }
    }

    pub fn find(&self, id: &str) -> Option<Arc<Unit>> {
        self.units.iter().find(|u| u.cfg.id == id).cloned()
    }

    pub fn of_role(&self, role: UnitRole) -> Vec<Arc<Unit>> {
        self.units.iter().filter(|u| u.cfg.role == role).cloned().collect()
    }

    pub fn boot(&self, role: UnitRole) {
        for u in self.of_role(role) {
            u.send(Ctl::Boot);
        }
    }

    pub async fn wait_running(&self, role: UnitRole, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.of_role(role).iter().all(|u| !matches!(u.state(), UnitState::Inactive | UnitState::Starting)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn set_zenoh(&self, s: UnitState) {
        if let Ok(mut g) = self.zenoh_state.lock() {
            let slot: &mut UnitState = &mut g;
            let _ = std::mem::replace(slot, s);
        }
    }

    pub fn zenoh(&self) -> UnitState {
        self.zenoh_state.lock().map(|g| g.to_owned()).unwrap_or(UnitState::Down)
    }

    pub fn down_units(&self) -> Vec<String> {
        let mut d: Vec<String> = self.units.iter().filter(|u| u.cfg.enabled && u.state() != UnitState::Running).map(|u| u.cfg.id.clone()).collect();
        if self.zenoh() != UnitState::Running {
            d.push(ZENOH_UNIT.to_string());
        }
        d
    }

    pub fn status(&self) -> serde_json::Value {
        let now = common::time::wall_ms();
        let mut units: Vec<serde_json::Value> = self.units.iter().map(|u| u.status(now)).collect();
        units.insert(
            units.iter().position(|u| u["role"] != "log").unwrap_or(units.len()),
            serde_json::json!({"id": ZENOH_UNIT, "kind": "inproc", "order": 0, "role": "zenoh", "state": self.zenoh().name(), "restarts": 0, "next_retry_ms": 0}),
        );
        serde_json::json!({"units": units, "supervising": !self.units.is_empty()})
    }

    pub async fn stop_role(&self, role: UnitRole) {
        let mut waits = Vec::new();
        for u in self.of_role(role).into_iter().rev() {
            let (tx, rx) = oneshot::channel();
            u.send(Ctl::Stop(tx));
            waits.push(rx);
        }
        for w in waits {
            let _ = tokio::time::timeout(STOP_GRACE + Duration::from_secs(1), w).await;
        }
    }
}

fn now_ms() -> u64 {
    common::time::wall_ms()
}

fn transition(unit: &Unit, ev: Event, lim: &Limits) -> (UnitState, Effect, UnitState) {
    let mut c = unit.core.lock().unwrap_or_else(|p| p.into_inner());
    let from = c.state;
    let eff = c.on(ev, now_ms(), lim);
    let to = c.state;
    if from != to {
        tracing::info!(kind = "ev", spec = "RULE-OPS-SUPERVISOR", unit = %unit.cfg.id, from = from.name(), to = to.name(), "unit state");
    }
    (from, eff, to)
}

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
}

async fn spawn(unit: &Arc<Unit>, sh: &Arc<Shared>, generation: u64, ready: &mpsc::UnboundedSender<(u64, String, Option<HandleClient<Channel>>)>) -> Option<Running> {
    match unit.cfg.kind {
        UnitKind::Process => {
            let mut cmd = Command::new(&unit.cfg.cmd[0]);
            cmd.args(&unit.cfg.cmd[1..]).args(&sh.extra).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(kind = "ev", spec = "RULE-OPS-SUPERVISOR", unit = %unit.cfg.id, "exec failed: {e}");
                    return None;
                }
            };
            unit.pid.store(child.id().unwrap_or(0), Ordering::Relaxed);
            let stdin = child.stdin.take();
            if let Some(out) = child.stdout.take() {
                let role = unit.cfg.role;
                let ready = ready.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(out).lines();
                    while let Ok(Some(l)) = lines.next_line().await {
                        if role == UnitRole::Log {
                            if let Some(e) = l.strip_prefix("ready epoch=") {
                                let _ = ready.send((generation, e.trim().to_string(), None));
                            }
                        } else {
                            println!("{l}");
                        }
                    }
                });
            }
            if unit.cfg.role == UnitRole::Sfu {
                probe(unit.clone(), sh.clone(), generation, ready.clone());
            }
            Some(Running { child, stdin })
        }
        UnitKind::Remote | UnitKind::Inproc => {
            probe(unit.clone(), sh.clone(), generation, ready.clone());
            None
        }
    }
}

fn probe(unit: Arc<Unit>, sh: Arc<Shared>, generation: u64, ready: mpsc::UnboundedSender<(u64, String, Option<HandleClient<Channel>>)>) {
    let Some(addr) = unit.cfg.addr.clone() else { return };
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + sh.hello_timeout;
        let url = format!("http://{addr}");
        while tokio::time::Instant::now() < deadline {
            if let Ok(ep) = Channel::from_shared(url.clone())
                && let Ok(ch) = ep.connect_timeout(Duration::from_millis(500)).connect().await
            {
                let mut c = HandleClient::new(ch);
                if let Ok(r) = c.hello(HelloReq { node: sh.node.clone(), hub_inst: sh.hub_inst.clone() }).await {
                    let r = r.into_inner();
                    tracing::info!(kind = "ev", spec = "CAND-BUS-HANDLE-SHAPE", unit = %unit.cfg.id, epoch = %r.epoch, build = %r.build, "Hello ok");
                    let _ = ready.send((generation, r.epoch, Some(c)));
                    return;
                }
            }
            tokio::time::sleep(PROBE_EVERY).await;
        }
        tracing::warn!(kind = "diag", unit = %unit.cfg.id, "Hello timed out");
        let _ = ready.send((generation, String::new(), None));
    });
}

fn signal(pid: u32, sig: nix::sys::signal::Signal) {
    if pid > 0 {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), sig);
    }
}

async fn terminate(r: &mut Option<Running>, unit: &Unit) {
    if let Some(run) = r.as_mut() {
        run.stdin = None;
        signal(unit.pid.load(Ordering::Relaxed), nix::sys::signal::Signal::SIGTERM);
        if tokio::time::timeout(STOP_GRACE, run.child.wait()).await.is_err() {
            let _ = run.child.start_kill();
            let _ = run.child.wait().await;
        }
    }
    let _ = r.take();
}

async fn run(unit: Arc<Unit>, sh: Arc<Shared>, mut rx: mpsc::UnboundedReceiver<Ctl>) {
    let (ready_tx, mut ready_rx) = mpsc::unbounded_channel::<(u64, String, Option<HandleClient<Channel>>)>();
    let mut cur: Option<Running> = None;
    let mut generation: u64 = 0;
    let lim = sh.limits.clone();
    loop {
        let retry_at = unit.core.lock().ok().and_then(|c| c.next_retry_at);
        let has_child = cur.is_some();
        let state = unit.state();
        tokio::select! {
            c = rx.recv() => {
                let Some(c) = c else { break };
                match c {
                    Ctl::Boot => {
                        let (_, eff, _) = transition(&unit, Event::Boot, &lim);
                        if eff == Effect::Spawn {
                            generation += 1;
                            cur = spawn(&unit, &sh, generation, &ready_tx).await;
                            if cur.is_none() && unit.cfg.kind == UnitKind::Process {
                                transition(&unit, Event::Exited, &lim);
                            }
                        }
                    }
                    Ctl::Load(reply) => {
                        let (from, eff, to) = transition(&unit, Event::Load, &lim);
                        match eff {
                            Effect::Refuse(why) => { let _ = reply.send(Err(why.to_string())); }
                            Effect::Spawn => {
                                let _ = reply.send(Ok((from.name().to_string(), to.name().to_string())));
                                generation += 1;
                                cur = spawn(&unit, &sh, generation, &ready_tx).await;
                                if cur.is_none() && unit.cfg.kind == UnitKind::Process {
                                    transition(&unit, Event::Exited, &lim);
                                }
                            }
                            _ => { let _ = reply.send(Ok((from.name().to_string(), to.name().to_string()))); }
                        }
                    }
                    Ctl::Stop(reply) => {
                        let (from, eff, _) = transition(&unit, Event::Stop, &lim);
                        if eff == Effect::Terminate {
                            terminate(&mut cur, &unit).await;
                            transition(&unit, Event::StopDone, &lim);
                            let _ = sh.events.send(UnitEvent::Down { id: unit.cfg.id.clone(), role: unit.cfg.role });
                        }
                        let _ = reply.send((from.name().to_string(), unit.state().name().to_string()));
                    }
                    Ctl::Kill(reply) => {
                        let from = unit.state();
                        if let Some(run) = cur.as_mut() {
                            let _ = run.child.start_kill();
                            let _ = run.child.wait().await;
                        }
                        cur = None;
                        transition(&unit, Event::Exited, &lim);
                        let _ = sh.events.send(UnitEvent::Down { id: unit.cfg.id.clone(), role: unit.cfg.role });
                        let _ = reply.send((from.name().to_string(), unit.state().name().to_string()));
                    }
                }
            }
            st = async { match cur.as_mut() { Some(r) => r.child.wait().await, None => std::future::pending().await } }, if has_child => {
                let _ = st;
                cur = None;
                unit.pid.store(0, Ordering::Relaxed);
                if state == UnitState::Stopping {
                    transition(&unit, Event::StopDone, &lim);
                } else {
                    transition(&unit, Event::Exited, &lim);
                }
                if let Ok(mut e) = unit.epoch.lock() {
                    let _ = e.take();
                }
                let _ = sh.events.send(UnitEvent::Down { id: unit.cfg.id.clone(), role: unit.cfg.role });
            }
            r = ready_rx.recv() => {
                let Some((g, epoch, client)) = r else { continue };
                if g != generation {
                    continue;
                }
                if epoch.is_empty() {
                    if let Some(run) = cur.as_mut() {
                        let _ = run.child.start_kill();
                    } else {
                        transition(&unit, Event::Exited, &lim);
                    }
                    continue;
                }
                let (_, _, to) = transition(&unit, Event::Ready, &lim);
                if to == UnitState::Running {
                    if let Ok(mut e) = unit.epoch.lock() {
                        let _ = e.insert(epoch.clone());
                    }
                    let _ = sh.events.send(UnitEvent::Ready { id: unit.cfg.id.clone(), role: unit.cfg.role, epoch, client: client.map(Box::new) });
                }
            }
            _ = async { match retry_at { Some(t) => tokio::time::sleep(Duration::from_millis(t.saturating_sub(now_ms()))).await, None => std::future::pending().await } } => {
                if sh.shutting.load(Ordering::Relaxed) {
                    if let Ok(mut c) = unit.core.lock() {
                        c.shutting = true;
                        c.next_retry_at = None;
                    }
                    continue;
                }
                let (_, eff, _) = transition(&unit, Event::BackoffDue, &lim);
                if eff == Effect::Spawn {
                    generation += 1;
                    cur = spawn(&unit, &sh, generation, &ready_tx).await;
                    if cur.is_none() && unit.cfg.kind == UnitKind::Process {
                        transition(&unit, Event::Exited, &lim);
                    }
                }
            }
        }
    }
}

pub fn shutting_mark(sup: &Supervisor) {
    for u in &sup.units {
        if let Ok(mut c) = u.core.lock() {
            c.shutting = true;
        }
    }
}
