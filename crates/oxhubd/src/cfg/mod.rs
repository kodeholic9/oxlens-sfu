// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 운영:RULE-BOOT-ARGS · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! hub 의 값 — 인자(strict) · 시스템 파일 · 정책 파일을 한 번 읽어 최종값으로 · 배너 칸은 출처 이름 그대로.

use std::path::PathBuf;

use common::args::Args;
use common::policy::Policy;
use common::system::System;

pub const ARGS: &[&str] = &["system", "policy", "id", "build", "listen", "log-dir", "log-socket"];

pub struct HubCfg {
    pub args: Args,
    pub system: System,
    pub policy: Policy,
    pub node: String,
    pub listen: String,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub build_label: Option<String>,
}

pub fn compiled_build() -> &'static str {
    env!("OX_BUILD")
}

impl HubCfg {
    pub fn load(argv: Vec<String>) -> Result<HubCfg, String> {
        let args = Args::parse(argv, ARGS, &["version"]).map_err(|e| e.to_string())?;
        let system_path = args.get("system").unwrap_or("system.toml").to_string();
        let policy_path = args.get("policy").unwrap_or("policy.toml").to_string();
        let system = System::load(std::path::Path::new(&system_path)).map_err(|e| e.to_string())?;
        let policy = Policy::load(std::path::Path::new(&policy_path)).map_err(|e| e.to_string())?;
        let node = match args.get("id") {
            Some(n) => n.to_string(),
            None => system.first_sfu_unit().map(|u| u.id.clone()).ok_or_else(|| "--id 없음 · sfu 유닛 없음".to_string())?,
        };
        if !common::keyspace::valid_piece(&node) {
            return Err(format!("node_id {node:?} 는 키 조각이 못 된다"));
        }
        let listen = args.get("listen").map(|s| s.to_string()).unwrap_or_else(|| system.hub.listen.clone());
        let log_dir = args.get("log-dir").map(PathBuf::from).or_else(|| system.hub.log_dir.as_ref().map(PathBuf::from));
        let log_socket = args.get("log-socket").map(PathBuf::from).or_else(|| system.hub.log_socket.as_ref().map(PathBuf::from));
        let build_label = args.get("build").map(|s| s.to_string());
        Ok(HubCfg { args, system, policy, node, listen, log_dir, log_socket, build_label })
    }

    pub fn build_id(&self) -> String {
        common::ident::build_id(compiled_build(), self.build_label.as_deref())
    }

    pub fn banner_fields(&self) -> Vec<(String, String)> {
        let mut v = self.args.banner_fields();
        v.retain(|(k, _)| k != "build");
        v.extend(self.system.banner_fields());
        v.extend(self.policy.banner_fields_hub());
        v
    }

    pub fn child_args(&self) -> Vec<String> {
        let mut a = vec!["--zenoh-namespace".to_string(), self.system.zenoh.namespace.clone()];
        if let Some(l) = self.system.zenoh.listen.first() {
            a.push("--zenoh-connect".into());
            a.push(l.clone());
        }
        if let Some(d) = &self.log_dir {
            a.push("--log-dir".into());
            a.push(d.display().to_string());
        }
        if let Some(s) = &self.log_socket {
            a.push("--log-socket".into());
            a.push(s.display().to_string());
        }
        a.push("--node-grace-secs".into());
        a.push(self.system.supervisor.node_grace_secs.to_string());
        a
    }
}
