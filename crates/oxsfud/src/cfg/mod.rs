// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CFG-THREE-PLACES · 정:RULE-CFG-INDEX · 정:CAND-BUS-NO-ZENOH-BOOT · 운영 규격서 §8-1 · 운영 규격서 §8-2 · model: claude-opus-5-5
//! 기동 값 — 인자 > 정책 파일 > 코드 기본값. 포트에 기본값이 없다(없으면 기동 실패) · `--public-ip auto` 는 기본 경로의 출발 주소 · 해석된 최종값을 배너 한 줄로.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;

use common::args::Args;
use common::policy::Policy;

pub const VALUED: &[&str] = &["grpc-listen", "udp-port", "tcp-port", "policy", "public-ip", "id", "build", "zenoh-namespace", "zenoh-connect", "log-dir", "log-socket"];
pub const SWITCHES: &[&str] = &["version"];

#[derive(Debug, Clone)]
pub struct Boot {
    pub args: Args,
    pub policy: Policy,
    pub node_id: String,
    pub grpc_listen: SocketAddr,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub public_ip: IpAddr,
    pub bind_ip: IpAddr,
    pub namespace: String,
    pub zenoh_connect: String,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub build: String,
}

fn local_ip() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .map(|a| a.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

fn is_local(ip: IpAddr) -> bool {
    UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
}

impl Boot {
    pub fn from_args(argv: &[String]) -> Result<Boot, String> {
        let args = Args::parse(argv, VALUED, SWITCHES)?;
        let build = common::identity::build(args.get("build"));
        if args.has("version") {
            return Err(format!("version:{build}"));
        }
        let policy = match args.get("policy") {
            Some(p) => Policy::load(&PathBuf::from(p)).map_err(|e| e.to_string())?,
            None => Policy::default(),
        };
        let node_id = args.require("id")?.to_string();
        if !common::bus::keys::chunk_ok(&node_id) {
            return Err(format!("node_id {node_id:?} 는 키 조각이 될 수 없다"));
        }
        let grpc_listen: SocketAddr = args.require("grpc-listen")?.parse().map_err(|e| format!("--grpc-listen: {e}"))?;
        let udp_port: u16 = args.require("udp-port")?.parse().map_err(|e| format!("--udp-port: {e}"))?;
        let tcp_port: Option<u16> = match args.get("tcp-port") {
            Some(v) => Some(v.parse().map_err(|e| format!("--tcp-port: {e}"))?),
            None => None,
        };
        let public_ip = match args.get("public-ip") {
            None | Some("auto") => local_ip(),
            Some(v) => v.parse().map_err(|e| format!("--public-ip: {e}"))?,
        };
        let bind_ip = if is_local(public_ip) { public_ip } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) };
        let zenoh_connect = args.require("zenoh-connect")?.to_string();
        let namespace = args.get("zenoh-namespace").unwrap_or("oxlens").to_string();
        Ok(Boot {
            policy,
            node_id,
            grpc_listen,
            udp_port,
            tcp_port,
            public_ip,
            bind_ip,
            namespace,
            zenoh_connect,
            log_dir: args.get("log-dir").map(PathBuf::from),
            log_socket: args.get("log-socket").map(PathBuf::from),
            build,
            args,
        })
    }

    pub fn banner(&self, epoch: &str) -> String {
        let p = &self.policy;
        format!(
            "node={} epoch={} grpc={} udp={}:{} tcp={} advertise={} zenoh.connect={} zenoh.namespace={} media.auto_layer={:?} media.bwe_mode={:?} media.max_bitrate_bps={} media.slot_mid_reserve={} floor.queue_max={} floor.t2_stop_talking_secs={} hub.room_unused_ttl_secs={} hub.room_departure_ttl_secs={} logging.level={} args=[{}]",
            self.node_id,
            epoch,
            self.grpc_listen,
            self.bind_ip,
            self.udp_port,
            self.tcp_port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            self.public_ip,
            self.zenoh_connect,
            self.namespace,
            p.media.auto_layer,
            p.media.bwe_mode,
            p.media.max_bitrate_bps,
            p.media.slot_mid_reserve,
            p.floor.queue_max,
            p.floor.t2_stop_talking_secs,
            p.hub.room_unused_ttl_secs,
            p.hub.room_departure_ttl_secs,
            p.logging.level.as_str(),
            self.args.banner(),
        )
    }
}
