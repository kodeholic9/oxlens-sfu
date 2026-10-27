// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:IMPL-BUS-FAULTS · 정:CAND-OPS-SUPERVISOR-DEFAULTS · 정:CAND-OPS-SHUTDOWN-NO-RESTART · 정:CAND-OPS-SUPERVISOR-EXTRA-TRANSITIONS · 정:CAND-OPS-ZENOH-UNIT-DOWN · 정:CAND-OPS-UNIT-STATE-LINE · 정:IMPL-CFG-SYSTEM-FILE · 운영:RULE-CTL-SUPERVISOR · model: claude-opus-5-5
//! supervisor — 유닛 종류 셋(process · remote · inproc — zenoh · 로거) · `UnitState` 여덟 · backoff · 폭주 창 · 기동 핸드셰이크 · 순서 속성(로거 → zenoh → sfu · 정지는 역순) · `log_socket` 이 없으면 로거 유닛을 세우지 않는다.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::grpc::{HandleClient, HelloReq};
use common::system::{Unit, UnitKind, UnitRole};
use oxsig::code::Code;
use serde_json::json;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot};
use tonic::transport::Channel;

use super::{Hub, HubRef};
use crate::cfg::HubCfg;

pub const ZENOH_UNIT: &str = "zenoh";
const STOP_GRACE_MS: u64 = 5_000;
const TERM_GRACE_MS: u64 = 2_000;

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

#[derive(Debug, Clone)]
pub struct UnitRt {
    pub cfg: Unit,
    pub state: UnitState,
    pub epoch: Option<String>,
    pub restarts: u32,
    pub next_retry_at_ms: Option<u64>,
    starts: VecDeque<u64>,
    backoff_i: usize,
}

#[derive(Debug)]
pub enum Cmd {
    Stop(oneshot::Sender<Result<(UnitState, UnitState), Code>>),
    Kill(String, oneshot::Sender<Result<(UnitState, UnitState), Code>>),
    Load(oneshot::Sender<Result<(UnitState, UnitState), Code>>),
}

pub struct Supervisor {
    units: Mutex<Vec<UnitRt>>,
    cmds: Mutex<HashMap<String, mpsc::Sender<Cmd>>>,
    clients: tokio::sync::Mutex<HashMap<String, HandleClient<Channel>>>,
    node_declared: AtomicBool,
}

pub fn ordered(units: &[Unit]) -> Vec<Unit> {
    let mut logs: Vec<Unit> = units.iter().filter(|u| u.role == UnitRole::Log).cloned().collect();
    logs.sort_by_key(|u| u.order);
    let mut rest: Vec<Unit> = units.iter().filter(|u| u.role != UnitRole::Log).cloned().collect();
    rest.sort_by_key(|u| u.order);
    let zenoh = Unit { id: ZENOH_UNIT.into(), kind: UnitKind::Inproc, order: 0, role: UnitRole::Sfu, addr: None, cmd: Vec::new(), enabled: true };
    let mut out = logs;
    out.push(zenoh);
    out.extend(rest);
    out
}

pub fn backoff_ms(table: &[u64], i: usize) -> u64 {
    table.get(i).or(table.last()).copied().unwrap_or(1000)
}

pub fn over_intensity(starts: &VecDeque<u64>, now_ms: u64, window_ms: u64, max: u32) -> bool {
    starts.iter().filter(|t| now_ms.saturating_sub(**t) < window_ms).count() > max as usize
}

impl Supervisor {
    pub fn new(cfg: &HubCfg) -> Supervisor {
        let units = ordered(&cfg.system.units)
            .into_iter()
            .filter(|u| u.role != UnitRole::Log || cfg.log_socket.is_some())
            .map(|u| {
                let state = if u.enabled { UnitState::Inactive } else { UnitState::Disabled };
                UnitRt { cfg: u, state, epoch: None, restarts: 0, next_retry_at_ms: None, starts: VecDeque::new(), backoff_i: 0 }
            })
            .collect();
        Supervisor { units: Mutex::new(units), cmds: Mutex::new(HashMap::new()), clients: tokio::sync::Mutex::new(HashMap::new()), node_declared: AtomicBool::new(false) }
    }

    pub fn snapshot(&self) -> Vec<UnitRt> {
        self.units.lock().map(|u| u.clone()).unwrap_or_default()
    }

    pub fn unit_by_epoch(&self, epoch: &str) -> Option<String> {
        self.units.lock().ok().and_then(|u| u.iter().find(|x| x.epoch.as_deref() == Some(epoch) && x.state == UnitState::Running).map(|x| x.cfg.id.clone()))
    }

