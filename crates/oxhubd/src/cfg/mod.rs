// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-INDEX · model: claude-opus-5-5
//! CFG — hub 기동 값(인자 > 시스템 파일 > 기본) · 부팅 1회 로드 · 해석된 최종값 전량을 배너 한 줄로.

use std::sync::Arc;

use common::args::Args;
use common::policy::Policy;
use common::system::SystemFile;

pub const VALUED: [&str; 7] = ["system", "policy", "id", "build", "listen", "log-dir", "log-socket"];
pub const FLAGS: [&str; 1] = ["version"];

pub struct Boot {
    pub sys: Arc<SystemFile>,
    pub policy: Arc<Policy>,
    pub policy_path: String,
    pub node: String,
    pub build: String,
    pub label: Option<String>,
    pub listen: String,
    pub log_dir: Option<String>,
    pub log_socket: Option<String>,
}

impl Boot {
    pub fn load(args: &Args, raw_build: &str) -> Result<Boot, String> {
        let sys_path = args.get("system").unwrap_or("system.toml").to_string();
        let sys_text = std::fs::read_to_string(&sys_path).map_err(|e| format!("system {sys_path}: {e}"))?;
        let sys = SystemFile::parse(&sys_text)?;
        let policy_path = args.get("policy").unwrap_or("policy.toml").to_string();
        let policy = match std::fs::read_to_string(&policy_path) {
            Ok(t) => Policy::parse(&t).map_err(|e| e.to_string())?,
            Err(e) if args.get("policy").is_some() => return Err(format!("policy {policy_path}: {e}")),
            Err(_) => Policy::default(),
        };
        let node = args.get("id").map(str::to_string).or_else(|| sys.first_sfu().map(|u| u.id.clone())).unwrap_or_else(|| "node".to_string());
        let label = args.get("build").map(str::to_string);
        let build = common::ident::build_identity(raw_build, label.as_deref());
        let listen = args.get("listen").map(str::to_string).unwrap_or_else(|| sys.hub.listen.clone());
        let log_dir = args.get("log-dir").map(str::to_string).or_else(|| sys.hub.log_dir.clone());
        let log_socket = args.get("log-socket").map(str::to_string).or_else(|| sys.hub.log_socket.clone());
        Ok(Boot { sys: Arc::new(sys), policy: Arc::new(policy), policy_path, node, build, label, listen, log_dir, log_socket })
    }

    pub fn child_args(&self) -> Vec<String> {
        let mut v = vec!["--id".to_string(), self.node.clone()];
        if let Some(l) = &self.label {
            v.push("--build".to_string());
            v.push(l.clone());
        }
        v.push("--zenoh-namespace".to_string());
        v.push(self.sys.zenoh.namespace.clone());
        if let Some(c) = self.sys.zenoh.listen.first() {
            v.push("--zenoh-connect".to_string());
            v.push(c.clone());
        }
        if let Some(d) = &self.log_dir {
            v.push("--log-dir".to_string());
            v.push(d.clone());
        }
        if let Some(s) = &self.log_socket {
            v.push("--log-socket".to_string());
            v.push(s.clone());
        }
        v
    }

    pub fn banner(&self) -> String {
        format!(
            "node={} build={} listen={} base_path={} zenoh.mode={} zenoh.listen={:?} zenoh.connect={:?} namespace={} units={} log_dir={:?} log_socket={:?} policy={} hub.ws_flow_window={} hub.resume_window_ms={} hub.heartbeat_interval_ms={} media.auto_layer={:?}",
            self.node,
            self.build,
            self.listen,
            self.sys.hub.base_path,
            self.sys.zenoh.mode,
            self.sys.zenoh.listen,
            self.sys.zenoh.connect,
            self.sys.zenoh.namespace,
            self.sys.unit.iter().map(|u| u.id.as_str()).collect::<Vec<_>>().join(","),
            self.log_dir,
            self.log_socket,
            self.policy_path,
            self.policy.hub.ws_flow_window,
            self.policy.hub.resume_window_ms,
            self.policy.hub.heartbeat_interval_ms,
            self.policy.media.auto_layer,
        )
    }
}
