// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-CFG-THREE-PLACES 정:IMPL-CFG-SYSTEM-FILE 정:CAND-CFG-SFUD-DEFAULT-PORTS 정:RULE-BUS-SHAPE 정:CAND-BUS-NO-ZENOH-BOOT 정:RULE-LINK-TRANSPORT 운영:RULE-BOOT-ARGS 정:CAND-CFG-SFUD-NODE-GRACE · model: claude-opus-5-5
//! sfud 기동 인자 · 정책 파일 — 포트는 기본값이 없다 · 버스 좌표가 없으면 기동 실패 · public-ip `auto` 는 기본 인터페이스 주소.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use common::policy::Policy;

#[derive(Debug, Clone, Default)]
pub struct Args {
    pub policy: Option<String>,
    pub id: Option<String>,
    pub build: Option<String>,
    pub grpc_listen: Option<String>,
    pub udp_port: Option<u16>,
    pub tcp_port: Option<u16>,
    pub public_ip: Option<String>,
    pub zenoh_namespace: Option<String>,
    pub zenoh_connect: Option<String>,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
}

pub enum Parsed {
    Version(Option<String>),
    Run(Args),
}

pub fn parse_args(argv: &[String]) -> Result<Parsed, String> {
    let mut a = Args::default();
    let mut version = false;
    let mut i = 0;
    while i < argv.len() {
        let k = argv[i].as_str();
        let mut v = || -> Result<String, String> {
            i += 1;
            argv.get(i).cloned().ok_or_else(|| format!("{k} 값이 없다"))
        };
        match k {
            "--version" => version = true,
            "--policy" => a.policy = Some(v()?),
            "--id" => a.id = Some(v()?),
            "--build" => a.build = Some(v()?),
            "--grpc-listen" => a.grpc_listen = Some(v()?),
            "--udp-port" => a.udp_port = Some(v()?.parse().map_err(|_| "--udp-port 가 수가 아니다".to_string())?),
            "--tcp-port" => a.tcp_port = Some(v()?.parse().map_err(|_| "--tcp-port 가 수가 아니다".to_string())?),
            "--public-ip" => a.public_ip = Some(v()?),
            "--zenoh-namespace" => a.zenoh_namespace = Some(v()?),
            "--zenoh-connect" => a.zenoh_connect = Some(v()?),
            "--log-dir" => a.log_dir = Some(v()?),
            "--log-socket" => a.log_socket = Some(v()?),
            other => return Err(format!("모르는 인자 {other}")),
        }
        i += 1;
    }
    if version {
        return Ok(Parsed::Version(a.build));
    }
    Ok(Parsed::Run(a))
}

#[derive(Debug, Clone)]
pub struct Config {
    pub policy: Policy,
    pub node_id: String,
    pub build: String,
    pub grpc_listen: String,
    pub udp_port: u16,
    pub tcp_port: Option<u16>,
    pub public_ip: IpAddr,
    pub bind_ip: IpAddr,
    pub zenoh_namespace: String,
    pub zenoh_connect: String,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
    pub node_grace_secs: u64,
}

pub const NODE_GRACE_SECS: u64 = 30;

fn default_iface() -> IpAddr {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|s| {
        s.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
        s.local_addr()
    });
    match probe {
        Ok(a) if !a.ip().is_unspecified() => a.ip(),
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
}

fn is_local(ip: IpAddr) -> bool {
    UdpSocket::bind((ip, 0)).is_ok()
}

pub fn load(a: &Args) -> Result<Config, String> {
    let policy = match &a.policy {
        Some(p) => Policy::parse(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| e.to_string())?,
        None => Policy::default(),
    };
    let grpc_listen = a.grpc_listen.clone().ok_or("--grpc-listen 이 없다(기본값 없음)")?;
    let udp_port = a.udp_port.ok_or("--udp-port 가 없다(기본값 없음)")?;
    let zenoh_connect = a.zenoh_connect.clone().ok_or("--zenoh-connect 가 없다 — 버스 없이는 기동하지 않는다")?;
    let public_ip = match a.public_ip.as_deref() {
        None => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some("auto") => default_iface(),
        Some(s) => s.parse::<IpAddr>().map_err(|_| format!("--public-ip {s} 형식"))?,
    };
    let bind_ip = if is_local(public_ip) { public_ip } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) };
    Ok(Config {
        build: common::build_id(a.build.as_deref()),
        node_id: a.id.clone().unwrap_or_else(|| "local".into()),
        grpc_listen,
        udp_port,
        tcp_port: a.tcp_port,
        public_ip,
        bind_ip,
        zenoh_namespace: a.zenoh_namespace.clone().unwrap_or_else(|| "ox".into()),
        zenoh_connect,
        log_dir: a.log_dir.clone(),
        log_socket: a.log_socket.clone(),
        node_grace_secs: NODE_GRACE_SECS,
        policy,
    })
}

impl Config {
    pub fn banner(&self, epoch: &str) -> String {
        let p = &self.policy;
        format!(
            "sfud boot node={} epoch={epoch} grpc={} udp={} tcp={} public_ip={} bind_ip={} zenoh.connect={} zenoh.ns={} log_dir={} log_socket={} node_grace_s={} bwe_mode={:?} auto_layer={:?} slot_mid_reserve={} max_bitrate_bps={} floor.queue_max={} floor.t2_s={} stat_interval_ms={}",
            self.node_id,
            self.grpc_listen,
            self.udp_port,
            self.tcp_port.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            self.public_ip,
            self.bind_ip,
            self.zenoh_connect,
            self.zenoh_namespace,
            self.log_dir.as_deref().unwrap_or("-"),
            self.log_socket.as_deref().unwrap_or("-"),
            self.node_grace_secs,
            p.media.bwe_mode,
            p.media.auto_layer,
            p.media.slot_mid_reserve,
            p.media.max_bitrate_bps,
            p.floor.queue_max,
            p.floor.t2_stop_talking_secs,
            p.logging.stat_interval_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn ports_have_no_default() {
        let Ok(Parsed::Run(a)) = parse_args(&argv(&["--zenoh-connect", "tcp/127.0.0.1:7447"])) else { panic!("parse") };
        assert!(load(&a).is_err(), "정:CAND-CFG-SFUD-DEFAULT-PORTS");
    }

    #[test]
    fn bus_is_required() {
        let Ok(Parsed::Run(a)) = parse_args(&argv(&["--grpc-listen", "127.0.0.1:1", "--udp-port", "1"])) else { panic!("parse") };
        assert!(load(&a).is_err(), "정:CAND-BUS-NO-ZENOH-BOOT");
    }

    #[test]
    fn unknown_arg_fails() {
        assert!(parse_args(&argv(&["--nope"])).is_err());
    }
}
