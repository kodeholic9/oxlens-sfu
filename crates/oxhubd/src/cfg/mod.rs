// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-CFG-THREE-PLACES 정:IMPL-CFG-SYSTEM-FILE 정:RULE-CFG-INDEX 운영:RULE-BOOT-ARGS 운영:RULE-BOOT-PORTS 운영:RULE-BOOT-SHAPE · model: claude-opus-5-5
//! hub 기동 인자 · 시스템 파일 · 정책 파일 — 부팅 1회 로드 · 인자가 파일을 덮는다 · 해석된 최종값은 배너 한 줄로.

use common::policy::Policy;
use common::system::System;

#[derive(Debug, Clone)]
pub struct Args {
    pub system: String,
    pub policy: Option<String>,
    pub id: Option<String>,
    pub build: Option<String>,
    pub listen: Option<String>,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
}

pub enum Parsed {
    Version(Option<String>),
    Run(Args),
}

pub fn parse_args(argv: &[String]) -> Result<Parsed, String> {
    let mut a = Args { system: String::new(), policy: None, id: None, build: None, listen: None, log_dir: None, log_socket: None };
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
            "--system" => a.system = v()?,
            "--policy" => a.policy = Some(v()?),
            "--id" => a.id = Some(v()?),
            "--build" => a.build = Some(v()?),
            "--listen" => a.listen = Some(v()?),
            "--log-dir" => a.log_dir = Some(v()?),
            "--log-socket" => a.log_socket = Some(v()?),
            other => return Err(format!("모르는 인자 {other}")),
        }
        i += 1;
    }
    if version {
        return Ok(Parsed::Version(a.build));
    }
    if a.system.is_empty() {
        return Err("--system 이 없다".into());
    }
    Ok(Parsed::Run(a))
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sys: System,
    pub policy: Policy,
    pub node_id: String,
    pub build: String,
    pub listen: String,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
}

pub fn load(a: &Args) -> Result<Config, String> {
    let text = std::fs::read_to_string(&a.system).map_err(|e| format!("{}: {e}", a.system))?;
    let sys = System::parse(&text).map_err(|e| e.to_string())?;
    let policy = match &a.policy {
        Some(p) => Policy::parse(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| e.to_string())?,
        None => Policy::default(),
    };
    let node_id = a.id.clone().or_else(|| sys.first_sfu_id().map(str::to_string)).unwrap_or_else(|| "local".to_string());
    let listen = a.listen.clone().unwrap_or_else(|| sys.hub.listen.clone());
    let log_dir = a.log_dir.clone().or_else(|| sys.hub.log_dir.clone());
    let log_socket = a.log_socket.clone().or_else(|| sys.hub.log_socket.clone());
    Ok(Config { build: common::build_id(a.build.as_deref()), sys, policy, node_id, listen, log_dir, log_socket })
}

impl Config {
    pub fn banner(&self) -> String {
        let s = &self.sys;
        let units: Vec<String> = s.unit.iter().map(|u| format!("{}:{}:{}:{}", u.id, u.kind.as_str(), u.order, if u.enabled { "on" } else { "off" })).collect();
        format!(
            "hub boot node={} listen={} base_path={} tls={} log_dir={} log_socket={} accounts={} units=[{}] zenoh.mode={} zenoh.listen={:?} zenoh.connect={:?} zenoh.ns={} zenoh.multicast={} zenoh.base_config={} supervisor.backoff_ms={:?} intensity={}/{}s hello_ms={} node_grace_s={} heartbeat_ms={} resume_window_ms={} ws_flow_window={} token_ttl_s={} capacity={} unused_ttl_s={} departure_ttl_s={} allowed_origins={:?} max_sessions={} max_rooms={} join_admit_per_sec={} hooks={}",
            self.node_id,
            self.listen,
            s.hub.base_path,
            s.hub.tls.is_some(),
            self.log_dir.as_deref().unwrap_or("-"),
            self.log_socket.as_deref().unwrap_or("-"),
            s.hub.auth.api_keys.len(),
            units.join(","),
            s.zenoh.mode,
            s.zenoh.listen,
            s.zenoh.connect,
            s.zenoh.namespace,
            s.zenoh.multicast_scouting,
            s.zenoh.base_config.as_deref().unwrap_or("-"),
            s.supervisor.backoff_ms,
            s.supervisor.intensity_max,
            s.supervisor.intensity_window_secs,
            s.supervisor.hello_timeout_ms,
            s.supervisor.node_grace_secs,
            self.policy.hub.heartbeat_interval_ms,
            self.policy.hub.resume_window_ms,
            self.policy.hub.ws_flow_window,
            self.policy.hub.token_ttl_secs,
            self.policy.hub.room_default_capacity,
            self.policy.hub.room_unused_ttl_secs,
            self.policy.hub.room_departure_ttl_secs,
            self.policy.hub.allowed_origins,
            self.policy.quota.max_sessions,
            self.policy.quota.max_rooms,
            self.policy.hub.join_admit_per_sec,
            s.hook.as_ref().map(|h| h.webhooks.len()).unwrap_or(0)
        )
    }
}