    pub fn first_running_sfu(&self) -> Option<(String, String)> {
        self.units.lock().ok().and_then(|u| {
            u.iter().find(|x| x.cfg.role == UnitRole::Sfu && x.cfg.id != ZENOH_UNIT && x.state == UnitState::Running).and_then(|x| x.epoch.clone().map(|e| (x.cfg.id.clone(), e)))
        })
    }

    pub fn node_declared(&self) -> bool {
        self.node_declared.load(Ordering::Relaxed)
    }

    pub fn ready(&self) -> (bool, Vec<String>) {
        let u = self.snapshot();
        let down: Vec<String> = u.iter().filter(|x| x.cfg.enabled && x.state != UnitState::Running).map(|x| x.cfg.id.clone()).collect();
        (down.is_empty(), down)
    }

    pub fn supervising(&self) -> bool {
        self.snapshot().iter().any(|u| u.cfg.enabled)
    }

    fn set(&self, id: &str, to: UnitState, cause: &str) -> Option<UnitState> {
        let mut u = self.units.lock().ok()?;
        let x = u.iter_mut().find(|x| x.cfg.id == id)?;
        let from = x.state;
        x.state = to;
        if to != UnitState::Running {
            x.epoch = if to == UnitState::Starting || to == UnitState::Stopping { x.epoch.clone() } else { None };
        }
        if to != UnitState::Down {
            x.next_retry_at_ms = None;
        }
        drop(u);
        common::log::unit_transition(id, from.as_str(), to.as_str(), cause);
        Some(from)
    }

    fn with<R>(&self, id: &str, f: impl FnOnce(&mut UnitRt) -> R) -> Option<R> {
        let mut u = self.units.lock().ok()?;
        u.iter_mut().find(|x| x.cfg.id == id).map(f)
    }

    pub async fn client_for(&self, inst: Option<&str>) -> Option<HandleClient<Channel>> {
        let id = match inst {
            Some(e) => self.unit_by_epoch(e)?,
            None => self.first_running_sfu()?.0,
        };
        self.clients.lock().await.get(&id).cloned()
    }

    pub async fn clients_running(&self) -> Vec<(String, String, HandleClient<Channel>)> {
        let units = self.snapshot();
        let c = self.clients.lock().await;
        units
            .iter()
            .filter(|u| u.state == UnitState::Running && u.cfg.kind != UnitKind::Inproc && u.cfg.role == UnitRole::Sfu)
            .filter_map(|u| c.get(&u.cfg.id).map(|cl| (u.cfg.id.clone(), u.epoch.clone().unwrap_or_default(), cl.clone())))
            .collect()
    }

    pub async fn command(&self, id: &str, make: impl FnOnce(oneshot::Sender<Result<(UnitState, UnitState), Code>>) -> Cmd) -> Result<(UnitState, UnitState), Code> {
        let tx = self.cmds.lock().ok().and_then(|c| c.get(id).cloned()).ok_or(Code::ROOM_NOT_FOUND)?;
        let (rtx, rrx) = oneshot::channel();
        tx.send(make(rtx)).await.map_err(|_| Code::INTERNAL_ERROR)?;
        rrx.await.map_err(|_| Code::INTERNAL_ERROR)?
    }
}

fn extra_args(cfg: &HubCfg, u: &Unit) -> Vec<String> {
    let mut a = vec!["--id".to_string(), cfg.node_id.clone()];
    if let Some(l) = &cfg.build_label {
        a.extend(["--build".to_string(), l.clone()]);
    }
    a.extend(["--zenoh-namespace".to_string(), cfg.system.zenoh.namespace.clone()]);
    if let Some(c) = cfg.system.zenoh.listen.first() {
        a.extend(["--zenoh-connect".to_string(), c.clone()]);
    }
    if let Some(d) = &cfg.log_dir {
        a.extend(["--log-dir".to_string(), d.clone()]);
    }
    if let Some(s) = &cfg.log_socket {
        a.extend(["--log-socket".to_string(), s.clone()]);
    }
    if u.role == UnitRole::Sfu {
        a.extend(["--node-grace-secs".to_string(), cfg.system.supervisor.node_grace_secs.to_string()]);
    }
    a
}

struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
}

