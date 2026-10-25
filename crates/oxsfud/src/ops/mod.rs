// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-DOC-PLANES 정:CAND-DOC-ORCHESTRATION-ASPECT 정:RULE-BUS-SHAPE 정:RULE-BUS-FATE 정:CAND-BUS-SELF-WATCH-VALUES 정:RULE-OPS-LOGGER 정:RULE-OPS-SURFACE 정:RULE-CFG-THREE-PLACES 정:RULE-LINK-TRANSPORT 정:CAND-LINK-UDP-BUFFER 정:IMPL-ROOM-VERSION-ISSUE 운영:RULE-LOG-SENDER 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! sfud 조율 — 기동(배너 · 로깅 · 인증서 · 소켓 · 버스 · Handle) · 레지스트리(방 · Peer) · 통지 내기 · 자가 감시 · 부모 생존.

pub mod admin;
pub mod aff;
pub mod call;
pub mod drops;
pub mod floor;
pub mod handle;
pub mod join;
pub mod peer;
pub mod publish;
pub mod sub;

use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwapOption;
use common::log::{LogHandle, LogOpts, Sink};
use dashmap::DashMap;
use oxsig::Op;
use serde::Serialize;

use self::drops::Drops;
use self::peer::Peer;
use crate::bus::Bus;
use crate::cfg::Config;
use crate::fwd::Plane;
use crate::link::dtls::Cert;
use crate::link::transport::{Io, SOCKET_BUF};
use crate::room::Room;

pub const WATCHDOG_BEAT_MS: u64 = 500;
pub const WATCHDOG_STALL_MS: u64 = 10_000;

pub struct Sfu {
    pub cfg: Config,
    pub epoch: String,
    pub log: LogHandle,
    pub bus: Arc<Bus>,
    pub cert: Arc<Cert>,
    pub io: Arc<Io>,
    pub plane: Plane,
    pub rooms: DashMap<String, Arc<Room>>,
    pub peers: DashMap<String, Arc<Peer>>,
    pub drops: Drops,
    pub hub_inst: ArcSwapOption<String>,
    pub shutting: AtomicBool,
    pub rooms_op: tokio::sync::Mutex<()>,
    ids: AtomicU64,
    beat: Arc<AtomicU64>,
}

impl Sfu {
    pub fn next_id(&self) -> u64 {
        self.ids.fetch_add(1, Ordering::Relaxed)
    }

    pub fn room(&self, id: &str) -> Option<Arc<Room>> {
        self.rooms.get(id).map(|r| r.clone())
    }

    pub fn peer(&self, user: &str) -> Option<Arc<Peer>> {
        self.peers.get(user).map(|p| p.clone())
    }

    pub fn emit_room<T: Serialize>(&self, room: &str, op: Op, body: &T, exclude: Vec<String>) {
        if let Ok(s) = serde_json::to_string(body) {
            self.bus.room_event(room, op.as_u16(), s, exclude);
        }
    }

    pub fn emit_user<T: Serialize>(&self, user: &str, op: Op, body: &T, room: Option<&str>) {
        if let Ok(s) = serde_json::to_string(body) {
            self.bus.user_event(user, op.as_u16(), s, room);
        }
    }
}

fn set_buf(sock: &std::net::UdpSocket) {
    use nix::sys::socket::{setsockopt, sockopt};
    let fd = sock.as_raw_fd();
    let _ = setsockopt(fd, sockopt::SndBuf, &SOCKET_BUF);
    let _ = setsockopt(fd, sockopt::RcvBuf, &SOCKET_BUF);
}

fn parent_watch(stop: Arc<tokio::sync::Notify>) {
    let ppid = std::os::unix::process::parent_id();
    let fifo = std::fs::metadata("/dev/stdin").map(|m| std::os::unix::fs::FileTypeExt::is_fifo(&m.file_type())).unwrap_or(false);
    if fifo {
        let s2 = stop.clone();
        let _ = std::thread::Builder::new().name("ox-parent".into()).spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                match std::io::Read::read(&mut std::io::stdin(), &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            tracing::warn!("parent channel closed — shutting down");
            s2.notify_one();
        });
    }
    let _ = std::thread::Builder::new().name("ox-orphan".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if std::os::unix::process::parent_id() != ppid {
                tracing::warn!("parent changed (orphan) — shutting down");
                stop.notify_one();
                std::thread::sleep(Duration::from_secs(3));
                std::process::exit(0);
            }
        }
    });
}

