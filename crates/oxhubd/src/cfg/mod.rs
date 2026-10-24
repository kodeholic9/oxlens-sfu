// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-INDEX · 운영 규격서 §8-2 · model: claude-opus-5-5
//! 기동 값 — 인자 > 시스템 파일 > 코드 기본값. 부팅 1회 로드 · 해석된 최종값 전량을 배너 한 줄로.

use std::path::PathBuf;

use common::args::Args;
use common::policy::Policy;
use common::system::SystemFile;

pub const VALUED: &[&str] = &["system", "policy", "id", "build", "listen", "log-dir", "log-socket"];
pub const SWITCHES: &[&str] = &["version"];

#[derive(Debug, Clone)]
pub struct Boot {
    pub args: Args,
    pub system: SystemFile,
    pub policy: Policy,
    pub node_id: String,
    pub listen: String,
    pub base_path: String,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub build: String,
    pub label: Option<String>,
}

impl Boot {
    pub fn from_args(argv: &[String]) -> Result<Boot, String> {
        let args = Args::parse(argv, VALUED, SWITCHES)?;
        let label = args.get("build").map(str::to_string);
        let build = common::identity::build(label.as_deref());
        if args.has("version") {
            return Err(format!("version:{build}"));
        }
        let sys_path = PathBuf::from(args.get("system").unwrap_or("system.toml"));
        let system = SystemFile::load(&sys_path).map_err(|e| e.to_string())?;
        let policy_path = args.get("policy").map(PathBuf::from);
        let policy = match &policy_path {
            Some(p) => Policy::load(p).map_err(|e| e.to_string())?,
            None => Policy::default(),
        };
        let node_id = match args.get("id") {
            Some(i) => i.to_string(),
            None => system.first_sfu().map(|u| u.id.clone()).ok_or("--id 가 없고 sfu 유닛도 없다")?,
        };
        if !common::bus::keys::chunk_ok(&node_id) {
            return Err(format!("node_id {node_id:?} 는 키 조각이 될 수 없다"));
        }
        let listen = args.get("listen").map(str::to_string).unwrap_or_else(|| system.hub.listen.clone());
        let log_dir = args.get("log-dir").map(PathBuf::from).or_else(|| system.hub.log_dir.as_ref().map(PathBuf::from));
        let log_socket = args.get("log-socket").map(PathBuf::from).or_else(|| system.hub.log_socket.as_ref().map(PathBuf::from));
        let base_path = system.hub.base_path.trim_end_matches('/').to_string();
        Ok(Boot { args, policy, node_id, listen, base_path, log_dir, log_socket, build, label, system })
    }

    pub fn banner(&self) -> String {
        let p = &self.policy;
        format!(
            "node={} listen={} base_path={} tls={} log_dir={} log_socket={} units={} zenoh.listen={:?} zenoh.connect={:?} zenoh.namespace={} \
             hub.heartbeat_interval_ms={} hub.heartbeat_timeout_ms={} hub.resume_window_ms={} hub.ws_flow_window={} hub.token_ttl_secs={} \
             hub.room_unused_ttl_secs={} hub.room_departure_ttl_secs={} hub.room_default_capacity={} hub.join_admit_per_sec={} quota.max_sessions={} quota.max_rooms={} \
             logging.level={} media.auto_layer={:?} args=[{}]",
            self.node_id,
            self.listen,
            self.base_path,
            self.system.hub.tls.is_some(),
            self.log_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "-".into()),
            self.log_socket.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "-".into()),
            self.system.unit.iter().map(|u| u.id.as_str()).collect::<Vec<_>>().join(","),
            self.system.zenoh.listen,
            self.system.zenoh.connect,
            self.system.zenoh.namespace,
            p.hub.heartbeat_interval_ms,
            p.hub.heartbeat_timeout_ms,
            p.hub.resume_window_ms,
            p.hub.ws_flow_window,
            p.hub.token_ttl_secs,
            p.hub.room_unused_ttl_secs,
            p.hub.room_departure_ttl_secs,
            p.hub.room_default_capacity,
            p.hub.join_admit_per_sec,
            p.quota.max_sessions,
            p.quota.max_rooms,
            p.logging.level.as_str(),
            p.media.auto_layer,
            self.args.banner(),
        )
    }
}
