// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · 정:RULE-BUS-FATE · 정:IMPL-CFG-SYSTEM-FILE · 정:IMPL-BUS-FAULTS · 운영:RULE-CTL-SUPERVISOR · 정:CAND-OPS-SUPERVISOR-DEFAULTS · 정:CAND-OPS-SHUTDOWN-NO-RESTART · 정:CAND-OPS-SUPERVISOR-EXTRA-TRANSITIONS · 정:CAND-OPS-ZENOH-UNIT-DOWN · 정:CAND-OPS-UNIT-STATE-LINE · 정:IMPL-OPS-REC-UNIT · 정:RULE-OPS-LOGGER · model: claude-opus-5-5
//! supervisor — `UnitState` 여덟 · 못 띄운 것도 기동 시도로 센다 · `Stopped` 뒤 기동은 창에 안 쌓는다 · 준비 = Hello 성공(녹음 유닛도 같다 — 그 Handle 은 유닛이 쥐고 로컬 미디어 서버 길과 섞지 않는다 · ready 에 세지 않는다 · 띄울 때마다 녹화 참가자 토큰) · 부모 생존 채널(stdin) · 종료 중엔 되살리지 않는다 · remote 는 같은 기계의 sfud 에 붙기만 한다(포트 기다림 없음 · 붙은 동안 1초마다 `Hello` — 실패 · 기동 신원 바뀜이면 `Down` · `stop` = 떼기 · `kill` = 떼고 다시 붙기) · 녹음 유닛 토큰은 수명의 3분의 1 마다 새로 서명해 `Call` 로 넘긴다 · 꺼 둔 유닛과 로거 유닛에 온 명령은 거절한다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::envelope::handle::handle_client::HandleClient;
use common::envelope::handle::{CallReq, HelloReq};
use common::envelope::{CallKind, Envelope, Reply};
use common::system::{Unit, UnitKind};
use oxsig::Code;
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
    Bus,
    Sfu,
    Rec,
}

pub struct RecLaunch {
    pub args: Vec<String>,
    pub accounts: crate::ses::token::Accounts,
    pub ttl_secs: u32,
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
    pub handle: Mutex<Option<Local>>,
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
    pub sfu_args: Vec<String>,
    pub rec: Option<RecLaunch>,
}

pub struct Supervisor {
    pub units: Vec<Arc<UnitHandle>>,
    pub cfg: SupCfg,
    pub stopping: AtomicBool,
    pub node_declared: AtomicBool,
    pub fatal: mpsc::UnboundedSender<String>,
    record_kinds: Arc<Mutex<Option<Vec<String>>>>,
}