pub fn start_all(hub: &HubRef) {
    let units = hub.sup.snapshot();
    let me = hub.clone();
    tokio::spawn(async move {
        for u in units {
            if !u.cfg.enabled {
                continue;
            }
            let (tx, rx) = mpsc::channel(8);
            if let Ok(mut c) = me.sup.cmds.lock() {
                c.insert(u.cfg.id.clone(), tx);
            }
            let (ready_tx, ready_rx) = oneshot::channel();
            let h = me.clone();
            let id = u.cfg.id.clone();
            tokio::spawn(async move { unit_task(h, id, rx, Some(ready_tx)).await });
            let _ = tokio::time::timeout(Duration::from_millis(me.cfg.system.supervisor.hello_timeout_ms + 2_000), ready_rx).await;
        }
    });
}

async fn wait_port_free(addr: &str) {
    loop {
        match std::net::TcpListener::bind(addr) {
            Ok(_) => return,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(_) => return,
        }
    }
}

async fn hello(hub: &Hub, addr: &str, deadline_ms: u64) -> Option<(HandleClient<Channel>, String)> {
    let url = if addr.contains("://") { addr.to_string() } else { format!("http://{addr}") };
    let until = super::now_ms() + deadline_ms;
    loop {
        if let Ok(ep) = tonic::transport::Endpoint::from_shared(url.clone())
            && let Ok(ch) = ep.connect_timeout(Duration::from_millis(500)).connect().await
        {
            let mut c = HandleClient::new(ch);
            if let Ok(r) = c.hello(HelloReq { node: hub.me().to_string(), hub_inst: hub.epoch.clone() }).await {
                return Some((c, r.into_inner().epoch));
            }
        }
        if super::now_ms() >= until {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn start_process(hub: &HubRef, u: &Unit) -> Result<(Proc, String, Option<HandleClient<Channel>>), String> {
    if u.role == UnitRole::Sfu
        && let Some(a) = &u.addr
    {
        wait_port_free(a).await;
    }
    let (prog, rest) = u.cmd.split_first().ok_or("cmd 가 비었다")?;
    let mut cmd = tokio::process::Command::new(prog);
    cmd.args(rest).args(extra_args(&hub.cfg, u)).env(common::proc::LIFELINE_ENV, "stdin").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::inherit()).stderr(std::process::Stdio::inherit());
    let mut child = cmd.spawn().map_err(|e| format!("exec {prog}: {e}"))?;
    let stdin = child.stdin.take();
    let timeout = hub.cfg.system.supervisor.hello_timeout_ms;
    let Some(addr) = &u.addr else { return Ok((Proc { child, stdin }, common::build::new_epoch(), None)) };
    match hello(hub, addr, timeout).await {
        Some((c, epoch)) => Ok((Proc { child, stdin }, epoch, Some(c))),
        None => {
            let _ = child.start_kill();
            Err("기동 핸드셰이크 시한".into())
        }
    }
}

fn signal(child: &Child, sig: nix::sys::signal::Signal) {
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), sig);
    }
}

async fn graceful_stop(mut p: Proc) {
    p.stdin.take();
    if tokio::time::timeout(Duration::from_millis(STOP_GRACE_MS), p.child.wait()).await.is_ok() {
        return;
    }
    signal(&p.child, nix::sys::signal::Signal::SIGTERM);
    if tokio::time::timeout(Duration::from_millis(TERM_GRACE_MS), p.child.wait()).await.is_ok() {
        return;
    }
    let _ = p.child.kill().await;
}

enum Started {
    Proc(Box<Proc>),
    Inproc,
    Logger(super::logger::Running),
    Remote,
}

fn logger_cfg(cfg: &HubCfg) -> Option<super::logger::Config> {
    let sock = cfg.log_socket.as_deref()?;
    let dir = cfg.log_dir.clone().map(std::path::PathBuf::from).or_else(|| std::path::Path::new(sock).parent().map(std::path::Path::to_path_buf)).unwrap_or_else(std::env::temp_dir);
    let l = &cfg.policy.logging;
    Some(super::logger::Config { dir, socket: sock.into(), rotate_max_bytes: l.rotate_max_bytes, retention_days: l.retention_days, rcvbuf: l.socket_rcvbuf_bytes, queue_max: l.queue_max_lines as usize })
}

