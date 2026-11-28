// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-OPS-SUPERVISOR · 정:RULE-BUS-FATE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-SHAPE · 정:IMPL-FLR-TALK-REPORT · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 녹음 유닛 본체 — 인자(`--id` · `--build` · `--bus-namespace` · `--bus-connect` · `--log-dir`? · `--log-socket`? · `--data-root` · `--rec-keep-days` · `--token` · `--backoff-ms` · `--intensity-max` · `--intensity-window-secs` · `--hub-base` · `--hub-ca`? · `--grpc-listen`) · 등록부를 받아 Handle 을 연다(등록은 그 전에 끝난다) · 기동 회수(제 node 의 쓰는 중 조각 · 방 작업을 열기 전) · 버스로 그 node 의 방 토큰을 보고 방마다 방 작업 하나 · 보관 지우기(기동 때 · 하루 한 번) · 부모 생존 채널(stdin) 닫힘 · 시그널 · 버스 끊김이면 방 작업마다 정상으로 닫고 끝낸다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use common::bus::TokenEvent;
use common::keyspace::Parsed;
use common::log::{Setup, Sink};
use common::talk::Report;
use tokio::sync::{mpsc, watch};

use crate::bus::Bus;
use crate::consumer::{RecoverCtx, Registry, SPEC};
use crate::files::Layout;
use crate::job::{Env, Presence, room_actor};

pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);
pub const REPORT_GRACE: Duration = Duration::from_secs(10);

pub struct Args {
    pub node: String,
    pub build: String,
    pub ns: String,
    pub connect: String,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub root: PathBuf,
    pub keep_days: u32,
    pub token: String,
    pub hub_base: String,
    pub hub_ca: Option<PathBuf>,
    pub grpc: std::net::SocketAddr,
    pub restart: crate::Restart,
}

pub const ARGS: [&str; 15] = ["id", "build", "bus-namespace", "bus-connect", "log-dir", "log-socket", "data-root", "rec-keep-days", "token", "hub-base", "hub-ca", "grpc-listen", "backoff-ms", "intensity-max", "intensity-window-secs"];

pub fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    let mut it = argv.into_iter();
    while let Some(k) = it.next() {
        let Some(name) = k.strip_prefix("--") else { return Err(format!("unknown argument {k}")) };
        if !ARGS.contains(&name) {
            return Err(format!("unknown argument --{name}"));
        }
        let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
        m.insert(name.to_string(), v);
    }
    let need = |k: &str| m.get(k).cloned().ok_or_else(|| format!("--{k} missing"));
    Ok(Args {
        node: need("id")?,
        build: m.get("build").cloned().unwrap_or_default(),
        ns: need("bus-namespace")?,
        connect: need("bus-connect")?,
        log_dir: m.get("log-dir").map(PathBuf::from),
        log_socket: m.get("log-socket").map(PathBuf::from),
        root: PathBuf::from(need("data-root")?),
        keep_days: m.get("rec-keep-days").map(|v| v.parse().map_err(|_| "--rec-keep-days".to_string())).transpose()?.unwrap_or(common::system::REC_KEEP_DAYS_DEFAULT),
        token: need("token")?,
        hub_base: need("hub-base")?,
        hub_ca: m.get("hub-ca").map(PathBuf::from),
        grpc: need("grpc-listen")?.parse().map_err(|e| format!("--grpc-listen: {e}"))?,
        restart: crate::Restart::parse(m.get("backoff-ms").map(String::as_str), m.get("intensity-max").map(String::as_str), m.get("intensity-window-secs").map(String::as_str))?,
    })
}

pub async fn main_with(registry: Registry) -> ! {
    let a = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxtapd: {e}");
            std::process::exit(2);
        }
    };
    std::process::exit(run(a, registry).await);
}

enum RoomCmd {
    Up(String),
    Down(String),
    BusGone,
}

