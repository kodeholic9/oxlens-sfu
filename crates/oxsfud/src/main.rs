// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-CFG-THREE-PLACES 정:RULE-BUS-FATE 정:RULE-OPS-SURFACE 정:IMPL-ROOM-VERSION-ISSUE 운영 규격서 §8-2 · model: claude-opus-5-5
//! oxsfud — 미디어 서버. 방 · 발행 · 구독 · 발언권 · 전송(ICE-lite · DTLS · SRTP · SCTP)의 상태 마스터.

mod admin;
mod bus;
mod floor;
mod layer;
mod media;
mod net;
mod peer;
mod publish;
mod room;
mod rpc;
mod rtcp_io;
mod rtp;
mod secure;
mod session;
mod sfu;
mod stun;
mod table;
mod tick;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use common::policy::Policy;

use crate::sfu::Sfu;

#[derive(Default)]
struct Args {
    grpc_listen: Option<String>,
    udp_port: Option<u16>,
    tcp_port: Option<u16>,
    policy: Option<String>,
    public_ip: Option<String>,
    id: Option<String>,
    build: Option<String>,
    zenoh_namespace: Option<String>,
    zenoh_connect: Option<String>,
    log_dir: Option<String>,
    log_socket: Option<String>,
    version: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{k} 에 값이 없다"));
        match k.as_str() {
            "--grpc-listen" => a.grpc_listen = Some(val()?),
            "--udp-port" => a.udp_port = Some(val()?.parse().map_err(|_| "--udp-port 가 수가 아니다".to_string())?),
            "--tcp-port" => a.tcp_port = Some(val()?.parse().map_err(|_| "--tcp-port 가 수가 아니다".to_string())?),
            "--policy" => a.policy = Some(val()?),
            "--public-ip" => a.public_ip = Some(val()?),
            "--id" => a.id = Some(val()?),
            "--build" => a.build = Some(val()?),
            "--zenoh-namespace" => a.zenoh_namespace = Some(val()?),
            "--zenoh-connect" => a.zenoh_connect = Some(val()?),
            "--log-dir" => a.log_dir = Some(val()?),
            "--log-socket" => a.log_socket = Some(val()?),
            "--version" => a.version = true,
            other => return Err(format!("모르는 인자: {other}")),
        }
    }
    Ok(a)
}

fn identity(label: Option<&str>) -> String {
    common::ident::build_identity(env!("OX_GIT_SHA"), env!("OX_BUILD_TIME"), label)
}

