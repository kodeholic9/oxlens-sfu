// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:IMPL-BUS-FAULTS · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! supervisor — 유닛 상태 여덟 · 종류 셋(process · remote · inproc) · 순서(로거 → zenoh → sfu · 정지는 역순) · 준비 신호(로거 stdout 한 줄 · sfu Hello) · backoff · 폭주 창.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::system::{SystemFile, Unit, UnitKind, UnitRole};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::Notify;

use crate::bus::Bus;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Start,
    Ready,
    Exited,
    Stop,
    Load,
    BackoffDue,
    Blocked,
}

pub fn step(s: UState, e: &Event) -> Option<UState> {
    use UState::*;
    match (s, e) {
        (Disabled, _) => None,
        (Inactive, Event::Start) => Some(Starting),
        (Starting, Event::Ready) => Some(Running),
        (Starting, Event::Exited) => Some(Down),
        (Running, Event::Exited) => Some(Down),
        (Running, Event::Stop) | (Starting, Event::Stop) => Some(Stopping),
        (Stopping, Event::Exited) => Some(Stopped),
        (Stopped, Event::Load) => Some(Starting),
        (Down, Event::BackoffDue) => Some(Starting),
        (Down, Event::Blocked) => Some(Blocked),
        (Down, Event::Stop) => Some(Stopped),
        (Blocked, Event::Load) => Some(Starting),
        _ => None,
    }
}

pub fn intensity_exceeded(starts: &VecDeque<u64>, now_ms: u64, window_ms: u64, max: u32) -> bool {
    starts.iter().filter(|t| now_ms.saturating_sub(**t) < window_ms).count() as u32 > max
}

pub struct Slot {
    pub conf: Unit,
    pub state: UState,
    pub restarts: u32,
    pub next_retry_ms: Option<u64>,
    pub epoch: Option<String>,
    pub build: Option<String>,
    pub starts: VecDeque<u64>,
    pub since: u64,
    pid: Option<u32>,
    stdin: Option<ChildStdin>,
    run: u64,
}

pub struct Supervisor {
    pub sys: Arc<SystemFile>,
    pub bus: Arc<Bus>,
    pub extra: Vec<String>,
    pub slots: Mutex<Vec<Slot>>,
    changed: Notify,
    closing: std::sync::atomic::AtomicBool,
}