pub async fn run(a: Args, registry: Registry) -> i32 {
    let policy = common::policy::Policy::default();
    let sink = match &a.log_socket {
        Some(p) => Sink::Socket(p.clone()),
        None => Sink::Stdout,
    };
    let epoch = common::ident::new_epoch();
    let _sender = common::log::init(Setup { node: a.node.clone(), proc_name: "peer", epoch: Some(epoch.clone()), sink, log_dir: a.log_dir.clone() }, &policy);
    let kinds = registry.names();
    tracing::info!(kind = "diag", spec = SPEC, build = %a.build, root = %a.root.display(), keep_days = a.keep_days, backoff_ms = ?a.restart.backoff_ms, intensity_max = a.restart.intensity_max, intensity_window_secs = a.restart.intensity_window.as_secs(), kinds = ?kinds, "rec unit boot");
    let ca = match &a.hub_ca {
        Some(p) => match std::fs::read(p) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("oxtapd: --hub-ca {}: {e}", p.display());
                return 2;
            }
        },
        None => None,
    };
    let Some(hub) = crate::hub_of(&a.hub_base, ca.as_deref()) else {
        eprintln!("oxtapd: --hub-base must be http(s)://host:port/base (https needs --hub-ca)");
        return 2;
    };
    let layout = Layout::new(&a.root, &a.node);
    if let Err(e) = std::fs::create_dir_all(layout.node_staging()) {
        eprintln!("oxtapd: staging {}: {e}", layout.node_staging().display());
        return 2;
    }
    let drops = Arc::new(AtomicU64::new(0));
    let (tok_tx, tok_rx) = watch::channel(a.token.clone());
    let svc = crate::Svc { epoch, build: common::ident::build_id(common::ident::COMPILED, Some(&a.build)), drops: drops.clone(), token: tok_tx, kinds };
    let addr = a.grpc;
    tokio::spawn(async move {
        if let Err(e) = tonic::transport::Server::builder().add_service(common::envelope::handle::handle_server::HandleServer::new(svc)).serve(addr).await {
            eprintln!("oxtapd: --grpc-listen {addr}: {e}");
            common::log::drain(Duration::from_millis(200));
            std::process::exit(2);
        }
    });
    let bus = match Bus::open(&a.node, &a.ns, &a.connect).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("oxtapd: bus: {e}");
            return 2;
        }
    };
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<(&'static str, i32)>();
    let st = stop_tx.clone();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink);
        let _ = st.send(("parent channel closed", 0));
    });
    for (sig, what) in [(tokio::signal::unix::SignalKind::terminate(), "SIGTERM"), (tokio::signal::unix::SignalKind::interrupt(), "SIGINT")] {
        if let Ok(mut s) = tokio::signal::unix::signal(sig) {
            let st = stop_tx.clone();
            tokio::spawn(async move {
                s.recv().await;
                let _ = st.send((what, 0));
            });
        }
    }
    let (sweep_root, keep) = (a.root.clone(), a.keep_days);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(crate::DAY);
        loop {
            tick.tick().await;
            match crate::sweep(&sweep_root, keep, std::time::SystemTime::now()) {
                Ok(n) if n > 0 => tracing::info!(kind = "diag", spec = SPEC, removed = n, "rec files past keep days removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!(kind = "diag", spec = SPEC, "rec sweep: {e}"),
            }
        }
    });
    let (rep_tx, rep_rx) = mpsc::unbounded_channel::<Report>();
    let (b2, d2) = (bus.clone(), drops.clone());
    let line = tokio::spawn(async move {
        crate::report_line(
            rep_rx,
            common::talk::RETRY_FOR,
            |p| {
                let b = b2.clone();
                async move { b.ask(common::talk::WHAT, p).await }
            },
            &d2,
        )
        .await
    });
    let registry = Arc::new(registry);
    registry.recover(&RecoverCtx { layout: layout.clone(), runs: layout.runs(), reports: rep_tx.clone(), reason: "crash" });
    let env = Arc::new(Env { layout, hub, hub_base: a.hub_base.clone(), ca, token: tok_rx, reports: rep_tx, registry, restart: a.restart.clone(), fatal: stop_tx.clone() });
    let (room_tx, mut room_rx) = mpsc::unbounded_channel::<RoomCmd>();
    if let Err(e) = discover(&bus, room_tx).await {
        eprintln!("oxtapd: bus watch: {e}");
        return 2;
    }
    let mut rooms: HashMap<String, (watch::Sender<Presence>, tokio::task::JoinHandle<()>)> = HashMap::new();
    let mut turn = 0u64;
    let code = loop {
        tokio::select! {
            why = stop_rx.recv() => {
                let (why, code) = why.unwrap_or(("-", 1));
                tracing::info!(kind = "diag", spec = SPEC, why = why, code, "rec unit stopping — closing every room job");
                break code;
            }
            c = room_rx.recv() => match c {
                Some(RoomCmd::Up(r)) => {
                    turn += 1;
                    let up = Presence { up: true, turn };
                    match rooms.get(&r) {
                        Some((tx, _)) => {
                            tx.send_replace(up);
                        }
                        None => {
                            let (tx, rx) = watch::channel(up);
                            let h = tokio::spawn(room_actor(env.clone(), r.clone(), rx));
                            rooms.insert(r, (tx, h));
                        }
                    }
                }
                Some(RoomCmd::Down(r)) => {
                    if let Some((tx, _)) = rooms.get(&r) {
                        tx.send_modify(|p| p.up = false);
                    }
                }
                Some(RoomCmd::BusGone) | None => {
                    tracing::error!(kind = "diag", spec = SPEC, "rec unit bus watch ended — closing every room job");
                    break 1;
                }
            },
        }
    };
    let mut handles = Vec::new();
    for (_, (tx, h)) in rooms {
        tx.send_modify(|p| p.up = false);
        drop(tx);
        handles.push(h);
    }
    let closed = tokio::time::timeout(SHUTDOWN_GRACE, async {
        for h in handles {
            let _ = h.await;
        }
    })
    .await;
    if closed.is_err() {
        tracing::warn!(kind = "diag", spec = SPEC, grace_s = SHUTDOWN_GRACE.as_secs(), "rec unit stop grace passed — open fragments are left for recovery");
    }
    drop(env);
    if tokio::time::timeout(REPORT_GRACE, line).await.is_err() {
        tracing::warn!(kind = "diag", spec = SPEC, grace_s = REPORT_GRACE.as_secs(), "rec reports still queued at stop — not sent");
    }
    common::log::drain(Duration::from_millis(200));
    code
}

async fn discover(bus: &Bus, tx: mpsc::UnboundedSender<RoomCmd>) -> Result<(), String> {
    let mut sub = bus.rooms().await?;
    let keys = bus.keys.clone();
    tokio::spawn(async move {
        let mut live: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        while let Some(ev) = sub.recv().await {
            let (k, put) = match ev {
                TokenEvent::Put(k) => (k, true),
                TokenEvent::Delete(k) => (k, false),
                TokenEvent::BatchEnd => continue,
            };
            let Some(Parsed::Room { room, inst, .. }) = keys.parse(&k) else { continue };
            let insts = live.entry(room.clone()).or_default();
            let was = !insts.is_empty();
            if put {
                insts.insert(inst);
            } else {
                insts.remove(&inst);
            }
            let now = !insts.is_empty();
            if !now {
                live.remove(&room);
            }
            match (was, now) {
                (false, true) => {
                    let _ = tx.send(RoomCmd::Up(room));
                }
                (true, false) => {
                    let _ = tx.send(RoomCmd::Down(room));
                }
                _ => {}
            }
        }
        let _ = tx.send(RoomCmd::BusGone);
    });
    Ok(())
}
