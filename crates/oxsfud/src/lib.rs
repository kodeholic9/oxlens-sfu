// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-DOC-OWNERSHIP · 정:RULE-DOC-CODE-MIRROR · 정:RULE-BUS-FATE · 정:RULE-CONC-SIX · 정:RULE-DOC-SCOPE-SERVER · model: claude-opus-5-5
//! `oxsfud` — 미디어 서버: Peer · 방 · 발행 · 구독 · 발언권. 영역마다 한 디렉터리(개념 link · room · aff · pub · trk · sub · flr · bus / 가로지르는 것 fwd · sim · rtcp · aux · ops · gc · cfg).

pub mod aff;
pub mod aux;
pub mod bus;
pub mod cfg;
pub mod flr;
pub mod fwd;
pub mod gc;
pub mod link;
pub mod ops;
pub mod r#pub;
pub mod room;
pub mod rtcp;
pub mod sim;
pub mod sub;
pub mod trk;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;

pub async fn run(boot: cfg::Boot) -> Result<(), String> {
    let _ = common::clock::start();
    common::log::init(common::log::LogConf {
        node: boot.node.clone(),
        proc_name: "sfu".to_string(),
        socket: boot.log_socket.as_ref().map(PathBuf::from),
        log_dir: boot.log_dir.as_ref().map(PathBuf::from),
        level: format!("{:?}", boot.policy.logging.level).to_lowercase(),
        line_max: boot.policy.logging.line_max_bytes as usize,
        queue_max: boot.policy.logging.queue_max_lines as usize,
        spool_max: boot.policy.logging.spool_max_bytes,
        sink: None,
    });
    tracing::info!(kind = "diag", epoch = %boot.epoch, build = %boot.build, banner = %boot.banner(), "sfu boot");
    let cert = link::shell::make_cert()?;
    let udp = link::shell::bind_udp(boot.public_ip, boot.udp_port).await?;
    let tcp = match boot.tcp_port {
        Some(p) => Some(tokio::net::TcpListener::bind(SocketAddr::new(boot.public_ip, p)).await.or(tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], p))).await).map_err(|e| format!("tcp {p}: {e}"))?),
        None => None,
    };
    let links = Arc::new(link::Links {
        peers: DashMap::new(),
        ufrags: DashMap::new(),
        addrs: DashMap::new(),
        arevs: DashMap::new(),
        udp: Arc::new(udp),
        cert,
        ip: boot.public_ip,
        udp_port: boot.udp_port,
        tcp_port: boot.tcp_port,
        slot_reserve: boot.policy.media.slot_mid_reserve as usize,
        sfu_id: boot.epoch.clone(),
        stun_bad: Default::default(),
        unknown_addr: Default::default(),
    });
    let bus = bus::Bus::open(&boot.node, &boot.epoch, &boot.zenoh_ns, boot.zenoh_connect.as_deref()).await?;
    let grpc: SocketAddr = boot.grpc_listen.parse().map_err(|_| format!("--grpc-listen {}", boot.grpc_listen))?;
    let svc = ops::Svc { epoch: boot.epoch.clone(), build: boot.build.clone() };
    let rooms = room::Rooms { map: DashMap::new(), epoch: boot.epoch.clone(), bus: bus.clone(), queue_max: boot.policy.floor.queue_max as usize, t2_secs: boot.policy.floor.t2_stop_talking_secs as u32 };
    let ctx = Arc::new(cfg::Ctx { boot, rooms, links: links.clone(), tracks: trk::Tracks::default(), bus: bus.clone(), drops: ops::Drops::default(), task_seq: Default::default() });
    cfg::install(ctx);
    bus.set_lost(Arc::new(|node, inst, new| {
        if let Some(c) = cfg::get() {
            gc::home_lost(c, &node, &inst, new.as_deref());
        }
    }));
    link::shell::spawn_udp(links.clone());
    if let Some(l) = tcp {
        link::shell::spawn_tcp(links.clone(), l);
    }
    link::shell::spawn_tick(links);
    rtcp::shell::spawn_tick();
    gc::spawn_sweeps();
    tracing::info!(kind = "diag", grpc = %grpc, "sfu listening");
    ops::serve(grpc, svc).await
}
