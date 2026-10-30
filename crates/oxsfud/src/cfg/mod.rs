// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 정:CAND-CFG-SFUD-DEFAULT-PORTS · 정:CAND-CFG-SFUD-NODE-GRACE · 정:RULE-BUS-KEYSPACE · 정:CAND-BUS-NO-ZENOH-BOOT · 정:RULE-FWD-MEDIA-WORKERS · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! sfud 값의 자리 — 기동 인자(포트 · 거두기 유예 · 버스 좌표는 기본값이 없다 — 없으면 기동 실패) · 정책 파일. `public-ip` 는 ICE 후보이자 UDP 바인드 주소(`auto` = 기본 인터페이스 · 없으면 루프백). 배너 한 줄 = 해석된 최종값 전량.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use common::args::{Args, Spec, flag, opt};
use common::banner::Banner;
use common::policy::{Policy, Reader};

pub const ARGS: &[Spec] = &[
    opt("grpc-listen"),
    opt("udp-port"),
    opt("tcp-port"),
    opt("policy"),
    opt("public-ip"),
    opt("id"),
    opt("build"),
    opt("zenoh-namespace"),
    opt("zenoh-connect"),
    opt("log-dir"),
    opt("log-socket"),
    opt("node-grace-secs"),
    opt("media-workers"),
    opt("control-threads"),
    opt("worker-queue-packets"),
    flag("version"),
];

pub const CONTROL_THREADS: usize = 2;
pub const WORKER_QUEUE_PACKETS: usize = 6_000;

#[derive(Debug, Clone)]
pub struct SfuCfg {
    pub grpc_listen: String,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub policy: Policy,
    pub public_ip: IpAddr,
    pub bind_ip: IpAddr,
    pub node_id: String,
    pub namespace: String,
    pub zenoh_connect: String,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
    pub node_grace_secs: u64,
    pub media_workers: usize,
    pub control_threads: usize,
    pub worker_queue_packets: usize,
    pub build: String,
    pub args: Args,
}

pub const MACOS_MEDIA_WORKERS: usize = 2;

pub fn default_media_workers() -> usize {
    let by_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).saturating_sub(2).max(1);
    if cfg!(target_os = "macos") { by_cores.min(MACOS_MEDIA_WORKERS) } else { by_cores }
}

fn default_interface() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

fn is_local(ip: IpAddr) -> bool {
    UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
}

impl SfuCfg {
    pub fn load(args: Args) -> Result<SfuCfg, String> {
        let grpc_listen = args.require("grpc-listen")?.to_string();
        let udp_port: u16 = args.parse_num("udp-port")?.ok_or("--udp-port 가 없다(기본값 없음)")?;
        let tcp_port: Option<u16> = args.parse_num("tcp-port")?;
        let policy = match args.get("policy") {
            Some(p) => Policy::load(p).map_err(|e| e.to_string())?,
            None => Policy::default(),
        };
        let public_ip = match args.get("public-ip") {
            None => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("auto") => default_interface().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Some(v) => v.parse().map_err(|_| format!("--public-ip 꼴: {v}"))?,
        };
        let bind_ip = if is_local(public_ip) { public_ip } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) };
        let node_id = args.require("id")?.to_string();
        common::bus::keys::check_fragment("--id", &node_id)?;
        let namespace = args.require("zenoh-namespace")?.to_string();
        common::bus::keys::check_fragment("--zenoh-namespace", &namespace)?;
        let zenoh_connect = args.require("zenoh-connect")?.to_string();
        let node_grace_secs: u64 = args.parse_num("node-grace-secs")?.ok_or("--node-grace-secs 가 없다(기본값 없음)")?;
        let media_workers: usize = args.parse_num::<usize>("media-workers")?.unwrap_or_else(default_media_workers).max(1);
        let control_threads: usize = args.parse_num::<usize>("control-threads")?.unwrap_or(CONTROL_THREADS).max(1);
        let worker_queue_packets: usize = args.parse_num::<usize>("worker-queue-packets")?.unwrap_or(WORKER_QUEUE_PACKETS).max(1);
        let build = common::build::identity(args.get("build"));
        Ok(SfuCfg {
            grpc_listen,
            udp_port,
            tcp_port,
            policy,
            public_ip,
            bind_ip,
            node_id,
            namespace,
            zenoh_connect,
            log_dir: args.get("log-dir").map(str::to_string),
            log_socket: args.get("log-socket").map(str::to_string),
            node_grace_secs,
            media_workers,
            control_threads,
            worker_queue_packets,
            build,
            args,
        })
    }

    pub fn banner(&self) -> Banner {
        let mut b = Banner::new();
        for name in ["grpc-listen", "udp-port", "tcp-port", "policy"] {
            b.put_opt(name, self.args.get(name));
        }
        b.put("public-ip", self.public_ip);
        for name in ["id", "build", "zenoh-namespace", "zenoh-connect", "log-dir", "log-socket", "node-grace-secs"] {
            b.put_opt(name, self.args.get(name));
        }
        b.put("media-workers", self.media_workers);
        b.put("control-threads", self.control_threads);
        b.put("worker-queue-packets", self.worker_queue_packets);
        self.policy.banner(&mut b, Reader::Sfud);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ports_and_grace_have_no_default() {
        let a = Args::parse(&argv(&["--grpc-listen", "127.0.0.1:1", "--id", "n", "--zenoh-namespace", "c", "--zenoh-connect", "tcp/127.0.0.1:7447", "--node-grace-secs", "30"]), ARGS).expect("args");
        assert!(SfuCfg::load(a).is_err());
        let a = Args::parse(&argv(&["--grpc-listen", "127.0.0.1:1", "--udp-port", "20000", "--id", "n", "--zenoh-namespace", "c", "--zenoh-connect", "tcp/127.0.0.1:7447"]), ARGS).expect("args");
        assert!(SfuCfg::load(a).is_err());
        let a = Args::parse(&argv(&["--grpc-listen", "127.0.0.1:1", "--udp-port", "20000", "--id", "n*", "--zenoh-namespace", "c", "--zenoh-connect", "x", "--node-grace-secs", "30"]), ARGS).expect("args");
        assert!(SfuCfg::load(a).is_err());
        let a = Args::parse(&argv(&["--grpc-listen", "127.0.0.1:1", "--udp-port", "20000", "--id", "n", "--zenoh-namespace", "c", "--zenoh-connect", "x", "--node-grace-secs", "30"]), ARGS).expect("args");
        let c = SfuCfg::load(a).expect("ok");
        assert_eq!(c.public_ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(c.node_grace_secs, 30);
    }
}