fn now() -> u64 {
    common::clock::wall_ms()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn rank(u: &Unit) -> (u32, u32) {
    let r = match u.role {
        UnitRole::Log => 0,
        UnitRole::Bus => 1,
        UnitRole::Sfu => 2,
        UnitRole::Other => 3,
    };
    (r, u.order)
}

impl Supervisor {
    pub fn new(sys: Arc<SystemFile>, bus: Arc<Bus>, extra: Vec<String>) -> Arc<Supervisor> {
        let mut units: Vec<Unit> = sys.unit.clone();
        units.sort_by_key(rank);
        let slots = units
            .into_iter()
            .map(|u| Slot {
                state: if u.enabled { UState::Inactive } else { UState::Disabled },
                conf: u,
                restarts: 0,
                next_retry_ms: None,
                epoch: None,
                build: None,
                starts: VecDeque::new(),
                since: now(),
                pid: None,
                stdin: None,
                run: 0,
            })
            .collect();
        Arc::new(Supervisor { sys, bus, extra, slots: Mutex::new(slots), changed: Notify::new(), closing: std::sync::atomic::AtomicBool::new(false) })
    }

    pub fn ids(&self, role: Option<UnitRole>) -> Vec<String> {
        lock(&self.slots).iter().filter(|s| role.map(|r| s.conf.role == r).unwrap_or(true)).map(|s| s.conf.id.clone()).collect()
    }

    pub fn state_of(&self, id: &str) -> Option<UState> {
        lock(&self.slots).iter().find(|s| s.conf.id == id).map(|s| s.state)
    }

    pub fn all_running(&self) -> (bool, Vec<String>) {
        let g = lock(&self.slots);
        let down: Vec<String> = g.iter().filter(|s| s.state != UState::Disabled && s.state != UState::Running).map(|s| s.conf.id.clone()).collect();
        (down.is_empty(), down)
    }

    pub fn status(&self) -> Value {
        let g = lock(&self.slots);
        let mut units: Vec<Value> = vec![json!({"id": "zenoh", "kind": "inproc", "order": 0, "state": "Running", "restarts": 0, "next_retry_ms": null, "epoch": self.bus.session.zid().to_string()})];
        for s in g.iter() {
            units.push(json!({
                "id": s.conf.id,
                "kind": match s.conf.kind { UnitKind::Process => "process", UnitKind::Remote => "remote", UnitKind::Inproc => "inproc" },
                "role": format!("{:?}", s.conf.role).to_lowercase(),
                "order": s.conf.order,
                "state": s.state,
                "restarts": s.restarts,
                "next_retry_ms": s.next_retry_ms.map(|t| t.saturating_sub(now())),
                "epoch": s.epoch,
                "addr": s.conf.addr,
            }));
        }
        json!({"units": units, "supervising": !g.is_empty()})
    }

    fn set(&self, id: &str, e: Event) -> Option<(UState, UState)> {
        let mut g = lock(&self.slots);
        let s = g.iter_mut().find(|s| s.conf.id == id)?;
        let to = step(s.state, &e)?;
        let from = s.state;
        s.state = to;
        s.since = now();
        drop(g);
        tracing::info!(kind = "diag", unit = %id, from = ?from, to = ?to, cause = ?e, "unit state");
        self.changed.notify_waiters();
        Some((from, to))
    }

    pub async fn start_all(self: &Arc<Self>) {
        let ids = self.ids(None);
        for id in ids {
            if self.state_of(&id) == Some(UState::Disabled) {
                continue;
            }
            self.clone().launch(&id).await;
            let deadline = now() + self.sys.supervisor.hello_timeout_ms;
            while now() < deadline && self.state_of(&id) == Some(UState::Starting) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    fn launch(self: Arc<Self>, id: &str) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let id = id.to_string();
        Box::pin(async move { self.launch_inner(&id).await })
    }

    async fn launch_inner(self: Arc<Self>, id: &str) {
        let conf = match lock(&self.slots).iter().find(|s| s.conf.id == id) {
            Some(s) => s.conf.clone(),
            None => return,
        };
        {
            let mut g = lock(&self.slots);
            if let Some(s) = g.iter_mut().find(|s| s.conf.id == id) {
                s.starts.push_back(now());
                while s.starts.len() > 64 {
                    s.starts.pop_front();
                }
            }
        }
        if self.state_of(id).map(|s| s != UState::Starting).unwrap_or(true) {
            let e = match self.state_of(id) {
                Some(UState::Inactive) => Event::Start,
                Some(UState::Stopped) | Some(UState::Blocked) => Event::Load,
                _ => Event::BackoffDue,
            };
            self.set(id, e);
        }
        match conf.kind {
            UnitKind::Process => self.spawn_process(conf).await,
            UnitKind::Remote => self.attach_remote(conf).await,
            UnitKind::Inproc => {
                self.set(id, Event::Ready);
            }
        }
    }

    async fn spawn_process(self: Arc<Self>, conf: Unit) {
        let id = conf.id.clone();
        let Some((prog, args)) = conf.cmd.split_first() else {
            self.exited(&id, 0).await;
            return;
        };
        let mut cmd = Command::new(prog);
        cmd.args(args).args(&self.extra).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(false);
        let child = cmd.spawn();
        let mut child: Child = match child {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(kind = "diag", unit = %id, err = %e, "exec failed — counted as a start");
                self.exited(&id, 0).await;
                return;
            }
        };
        let run = {
            let mut g = lock(&self.slots);
            let Some(s) = g.iter_mut().find(|s| s.conf.id == id) else { return };
            s.run += 1;
            s.pid = child.id();
            s.stdin = child.stdin.take();
            s.epoch = None;
            s.run
        };
        if let Some(err) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    eprintln!("{l}");
                }
            });
        }
        let is_log = conf.role == UnitRole::Log;
        if let Some(out) = child.stdout.take() {
            let me = self.clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if is_log
                        && let Some(ep) = l.strip_prefix("ready epoch=") {
                            me.ready(&id2, run, ep.trim().to_string(), None).await;
                            continue;
                        }
                    println!("{l}");
                }
            });
        }
        if conf.role == UnitRole::Sfu {
            let me = self.clone();
            let addr = conf.addr.clone().unwrap_or_default();
            let id2 = id.clone();
            let timeout = self.sys.supervisor.hello_timeout_ms;
            tokio::spawn(async move {
                let deadline = now() + timeout;
                while now() < deadline {
                    if me.run_of(&id2) != Some(run) {
                        return;
                    }
                    match me.bus.set_local(&addr).await {
                        Ok((epoch, build)) => {
                            me.ready(&id2, run, epoch, Some(build)).await;
                            return;
                        }
                        Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                    }
                }
                tracing::warn!(kind = "diag", unit = %id2, "hello timeout — killing");
                me.kill_pid(&id2);
            });
        }
        let me = self.clone();
        tokio::spawn(async move {
            let _ = child.wait().await;
            me.exited(&id, run).await;
        });
    }

    async fn attach_remote(self: Arc<Self>, conf: Unit) {
        let addr = conf.addr.clone().unwrap_or_default();
        let run = {
            let mut g = lock(&self.slots);
            let Some(s) = g.iter_mut().find(|s| s.conf.id == conf.id) else { return };
            s.run += 1;
            s.run
        };
        let deadline = now() + self.sys.supervisor.hello_timeout_ms;
        while now() < deadline {
            if let Ok((epoch, build)) = self.bus.set_local(&addr).await {
                self.ready(&conf.id, run, epoch, Some(build)).await;
                let me = self.clone();
                let id = conf.id.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(1000)).await;
                        if me.run_of(&id) != Some(run) {
                            return;
                        }
                        if me.bus.local_hello().await.is_none() {
                            me.exited(&id, run).await;
                            return;
                        }
                    }
                });
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        self.exited(&conf.id, run).await;
    }

    fn run_of(&self, id: &str) -> Option<u64> {
        lock(&self.slots).iter().find(|s| s.conf.id == id).map(|s| s.run)
    }

    async fn ready(&self, id: &str, run: u64, epoch: String, build: Option<String>) {
        let role = {
            let mut g = lock(&self.slots);
            let Some(s) = g.iter_mut().find(|s| s.conf.id == id) else { return };
            if s.run != run {
                return;
            }
            s.epoch = Some(epoch.clone());
            s.build = build;
            s.conf.role
        };
        if self.set(id, Event::Ready).is_none() {
            return;
        }
        if role == UnitRole::Sfu
            && let Err(e) = self.bus.declare_node(&epoch).await {
                tracing::warn!(kind = "diag", unit = %id, err = %e, "node token declare failed");
            }
    }

    async fn exited(self: &Arc<Self>, id: &str, run: u64) {
        let (role, state) = {
            let mut g = lock(&self.slots);
            let Some(s) = g.iter_mut().find(|s| s.conf.id == id) else { return };
            if run != 0 && s.run != run {
                return;
            }
            s.pid = None;
            s.stdin = None;
            (s.conf.role, s.state)
        };
        if role == UnitRole::Sfu {
            self.bus.clear_local().await;
            self.bus.undeclare_node();
        }
        let moved = self.set(id, Event::Exited);
        if state == UState::Stopping || moved.map(|(_, to)| to == UState::Stopped).unwrap_or(false) || self.closing.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let (exceeded, wait) = {
            let mut g = lock(&self.slots);
            let Some(s) = g.iter_mut().find(|s| s.conf.id == id) else { return };
            let sup = &self.sys.supervisor;
            let exceeded = intensity_exceeded(&s.starts, now(), sup.intensity_window_secs * 1000, sup.intensity_max);
            let idx = (s.restarts as usize).min(sup.backoff_ms.len().saturating_sub(1));
            let wait = sup.backoff_ms.get(idx).copied().unwrap_or(1000);
            s.restarts += 1;
            s.next_retry_ms = if exceeded { None } else { Some(now() + wait) };
            (exceeded, wait)
        };
        if exceeded {
            self.set(id, Event::Blocked);
            tracing::error!(kind = "diag", unit = %id, "backoff storm — blocked");
            return;
        }
        let me = self.clone();
        let id2 = id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            if me.state_of(&id2) == Some(UState::Down) {
                me.clone().launch(&id2).await;
            }
        });
    }

    fn kill_pid(&self, id: &str) {
        let pid = lock(&self.slots).iter().find(|s| s.conf.id == id).and_then(|s| s.pid);
        if let Some(p) = pid {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), nix::sys::signal::Signal::SIGKILL);
        }
    }

    pub async fn kill(self: &Arc<Self>, id: &str) -> Option<(UState, UState)> {
        let from = self.state_of(id)?;
        self.kill_pid(id);
        Some((from, UState::Down))
    }

    pub async fn stop(self: &Arc<Self>, id: &str) -> Option<(UState, UState)> {
        let from = self.state_of(id)?;
        if from == UState::Down || from == UState::Blocked {
            let mut g = lock(&self.slots);
            if let Some(s) = g.iter_mut().find(|s| s.conf.id == id) {
                s.state = UState::Stopped;
            }
            return Some((from, UState::Stopped));
        }
        self.set(id, Event::Stop)?;
        let (pid, stdin) = {
            let mut g = lock(&self.slots);
            let s = g.iter_mut().find(|s| s.conf.id == id)?;
            (s.pid, s.stdin.take())
        };
        if let Some(mut si) = stdin {
            let _ = si.shutdown().await;
        }
        if let Some(p) = pid {
            let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(p as i32), nix::sys::signal::Signal::SIGTERM);
        } else if self.state_of(id) == Some(UState::Stopping) {
            self.exited(id, 0).await;
        }
        let deadline = now() + 5_000;
        while now() < deadline && self.state_of(id) == Some(UState::Stopping) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if self.state_of(id) == Some(UState::Stopping) {
            self.kill_pid(id);
        }
        Some((from, UState::Stopped))
    }

    pub async fn load(self: &Arc<Self>, id: &str) -> Option<(UState, UState)> {
        let from = self.state_of(id)?;
        if from != UState::Stopped && from != UState::Blocked {
            return None;
        }
        {
            let mut g = lock(&self.slots);
            if let Some(s) = g.iter_mut().find(|s| s.conf.id == id) {
                s.starts.clear();
                s.restarts = 0;
            }
        }
        self.clone().launch(id).await;
        Some((from, UState::Starting))
    }

    pub fn begin_close(&self) {
        self.closing.store(true, std::sync::atomic::Ordering::Release);
    }

    pub async fn stop_all(self: &Arc<Self>) {
        self.begin_close();
        let mut ids = self.ids(None);
        ids.reverse();
        for id in ids {
            if matches!(self.state_of(&id), Some(UState::Running) | Some(UState::Starting)) {
                self.stop(&id).await;
            }
        }
    }

    pub fn epoch_of(&self, id: &str) -> Option<String> {
        lock(&self.slots).iter().find(|s| s.conf.id == id).and_then(|s| s.epoch.clone())
    }

    pub fn sfu_units(&self) -> Vec<(String, Option<String>, UState, Option<String>)> {
        lock(&self.slots).iter().filter(|s| s.conf.role == UnitRole::Sfu).map(|s| (s.conf.id.clone(), s.epoch.clone(), s.state, s.conf.addr.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_states_down_and_stopped_differ() {
        use UState::*;
        assert_eq!(step(Inactive, &Event::Start), Some(Starting));
        assert_eq!(step(Inactive, &Event::Ready), None, "띄운 적 없는 것이 Running 으로 직행하지 않는다");
        assert_eq!(step(Running, &Event::Exited), Some(Down));
        assert_eq!(step(Stopping, &Event::Exited), Some(Stopped));
        assert_eq!(step(Stopped, &Event::BackoffDue), None, "Stopped 는 되살리지 않는다");
        assert_eq!(step(Down, &Event::BackoffDue), Some(Starting));
        assert_eq!(step(Disabled, &Event::Start), None);
    }

    #[test]
    fn intensity_counts_failed_execs_in_window() {
        let starts: VecDeque<u64> = [1000, 2000, 3000, 4000].into_iter().collect();
        assert!(intensity_exceeded(&starts, 5000, 60_000, 3));
        assert!(!intensity_exceeded(&starts, 5000, 60_000, 4));
        assert!(!intensity_exceeded(&starts, 100_000, 60_000, 3));
    }
}