fn auto_ip() -> IpAddr {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn resolve_ip(v: Option<&str>) -> Result<IpAddr, String> {
    match v {
        None => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        Some("auto") => Ok(auto_ip()),
        Some(s) => s.parse().map_err(|_| format!("--public-ip 가 주소가 아니다: {s}")),
    }
}

fn bind_udp(ip: IpAddr, port: u16) -> Result<std::net::UdpSocket, String> {
    match std::net::UdpSocket::bind(SocketAddr::new(ip, port)) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::AddrNotAvailable => {
            std::net::UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn watch_parent() {
    std::thread::Builder::new()
        .name("ox-parent".into())
        .spawn(|| {
            let mut buf = [0u8; 64];
            loop {
                match std::io::Read::read(&mut std::io::stdin(), &mut buf) {
                    Ok(0) | Err(_) => {
                        tracing::warn!(kind = "ev", spec = "RULE-BUS-FATE", "parent liveness closed — exit");
                        std::process::exit(0);
                    }
                    Ok(_) => {}
                }
            }
        })
        .ok();
}

fn watch_runtime(beat: Arc<std::sync::atomic::AtomicU64>) {
    std::thread::Builder::new()
        .name("ox-watchdog".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let last = beat.load(std::sync::atomic::Ordering::Relaxed);
                if last != 0 && common::ident::now_ms().saturating_sub(last) > 10_000 {
                    tracing::error!(kind = "ev", spec = "RULE-BUS-FATE", stalled_ms = common::ident::now_ms() - last, "runtime hang — exit");
                    std::thread::sleep(Duration::from_millis(200));
                    std::process::exit(70);
                }
            }
        })
        .ok();
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let build = identity(args.build.as_deref());
    if args.version {
        println!("{build}");
        return;
    }
    let policy = match args.policy.as_deref() {
        Some(p) => Policy::load(std::path::Path::new(p)),
        None => Policy::parse(""),
    };
    let policy = match policy {
        Ok(p) => p,
        Err(e) => {
            eprintln!("oxsfud: 정책 파일 — {}", e.0);
            std::process::exit(2);
        }
    };
    let node = args.id.clone().unwrap_or_else(|| "node-a".into());
    common::log::init(common::log::LogConfig {
        node: &node,
        proc_name: "oxsfud",
        logging: &policy.logging,
        log_dir: args.log_dir.as_deref(),
        log_socket: args.log_socket.as_deref(),
    });
    let ip = match resolve_ip(args.public_ip.as_deref()) {
        Ok(ip) => ip,
        Err(e) => {
            eprintln!("oxsfud: {e}");
            std::process::exit(2);
        }
    };
    let udp_port = args.udp_port.unwrap_or(20000);
    let grpc = args.grpc_listen.clone().unwrap_or_else(|| "127.0.0.1:50061".into());
    let udp = match bind_udp(ip, udp_port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("oxsfud: UDP {ip}:{udp_port} — {e}");
            std::process::exit(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("oxsfud: runtime — {e}");
            std::process::exit(2);
        }
    };
    watch_parent();
    rt.block_on(async move {
        let _ = udp.set_nonblocking(true);
        let udp = match tokio::net::UdpSocket::from_std(udp) {
            Ok(u) => Arc::new(u),
            Err(e) => {
                eprintln!("oxsfud: UDP — {e}");
                std::process::exit(2);
            }
        };
        let tcp = match args.tcp_port {
            Some(p) => match tokio::net::TcpListener::bind(SocketAddr::new(ip, p)).await {
                Ok(l) => Some(l),
                Err(_) => match tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), p)).await {
                    Ok(l) => Some(l),
                    Err(e) => {
                        eprintln!("oxsfud: TCP {p} — {e}");
                        std::process::exit(2);
                    }
                },
            },
            None => None,
        };
        let namespace = args.zenoh_namespace.clone().unwrap_or_else(|| "ox".into());
        let sfu = match Sfu::new(node.clone(), build.clone(), policy, &namespace, ip, udp_port, args.tcp_port, udp) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("oxsfud: {e}");
                std::process::exit(2);
            }
        };
        tracing::info!(
            kind = "ev",
            spec = "RULE-CFG-THREE-PLACES",
            epoch = sfu.epoch.as_str(),
            node = node.as_str(),
            build = build.as_str(),
            grpc = grpc.as_str(),
            udp = format!("{ip}:{udp_port}").as_str(),
            tcp = format!("{:?}", args.tcp_port).as_str(),
            zenoh_connect = format!("{:?}", args.zenoh_connect).as_str(),
            namespace = namespace.as_str(),
            auto_layer = format!("{:?}", sfu.policy.media.auto_layer).as_str(),
            max_bitrate_bps = sfu.policy.media.max_bitrate_bps,
            slot_mid_reserve = sfu.policy.media.slot_mid_reserve,
            queue_max = sfu.policy.floor.queue_max,
            "boot"
        );
        watch_runtime(sfu.beat.clone());
        net::spawn_udp(&sfu);
        if let Some(l) = tcp {
            net::spawn_tcp(&sfu, l);
        }
        tick::spawn_all(&sfu);
        bus::spawn(&sfu, args.zenoh_connect.clone());
        if let Err(e) = rpc::serve(sfu.clone(), &grpc).await {
            eprintln!("oxsfud: gRPC {grpc} — {e}");
            std::process::exit(2);
        }
    });
}