async fn start_unit(hub: &HubRef, u: &Unit) -> Result<(Started, String, Option<HandleClient<Channel>>), String> {
    match u.kind {
        UnitKind::Inproc if u.role == UnitRole::Log => {
            let lc = logger_cfg(&hub.cfg).ok_or("로거 유닛인데 log_socket 이 없다")?;
            let id = u.id.clone();
            let r = tokio::task::spawn_blocking(move || super::logger::start(lc, &id)).await.map_err(|e| format!("로거 기동: {e}"))??;
            if let Some(g) = hub.log_gate.get() {
                g.open(r.tx.clone());
            }
            Ok((Started::Logger(r), hub.epoch.clone(), None))
        }
        UnitKind::Inproc => {
            let bus = crate::bus::Bus::open(&hub.cfg.system.zenoh, hub.me(), &hub.epoch).await?;
            let _ = hub.bus.set(Arc::new(bus));
            super::query::start(hub).await;
            Ok((Started::Inproc, hub.epoch.clone(), None))
        }
        UnitKind::Remote => {
            let addr = u.addr.clone().ok_or("remote 유닛에 addr 가 없다")?;
            let (c, e) = hello(hub, &addr, hub.cfg.system.supervisor.hello_timeout_ms).await.ok_or("붙지 못했다")?;
            Ok((Started::Remote, e, Some(c)))
        }
        UnitKind::Process => {
            let (p, e, c) = start_process(hub, u).await?;
            Ok((Started::Proc(Box::new(p)), e, c))
        }
    }
}

async fn unit_task(hub: HubRef, id: String, mut rx: mpsc::Receiver<Cmd>, mut ready: Option<oneshot::Sender<()>>) {
    let Some(u) = hub.sup.snapshot().into_iter().find(|x| x.cfg.id == id) else { return };
    let cfg = u.cfg.clone();
    let sv = hub.cfg.system.supervisor.clone();
    let mut cause = "boot";
    let mut from_stopped = false;
    loop {
        if hub.shutting.load(Ordering::Relaxed) {
            return;
        }
        hub.sup.set(&id, UnitState::Starting, cause);
        if !from_stopped {
            let now = super::now_ms();
            hub.sup.with(&id, |x| x.starts.push_back(now));
        }
        from_stopped = false;
        let started = start_unit(&hub, &cfg).await;
        let mut proc_ = None;
        let down_cause: &str;
        match started {
            Ok((s, epoch, client)) => {
                if let Some(c) = client {
                    hub.sup.clients.lock().await.insert(id.clone(), c);
                }
                hub.sup.with(&id, |x| {
                    x.epoch = Some(epoch.clone());
                    x.backoff_i = 0;
                });
                hub.sup.set(&id, UnitState::Running, "ready");
                hub.sup.with(&id, |x| x.epoch = Some(epoch.clone()));
                if let Some(r) = ready.take() {
                    let _ = r.send(());
                }
                if cfg.role == UnitRole::Sfu && cfg.kind != UnitKind::Inproc && !hub.sup.node_declared.swap(true, Ordering::Relaxed) {
                    declare_node(&hub).await;
                }
                let mut logger = None;
                match s {
                    Started::Proc(p) => proc_ = Some(*p),
                    Started::Logger(l) => logger = Some(l),
                    Started::Inproc | Started::Remote => {}
                }
                let r = run(&hub, &id, &cfg, &mut rx, proc_.take(), logger).await;
                match r {
                    Exit::Stopped => {
                        if !park(&hub, &id, &mut rx, UnitState::Stopped).await {
                            return;
                        }
                        cause = "load";
                        from_stopped = true;
                        continue;
                    }
                    Exit::Down(c) => down_cause = c,
                    Exit::Shutdown => return,
                }
            }
            Err(e) => {
                tracing::warn!(unit = %id, error = %e, "유닛 기동 실패");
                down_cause = "start_fail";
            }
        }
        hub.sup.clients.lock().await.remove(&id);
        if hub.shutting.load(Ordering::Relaxed) {
            return;
        }
        if cfg.id == ZENOH_UNIT && down_cause != "start_fail" {
            hub.sup.set(&id, UnitState::Down, down_cause);
            super::fault_shutdown(&hub, "zenoh unit down").await;
            return;
        }
        hub.sup.set(&id, UnitState::Down, down_cause);
        let now = super::now_ms();
        let blocked = hub.sup.with(&id, |x| over_intensity(&x.starts, now, sv.intensity_window_secs * 1000, sv.intensity_max)).unwrap_or(false);
        if blocked {
            hub.sup.set(&id, UnitState::Blocked, "intensity");
            if cfg.id == ZENOH_UNIT {
                super::fault_shutdown(&hub, "zenoh unit blocked").await;
                return;
            }
            if !park(&hub, &id, &mut rx, UnitState::Blocked).await {
                return;
            }
            cause = "load";
            from_stopped = true;
            continue;
        }
        let wait = hub.sup.with(&id, |x| {
            let w = backoff_ms(&sv.backoff_ms, x.backoff_i);
            x.backoff_i += 1;
            x.restarts += 1;
            x.next_retry_at_ms = Some(super::now_ms() + w);
            w
        });
        let wait = wait.unwrap_or(1000);
        let sleep = tokio::time::sleep(Duration::from_millis(wait));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => { cause = "backoff"; break; }
                c = rx.recv() => match c {
                    Some(Cmd::Stop(r)) => {
                        let _ = r.send(Ok((UnitState::Down, UnitState::Stopped)));
                        if !park(&hub, &id, &mut rx, UnitState::Stopped).await { return; }
                        cause = "load";
                        from_stopped = true;
                        break;
                    }
                    Some(Cmd::Load(r)) => { let _ = r.send(Err(Code::PRECONDITION_FAILED)); }
                    Some(Cmd::Kill(_, r)) => { let _ = r.send(Err(Code::PRECONDITION_FAILED)); }
                    None => return,
                }
            }
        }
    }
}