pub fn supported_kinds(has_rec_unit: bool, read: Option<&[String]>) -> Option<Vec<String>> {
    match (has_rec_unit, read) {
        (false, _) => Some(Vec::new()),
        (true, Some(k)) => Some(k.to_vec()),
        (true, None) => None,
    }
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
                handle: Mutex::new(None),
                cfg: u,
                tx,
                rx: Mutex::new(Some(rx)),
            })
        };
        for u in units_cfg.iter().filter(|u| u.role == "log") {
            units.push(mk(u.clone(), Role::Log));
        }
        let z = Unit {
            id: "bus".into(),
            kind: UnitKind::Inproc,
            order: 0,
            role: "bus".into(),
            addr: None,
            cmd: Vec::new(),
            enabled: true,
        };
        units.push(mk(z, Role::Bus));
        let mut rest: Vec<&Unit> = units_cfg.iter().filter(|u| u.role != "log").collect();
        rest.sort_by_key(|u| u.order);
        for u in rest {
            let role = if u.role == common::system::ROLE_REC { Role::Rec } else { Role::Sfu };
            units.push(mk(u.clone(), role));
        }
        Arc::new(Supervisor {
            units,
            cfg,
            stopping: AtomicBool::new(false),
            node_declared: AtomicBool::new(false),
            fatal,
            record_kinds: Arc::default(),
        })
    }

    pub fn unit(&self, id: &str) -> Option<Arc<UnitHandle>> {
        self.units.iter().find(|u| u.id == id).cloned()
    }

    pub fn sfu_units(&self) -> Vec<Arc<UnitHandle>> {
        self.units.iter().filter(|u| u.role == Role::Sfu).cloned().collect()
    }

    pub fn rec_units(&self) -> Vec<Arc<UnitHandle>> {
        self.units.iter().filter(|u| u.role == Role::Rec).cloned().collect()
    }

    pub fn process_units(&self) -> Vec<Arc<UnitHandle>> {
        self.units.iter().filter(|u| matches!(u.role, Role::Sfu | Role::Rec)).cloned().collect()
    }

    pub fn down_units(&self) -> Vec<String> {
        self.units
            .iter()
            .filter(|u| u.role != Role::Rec)
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
        for u in self.process_units() {
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
        c.args(["--build", self.cfg.build_label.as_deref().unwrap_or("")]);
        c.args(&self.cfg.extra_args);
        match u.role {
            Role::Rec => {
                let r = self.cfg.rec.as_ref()?;
                c.args(&r.args);
                let Some(tok) = r.accounts.sign_unit(&rec_user(&self.cfg.node), r.ttl_secs) else {
                    tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", unit = %u.id, "rec unit token not signed — no unit key (os random unavailable), unit not started");
                    return None;
                };
                c.args(["--token", &tok]);
            }
            _ => {
                c.args(&self.cfg.sfu_args);
            }
        }
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
                tracing::warn!(kind = "diag", spec = "SV-RULE-BUS-PRESENCE", unit = %u.id, "node token: {e}");
            } else {
                common::log::emit(common::log::Line::diag("SV-RULE-BUS-PRESENCE").key("unit", &u.id).msg("node token declared"));
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
                if u.kind == UnitKind::Process {
                    self.wait_port_free(&u).await;
                }
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
            if u.role == Role::Rec {
                let Some((local, build)) = self.hello(&u).await else {
                    if let Some(mut c) = child {
                        let _ = c.start_kill();
                        let _ = c.wait().await;
                    }
                    self.go_down(&u, "hello timeout");
                    continue;
                };
                if let Ok(mut r) = u.rec.lock() {
                    r.epoch = Some(local.epoch.clone());
                    r.build = Some(build);
                }
                if let Ok(mut h) = u.handle.lock() {
                    let _ = h.replace(local);
                }
                u.set(UnitState::Running, "hello");
                self.read_kinds(&u);
                let outcome = self.supervise(&u, child, &mut rx, None).await;
                if let Ok(mut h) = u.handle.lock() {
                    let _ = h.take();
                }
                if let Ok(mut r) = u.rec.lock() {
                    r.epoch = None;
                }
                match outcome {
                    Outcome::Exited | Outcome::Killed | Outcome::Lost(_) => {
                        if self.stopping.load(Ordering::SeqCst) {
                            u.set(UnitState::Stopped, "shutdown");
                            return;
                        }
                        self.go_down(&u, outcome.cause());
                    }
                    Outcome::Stopped => {}
                    Outcome::Gone => return,
                }
                continue;
            }
            let Some((local, build)) = self.hello(&u).await else {
                if let Some(mut c) = child {
                    let _ = c.start_kill();
                    let _ = c.wait().await;
                }
                self.go_down(&u, "hello timeout");
                continue;
            };
            let watch = (u.kind == UnitKind::Remote).then(|| Watch { client: local.client.clone(), epoch: local.epoch.clone() });
            self.became_running(&u, &bus, local, build).await;
            let outcome = self.supervise(&u, child, &mut rx, watch).await;
            bus.set_local(None);
            if let Ok(mut r) = u.rec.lock() {
                r.epoch = None;
            }
            match outcome {
                Outcome::Exited | Outcome::Killed | Outcome::Lost(_) => {
                    if self.stopping.load(Ordering::SeqCst) {
                        u.set(UnitState::Stopped, "shutdown");
                        return;
                    }
                    self.go_down(&u, outcome.cause());
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

    async fn remote_lost(&self, w: &mut Option<(Watch, tokio::time::Interval)>) -> &'static str {
        let Some((w, tick)) = w.as_mut() else { return std::future::pending().await };
        loop {
            tick.tick().await;
            let req = HelloReq { node: self.cfg.node.clone(), hub_inst: self.cfg.hub_inst.clone() };
            match tokio::time::timeout(REMOTE_CHECK, w.client.hello(req)).await {
                Ok(Ok(r)) if r.get_ref().epoch == w.epoch => {}
                Ok(Ok(_)) => return "remote identity changed",
                Ok(Err(_)) => return "remote lost",
                Err(_) => {}
            }
        }
    }

    async fn supervise(&self, u: &UnitHandle, child: Option<tokio::process::Child>, rx: &mut mpsc::UnboundedReceiver<Cmd>, watch: Option<Watch>) -> Outcome {
        let Some(mut child) = child else {
            let mut watch = watch.map(|w| (w, tokio::time::interval_at(tokio::time::Instant::now() + REMOTE_CHECK, REMOTE_CHECK)));
            loop {
                tokio::select! {
                    why = self.remote_lost(&mut watch) => return Outcome::Lost(why),
                    c = rx.recv() => match c {
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
            }
        };
        let _parent_channel = child.stdin.take();
        let mut renew = match (u.role, self.cfg.rec.as_ref()) {
            (Role::Rec, Some(r)) => {
                let every = renew_every(r.ttl_secs);
                let mut t = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
                t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                Some(t)
            }
            _ => None,
        };
        loop {
            tokio::select! {
                _ = child.wait() => return Outcome::Exited,
                _ = next_tick(&mut renew) => self.hand_token(u),
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

    fn hand_token(&self, u: &UnitHandle) {
        let Some(r) = self.cfg.rec.as_ref() else { return };
        let Some(tok) = r.accounts.sign_unit(&rec_user(&self.cfg.node), r.ttl_secs) else {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", unit = %u.id, "rec unit token not signed — no unit key (os random unavailable), token not handed");
            return;
        };
        let Some(mut client) = u.handle.lock().ok().and_then(|h| h.as_ref().map(|l| l.client.clone())) else { return };
        let mut env = Envelope::new(CallKind::Admin);
        env.method = Some("POST".into());
        env.path = Some(format!("/{}", common::system::REC_TOKEN_PATH));
        env.body = serde_json::json!({ "token": tok });
        let id = u.id.clone();
        tokio::spawn(async move {
            let req = CallReq { envelope: env.encode() };
            let why = match tokio::time::timeout(crate::bus::LOCAL_CALL_TIMEOUT, client.call(req)).await {
                Ok(Ok(r)) => match Reply::decode(&r.into_inner().reply) {
                    Ok(Reply::Ok { .. }) => None,
                    Ok(other) => Some(format!("refused {other:?}")),
                    Err(e) => Some(format!("bad reply {e}")),
                },
                Ok(Err(e)) => Some(format!("call failed {:?}", e.code())),
                Err(_) => Some("call timed out".to_string()),
            };
            match why {
                None => common::log::emit(common::log::Line::diag("SV-IMPL-OPS-REC-UNIT").key("unit", &id).msg("rec unit token handed")),
                Some(w) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", unit = %id, "rec unit token not handed — {w} — the next period tries again"),
            }
        });
    }

    pub fn record_kinds(&self) -> Option<Vec<String>> {
        let has_rec = self.units.iter().any(|u| u.role == Role::Rec);
        let read = self.record_kinds.lock().ok().and_then(|g| g.clone());
        supported_kinds(has_rec, read.as_deref())
    }

    fn read_kinds(&self, u: &UnitHandle) {
        let Some(mut client) = u.handle.lock().ok().and_then(|h| h.as_ref().map(|l| l.client.clone())) else { return };
        let mut env = Envelope::new(CallKind::Admin);
        env.method = Some("GET".into());
        env.path = Some(format!("/{}", common::system::REC_KINDS_PATH));
        let id = u.id.clone();
        let store = self.record_kinds.clone();
        tokio::spawn(async move {
            let req = CallReq { envelope: env.encode() };
            let got = match tokio::time::timeout(crate::bus::LOCAL_CALL_TIMEOUT, client.call(req)).await {
                Ok(Ok(r)) => match Reply::decode(&r.into_inner().reply) {
                    Ok(Reply::Ok { body }) => serde_json::from_value::<Vec<String>>(body.get("kinds").cloned().unwrap_or_default()).map_err(|e| format!("bad kinds {e}")),
                    Ok(other) => Err(format!("refused {other:?}")),
                    Err(e) => Err(format!("bad reply {e}")),
                },
                Ok(Err(e)) => Err(format!("call failed {:?}", e.code())),
                Err(_) => Err("call timed out".to_string()),
            };
            match got {
                Ok(k) => {
                    common::log::emit(common::log::Line::diag("SV-IMPL-OPS-REC-UNIT").key("unit", &id).key("kinds", k.join(",")).msg("rec unit record kinds read"));
                    if let Ok(mut g) = store.lock() {
                        let _ = g.replace(k);
                    }
                }
                Err(w) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", unit = %id, "rec unit record kinds not read — {w} — the last read set stays"),
            }
        });
    }

    pub async fn request(&self, id: &str, which: &str) -> Result<(UnitState, UnitState), String> {
        let u = self.unit(id).ok_or_else(|| "no unit".to_string())?;
        if u.state() == UnitState::Disabled {
            return Err("disabled unit — enable it in the system file and restart the hub".into());
        }
        if u.role == Role::Log {
            return Err("inproc unit — the logger has no stop device, it ends with the hub".into());
        }
        if u.role == Role::Bus {
            if which == "load" {
                return Err("inproc unit".into());
            }
            let from = u.state();
            let _ = self.fatal.send(format!("bus unit {which}"));
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
        let mut units: Vec<Arc<UnitHandle>> = self.process_units();
        units.reverse();
        for u in units {
            if u.state() == UnitState::Running {
                let _ = self.request(&u.id, "stop").await;
            }
        }
    }
}

pub fn rec_user(node: &str) -> String {
    format!("rec-{node}")
}

pub fn rec_room_user(node: &str, room: &str) -> String {
    format!("rec-{node}-{room}")
}

pub fn grant_room_session(node: &str, unit_user: &str, room: &str, room_node: Option<&str>) -> Result<String, Code> {
    if unit_user != rec_user(node) {
        return Err(Code::TokenInvalid);
    }
    if room_node != Some(node) {
        return Err(Code::RoomNotFound);
    }
    Ok(rec_room_user(node, room))
}

pub fn renew_every(ttl_secs: u32) -> Duration {
    Duration::from_secs(u64::from(ttl_secs / 3).max(1))
}

async fn next_tick(t: &mut Option<tokio::time::Interval>) {
    match t {
        Some(t) => {
            t.tick().await;
        }
        None => std::future::pending::<()>().await,
    }
}

enum Outcome {
    Exited,
    Killed,
    Lost(&'static str),
    Stopped,
    Gone,
}

impl Outcome {
    fn cause(&self) -> &'static str {
        match self {
            Outcome::Killed => "kill",
            Outcome::Lost(why) => *why,
            _ => "exit",
        }
    }
}

struct Watch {
    client: HandleClient<tonic::transport::Channel>,
    epoch: String,
}

pub const REMOTE_CHECK: Duration = Duration::from_secs(1);

async fn terminate(child: &mut tokio::process::Child, grace: Duration) {
    if let Some(pid) = child.id() {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), nix::sys::signal::Signal::SIGTERM);
    }
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}
