// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CFG-THREE-PLACES · 정:RULE-CFG-INDEX · 정:RULE-FWD-MEDIA-WORKERS · 정:RULE-LINK-TRANSPORT · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! sfud 의 값 — 인자(strict · 인자 > 코드 기본값) · 정책 파일 · `public-ip`(ICE 후보이자 바인드 주소 · 없으면 루프백 · `auto` = 기본 인터페이스) · 워커 수 기본 = 코어 − 2(macOS 는 최대 2).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;

use common::args::Args;
use common::policy::Policy;

pub const ARGS: &[&str] = &[
    "id",
    "build",
    "grpc-listen",
    "udp-port",
    "tcp-port",
    "policy",
    "public-ip",
    "zenoh-namespace",
    "zenoh-connect",
    "log-dir",
    "log-socket",
    "node-grace-secs",
    "media-workers",
    "control-threads",
    "worker-queue-packets",
];

pub const DEFAULT_CONTROL_THREADS: usize = 2;
pub const DEFAULT_GRACE_SECS: u64 = 10;

pub struct SfuCfg {
    pub args: Args,
    pub policy: Policy,
    pub node: String,
    pub build_label: Option<String>,
    pub grpc: SocketAddr,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub ip: IpAddr,
    pub bind_ip: IpAddr,
    pub namespace: String,
    pub connect: String,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub grace_secs: u64,
    pub workers: usize,
    pub control_threads: usize,
    pub queue_packets: usize,
}

pub fn compiled_build() -> &'static str {
    env!("OX_BUILD")
}

fn default_iface() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

fn is_local(ip: IpAddr) -> bool {
    UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
}

pub fn default_workers() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let n = cores.saturating_sub(2).max(1);
    if cfg!(target_os = "macos") { n.min(2) } else { n }
}

impl SfuCfg {
    pub fn load(argv: Vec<String>) -> Result<SfuCfg, String> {
        let args = Args::parse(argv, ARGS, &["version"]).map_err(|e| e.to_string())?;
        let e = |x: common::args::ArgError| x.to_string();
        let node = args.require("id").map_err(e)?.to_string();
        if !common::keyspace::valid_piece(&node) {
            return Err(format!("node_id {node:?} 는 키 조각이 못 된다"));
        }
        let policy_path = args.get("policy").unwrap_or("policy.toml").to_string();
        let policy = Policy::load(std::path::Path::new(&policy_path)).map_err(|x| x.to_string())?;
        let grpc: SocketAddr = args.get("grpc-listen").unwrap_or("127.0.0.1:50061").parse().map_err(|x| format!("--grpc-listen: {x}"))?;
        let udp_port = args.parse_num::<u16>("udp-port").map_err(e)?.unwrap_or(20_000);
        let tcp_port = args.parse_num::<u16>("tcp-port").map_err(e)?;
        let ip = match args.get("public-ip") {
            None => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("auto") => default_iface().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Some(v) => v.parse().map_err(|x| format!("--public-ip {v}: {x}"))?,
        };
        let bind_ip = if is_local(ip) { ip } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) };
        let namespace = args.get("zenoh-namespace").unwrap_or("oxlens").to_string();
        let connect = args.get("zenoh-connect").unwrap_or("tcp/127.0.0.1:7447").to_string();
        let log_dir = args.get("log-dir").map(PathBuf::from);
        let log_socket = args.get("log-socket").map(PathBuf::from);
        let grace_secs = args.parse_num::<u64>("node-grace-secs").map_err(e)?.unwrap_or(DEFAULT_GRACE_SECS);
        let workers = args.parse_num::<usize>("media-workers").map_err(e)?.unwrap_or_else(default_workers).max(1);
        let control_threads = args.parse_num::<usize>("control-threads").map_err(e)?.unwrap_or(DEFAULT_CONTROL_THREADS).max(1);
        let queue_packets = args.parse_num::<usize>("worker-queue-packets").map_err(e)?.unwrap_or(crate::fwd::rt::DEFAULT_QUEUE_PACKETS).max(1);
        let build_label = args.get("build").map(|s| s.to_string());
        Ok(SfuCfg {
            args,
            policy,
            node,
            build_label,
            grpc,
            udp_port,
            tcp_port,
            ip,
            bind_ip,
            namespace,
            connect,
            log_dir,
            log_socket,
            grace_secs,
            workers,
            control_threads,
            queue_packets,
        })
    }

    pub fn build_id(&self) -> String {
        common::ident::build_id(compiled_build(), self.build_label.as_deref())
    }

    pub fn banner_fields(&self) -> Vec<(String, String)> {
        let mut v = self.args.banner_fields();
        for (k, x) in [
            ("public-ip", self.ip.to_string()),
            ("media-workers", self.workers.to_string()),
            ("control-threads", self.control_threads.to_string()),
            ("worker-queue-packets", self.queue_packets.to_string()),
        ] {
            if let Some(e) = v.iter_mut().find(|(n, _)| n == k) {
                e.1 = x;
            }
        }
        v.extend(self.policy.banner_fields_sfu());
        v
    }
}