enum Exit {
    Stopped,
    Down(&'static str),
    Shutdown,
}

async fn stop_logger(hub: &HubRef, l: &mut super::logger::Running) {
    if let Some(g) = hub.log_gate.get() {
        g.close();
    }
    l.stop();
    let _ = tokio::time::timeout(Duration::from_millis(STOP_GRACE_MS), &mut l.done).await;
}

async fn run(hub: &HubRef, id: &str, cfg: &Unit, rx: &mut mpsc::Receiver<Cmd>, proc_: Option<Proc>, logger: Option<super::logger::Running>) -> Exit {
    let mut p = proc_;
    let mut lg = logger;
    loop {
        let exited = async {
            match (p.as_mut(), lg.as_mut()) {
                (Some(pp), _) => {
                    let _ = pp.child.wait().await;
                }
                (None, Some(l)) => {
                    let _ = (&mut l.done).await;
                }
                (None, None) => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = exited => {
                if let Some(g) = hub.log_gate.get().filter(|_| lg.is_some()) {
                    g.close();
                }
                return if hub.shutting.load(Ordering::Relaxed) { Exit::Shutdown } else { Exit::Down("exit") };
            }
            c = rx.recv() => match c {
                None => return Exit::Shutdown,
                Some(Cmd::Stop(r)) => {
                    if let Some(l) = lg.as_mut() {
                        hub.sup.set(id, UnitState::Stopping, "stop");
                        stop_logger(hub, l).await;
                        hub.sup.set(id, UnitState::Stopped, "stop");
                        let _ = r.send(Ok((UnitState::Running, UnitState::Stopped)));
                        return Exit::Stopped;
                    }
                    if cfg.kind == UnitKind::Inproc {
                        let _ = r.send(Ok((UnitState::Running, UnitState::Stopped)));
                        super::begin_shutdown(hub, "zenoh unit stop").await;
                        return Exit::Shutdown;
                    }
                    hub.sup.set(id, UnitState::Stopping, "stop");
                    if let Some(pp) = p.take() {
                        graceful_stop(pp).await;
                    }
                    hub.sup.clients.lock().await.remove(id);
                    hub.sup.set(id, UnitState::Stopped, "stop");
                    let _ = r.send(Ok((UnitState::Running, UnitState::Stopped)));
                    return Exit::Stopped;
                }
                Some(Cmd::Kill(epoch, r)) => {
                    let cur = hub.sup.with(id, |x| x.epoch.clone()).flatten();
                    if cur.as_deref() != Some(epoch.as_str()) {
                        let _ = r.send(Err(Code::PRECONDITION_FAILED));
                        continue;
                    }
                    if let Some(l) = lg.as_mut() {
                        stop_logger(hub, l).await;
                        let _ = r.send(Ok((UnitState::Running, UnitState::Down)));
                        return Exit::Down("kill");
                    }
                    if cfg.kind == UnitKind::Inproc {
                        let _ = r.send(Ok((UnitState::Running, UnitState::Down)));
                        super::begin_shutdown(hub, "zenoh unit kill").await;
                        return Exit::Shutdown;
                    }
                    if let Some(pp) = p.as_mut() {
                        let _ = pp.child.start_kill();
                        let _ = pp.child.wait().await;
                    }
                    let _ = r.send(Ok((UnitState::Running, UnitState::Down)));
                    return Exit::Down("kill");
                }
                Some(Cmd::Load(r)) => { let _ = r.send(Err(Code::PRECONDITION_FAILED)); }
            }
        }
    }
}

async fn park(hub: &HubRef, id: &str, rx: &mut mpsc::Receiver<Cmd>, state: UnitState) -> bool {
    loop {
        match rx.recv().await {
            None => return false,
            Some(Cmd::Load(r)) => {
                let _ = r.send(Ok((state, UnitState::Starting)));
                hub.sup.with(id, |x| {
                    x.starts.clear();
                    x.backoff_i = 0;
                });
                return true;
            }
            Some(Cmd::Stop(r)) => {
                if state == UnitState::Blocked {
                    hub.sup.set(id, UnitState::Stopped, "stop");
                    let _ = r.send(Ok((UnitState::Blocked, UnitState::Stopped)));
                    return Box::pin(park(hub, id, rx, UnitState::Stopped)).await;
                }
                let _ = r.send(Ok((state, UnitState::Stopped)));
            }
            Some(Cmd::Kill(_, r)) => {
                let _ = r.send(Err(Code::PRECONDITION_FAILED));
            }
        }
    }
}

async fn declare_node(hub: &HubRef) {
    let Some(bus) = hub.bus() else { return };
    match bus.declare_node().await {
        Ok(t) => {
            hub.node_token.lock().await.replace(t);
            tracing::info!("node 토큰 선언");
        }
        Err(e) => tracing::error!(error = %e, "node 토큰 선언 실패"),
    }
}

pub async fn stop_all(hub: &HubRef, loggers: bool) {
    let units = hub.sup.snapshot();
    for u in units.iter().rev() {
        if u.state != UnitState::Running || u.cfg.id == ZENOH_UNIT || (u.cfg.role == UnitRole::Log) != loggers {
            continue;
        }
        let tx = hub.sup.cmds.lock().ok().and_then(|c| c.get(&u.cfg.id).cloned());
        if let Some(tx) = tx {
            let (r, rr) = oneshot::channel();
            if tx.send(Cmd::Stop(r)).await.is_ok() {
                let _ = tokio::time::timeout(Duration::from_millis(STOP_GRACE_MS + TERM_GRACE_MS + 1_000), rr).await;
            }
        }
    }
}

pub fn status_json(hub: &Hub) -> serde_json::Value {
    let now = super::now_ms();
    let units: Vec<serde_json::Value> = hub
        .sup
        .snapshot()
        .iter()
        .map(|u| {
            let mut o = json!({"id": u.cfg.id, "kind": u.cfg.kind.as_str(), "order": u.cfg.order.max(0), "state": u.state.as_str(), "restarts": u.restarts});
            if u.state == UnitState::Down
                && let Some(t) = u.next_retry_at_ms
            {
                o["next_retry_ms"] = json!(t.saturating_sub(now));
            }
            if u.state == UnitState::Running
                && u.cfg.kind != UnitKind::Inproc
                && let Some(e) = &u.epoch
            {
                o["epoch"] = json!(e);
            }
            o
        })
        .collect();
    json!({"units": units, "supervising": hub.sup.supervising()})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: &str, role: UnitRole, order: i32) -> Unit {
        Unit { id: id.into(), kind: UnitKind::Process, order, role, addr: None, cmd: vec!["x".into()], enabled: true }
    }

    #[test]
    fn logger_first_then_zenoh_then_rest() {
        let o = ordered(&[unit("sfu-b", UnitRole::Sfu, 2), unit("log", UnitRole::Log, 5), unit("sfu-a", UnitRole::Sfu, 1)]);
        let ids: Vec<&str> = o.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(ids, vec!["log", ZENOH_UNIT, "sfu-a", "sfu-b"]);
    }

    #[test]
    fn backoff_steps_then_holds_last() {
        let t = [200, 500, 1000];
        assert_eq!(backoff_ms(&t, 0), 200);
        assert_eq!(backoff_ms(&t, 2), 1000);
        assert_eq!(backoff_ms(&t, 9), 1000);
    }

    #[test]
    fn intensity_counts_starts_in_window() {
        let s: VecDeque<u64> = [0, 1_000, 2_000, 3_000, 4_000, 5_000].into();
        assert!(over_intensity(&s, 6_000, 60_000, 5));
        assert!(!over_intensity(&s, 63_500, 60_000, 5));
    }
}