fn watchdog(beat: Arc<AtomicU64>) {
    let _ = std::thread::Builder::new().name("ox-watchdog".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(WATCHDOG_BEAT_MS));
            let last = beat.load(Ordering::Acquire);
            let now = common::log::now_ms();
            if last != 0 && now.saturating_sub(last) > WATCHDOG_STALL_MS {
                eprintln!("oxsfud watchdog: runtime stalled {} ms — exiting", now.saturating_sub(last));
                tracing::error!("runtime stalled {} ms — self-watchdog exit", now.saturating_sub(last));
                std::thread::sleep(Duration::from_millis(200));
                std::process::exit(70);
            }
        }
    });
}

pub async fn run(cfg: Config) -> Result<(), String> {
    let epoch = common::id::instance();
    let p = &cfg.policy.logging;
    let sink = match (&cfg.log_socket, &cfg.log_dir) {
        (Some(s), Some(d)) => Sink::Socket { path: s.into(), spool: common::log::spool_path(d, &cfg.node_id, "sfu"), spool_max: p.spool_max_bytes },
        (Some(s), None) => Sink::Socket { path: s.into(), spool: common::log::spool_path("/tmp", &cfg.node_id, "sfu"), spool_max: p.spool_max_bytes },
        _ => Sink::Stdout,
    };
    let log = common::log::init(LogOpts {
        node: cfg.node_id.clone(),
        proc_: "sfu",
        level: common::log::env_level(p.level),
        line_max: p.line_max_bytes as usize,
        queue_max: p.queue_max_lines as usize,
        sink,
    });
    log.set_epoch(&epoch);
    common::ev!(spec = "SV-STATE-UNIT-STATE", build = cfg.build.as_str(), "{} level={}", cfg.banner(&epoch), common::log::level_name(log.level()));
    let cert = Arc::new(crate::link::dtls::make_cert()?);
    let std_udp = std::net::UdpSocket::bind((cfg.bind_ip, cfg.udp_port)).map_err(|e| format!("udp bind {}:{}: {e}", cfg.bind_ip, cfg.udp_port))?;
    set_buf(&std_udp);
    std_udp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let udp = Arc::new(tokio::net::UdpSocket::from_std(std_udp).map_err(|e| e.to_string())?);
    let tcp = match cfg.tcp_port {
        Some(port) => Some(tokio::net::TcpListener::bind((cfg.bind_ip, port)).await.map_err(|e| format!("tcp bind {port}: {e}"))?),
        None => None,
    };
    let bus = Bus::open(&cfg.zenoh_connect, &cfg.zenoh_namespace, &cfg.node_id, &epoch).await?;
    let beat = Arc::new(AtomicU64::new(0));
    let grpc = cfg.grpc_listen.clone();
    let sfu = Arc::new(Sfu {
        cfg,
        epoch,
        log,
        bus,
        cert,
        io: Arc::new(Io::new(udp)),
        plane: Plane::default(),
        rooms: DashMap::new(),
        peers: DashMap::new(),
        drops: Drops::default(),
        hub_inst: ArcSwapOption::empty(),
        shutting: AtomicBool::new(false),
        rooms_op: tokio::sync::Mutex::new(()),
        ids: AtomicU64::new(1),
        beat: beat.clone(),
    });
    watchdog(beat.clone());
    let b2 = beat.clone();
    tokio::spawn(async move {
        loop {
            b2.store(common::log::now_ms(), Ordering::Release);
            tokio::time::sleep(Duration::from_millis(WATCHDOG_BEAT_MS)).await;
        }
    });
    let stop = Arc::new(tokio::sync::Notify::new());
    parent_watch(stop.clone());
    tokio::spawn(crate::fwd::shell::udp_loop(sfu.clone()));
    if let Some(l) = tcp {
        tokio::spawn(crate::fwd::shell::tcp_listen(sfu.clone(), l));
    }
    crate::gc::start(&sfu);
    let addr: std::net::SocketAddr = grpc.parse().map_err(|e| format!("--grpc-listen {grpc}: {e}"))?;
    let svc = common::handle::HandleServer::new(handle::Svc { sfu: sfu.clone() });
    let s3 = stop.clone();
    let serve = tonic::transport::Server::builder().add_service(svc).serve_with_shutdown(addr, async move {
        tokio::select! {
            _ = s3.notified() => {}
            _ = tokio::signal::ctrl_c() => {}
            _ = term() => {}
        }
    });
    tracing::info!("sfud ready grpc={addr} udp={} epoch={}", sfu.cfg.udp_port, sfu.epoch);
    let r = serve.await.map_err(|e| e.to_string());
    sfu.shutting.store(true, Ordering::Release);
    common::ev!(spec = "SV-STATE-UNIT-STATE", "sfud shutdown epoch={}", sfu.epoch);
    sfu.bus.close().await;
    let _ = sfu.beat.load(Ordering::Relaxed);
    r
}

async fn term() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut s) => {
            s.recv().await;
        }
        Err(_) => std::future::pending::<()>().await,
    }
}
