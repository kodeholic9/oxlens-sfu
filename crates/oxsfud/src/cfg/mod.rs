// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CFG-THREE-PLACES · 정:RULE-CFG-INDEX · 정:IMPL-ROOM-VERSION-ISSUE · 정:RULE-LINK-TRANSPORT · model: claude-opus-5-5
//! CFG — sfud 기동 값(인자 > 정책 파일 > 기본) · 기동 신원(epoch = sfu_id = inst) · public-ip(`auto` = 기본 인터페이스) · 해석된 최종값 배너 · 프로세스 맥락 하나.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, OnceLock};

use common::args::Args;
use common::policy::Policy;

use crate::bus::Bus;
use crate::link::Links;
use crate::room::Rooms;
use crate::trk::Tracks;

pub const VALUED: [&str; 11] = ["grpc-listen", "udp-port", "tcp-port", "policy", "public-ip", "id", "build", "zenoh-connect", "zenoh-namespace", "log-dir", "log-socket"];
pub const FLAGS: [&str; 1] = ["version"];

pub struct Boot {
    pub node: String,
    pub build: String,
    pub policy: Arc<Policy>,
    pub policy_path: String,
    pub grpc_listen: String,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub public_ip: IpAddr,
    pub zenoh_ns: String,
    pub zenoh_connect: Option<String>,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
    pub epoch: String,
}

fn auto_ip() -> IpAddr {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| {
        s.connect("8.8.8.8:80")?;
        s.local_addr()
    });
    match probe {
        Ok(a) if !a.ip().is_unspecified() => a.ip(),
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
}

fn port(args: &Args, k: &str, d: Option<u16>) -> Result<Option<u16>, String> {
    match args.get(k) {
        Some(v) => v.parse::<u16>().map(Some).map_err(|_| format!("--{k} {v}")),
        None => Ok(d),
    }
}

impl Boot {
    pub fn load(args: &Args, raw_build: &str) -> Result<Boot, String> {
        let policy_path = args.get("policy").unwrap_or("policy.toml").to_string();
        let policy = match std::fs::read_to_string(&policy_path) {
            Ok(t) => Policy::parse(&t).map_err(|e| e.to_string())?,
            Err(e) if args.get("policy").is_some() => return Err(format!("policy {policy_path}: {e}")),
            Err(_) => Policy::default(),
        };
        let public_ip = match args.get("public-ip") {
            None => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("auto") => auto_ip(),
            Some(v) => v.parse::<IpAddr>().map_err(|_| format!("--public-ip {v}"))?,
        };
        Ok(Boot {
            node: args.get("id").unwrap_or("node").to_string(),
            build: common::ident::build_identity(raw_build, args.get("build")),
            policy: Arc::new(policy),
            policy_path,
            grpc_listen: args.get("grpc-listen").unwrap_or("127.0.0.1:50061").to_string(),
            udp_port: port(args, "udp-port", Some(20000))?.unwrap_or(20000),
            tcp_port: port(args, "tcp-port", None)?,
            public_ip,
            zenoh_ns: args.get("zenoh-namespace").unwrap_or("ox").to_string(),
            zenoh_connect: args.get("zenoh-connect").map(str::to_string),
            log_dir: args.get("log-dir").map(str::to_string),
            log_socket: args.get("log-socket").map(str::to_string),
            epoch: common::ident::new_epoch("sfu"),
        })
    }

    pub fn banner(&self) -> String {
        format!(
            "node={} epoch={} build={} grpc_listen={} public_ip={} udp_port={} tcp_port={:?} zenoh.namespace={} zenoh.connect={:?} log_dir={:?} log_socket={:?} policy={} media.max_bitrate_bps={} media.slot_mid_reserve={} media.auto_layer={:?} media.bwe_mode={:?} floor.queue_max={} floor.t2_stop_talking_secs={} logging.level={:?}",
            self.node,
            self.epoch,
            self.build,
            self.grpc_listen,
            self.public_ip,
            self.udp_port,
            self.tcp_port,
            self.zenoh_ns,
            self.zenoh_connect,
            self.log_dir,
            self.log_socket,
            self.policy_path,
            self.policy.media.max_bitrate_bps,
            self.policy.media.slot_mid_reserve,
            self.policy.media.auto_layer,
            self.policy.media.bwe_mode,
            self.policy.floor.queue_max,
            self.policy.floor.t2_stop_talking_secs,
            self.policy.logging.level,
        )
    }
}

pub struct Ctx {
    pub boot: Boot,
    pub rooms: Rooms,
    pub links: Arc<Links>,
    pub tracks: Tracks,
    pub bus: Arc<Bus>,
    pub drops: crate::ops::Drops,
    pub task_seq: AtomicU32,
}

static CTX: OnceLock<Arc<Ctx>> = OnceLock::new();

pub fn install(c: Arc<Ctx>) {
    let _ = CTX.set(c);
}

pub fn get() -> Option<&'static Arc<Ctx>> {
    CTX.get()
}

impl Ctx {
    pub fn v2(&self) -> bool {
        format!("{:?}", self.boot.policy.media.auto_layer).eq_ignore_ascii_case("v2")
    }

    pub fn auto(&self) -> bool {
        !format!("{:?}", self.boot.policy.media.auto_layer).eq_ignore_ascii_case("off")
    }
}
