// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · 정:CAND-OPS-SUPERVISOR-DEFAULTS · 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량을 strict 로 읽는다. 모르는 키 · 버스 전제 위반은 기동 실패.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemError(pub String);

impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "system: {}", self.0)
    }
}

impl std::error::Error for SystemError {}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFile {
    pub hub: Hub,
    #[serde(default)]
    pub unit: Vec<Unit>,
    pub zenoh: Zenoh,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default)]
    pub hook: Option<Hook>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hub {
    pub listen: String,
    #[serde(default = "default_base_path")]
    pub base_path: String,
    #[serde(default)]
    pub log_dir: Option<String>,
    #[serde(default)]
    pub log_socket: Option<String>,
    #[serde(default)]
    pub tls: Option<Tls>,
    #[serde(default)]
    pub auth: Auth,
}

fn default_base_path() -> String {
    "/media".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    #[serde(default)]
    pub jwt_secret: Option<String>,
    #[serde(default)]
    pub api_keys: Vec<ApiKey>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_types")]
    pub participant_types: Vec<u8>,
    #[serde(default)]
    pub hidden_allowed: bool,
    #[serde(default)]
    pub ops_allowed: bool,
    #[serde(default)]
    pub hook_allowed: bool,
}

fn default_types() -> Vec<u8> {
    vec![0]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    Process,
    Remote,
    Inproc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitRole {
    Sfu,
    Log,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    #[serde(default)]
    pub order: u32,
    #[serde(default = "default_role")]
    pub role: UnitRole,
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_role() -> UnitRole {
    UnitRole::Sfu
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zenoh {
    pub mode: String,
    #[serde(default)]
    pub listen: Vec<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub multicast_scouting: bool,
    #[serde(default = "default_true")]
    pub qos_enabled: bool,
    #[serde(default)]
    pub lowlatency: bool,
    pub namespace: String,
    #[serde(default)]
    pub base_config: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Supervisor {
    pub backoff_ms: Vec<u64>,
    pub intensity_max: u32,
    pub intensity_window_secs: u64,
    pub hello_timeout_ms: u64,
    pub node_grace_secs: u64,
}

impl Default for Supervisor {
    fn default() -> Self {
        Supervisor { backoff_ms: vec![200, 500, 1000, 2000, 5000], intensity_max: 5, intensity_window_secs: 60, hello_timeout_ms: 10_000, node_grace_secs: 30 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    #[serde(default)]
    pub webhooks: Vec<Webhook>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    pub url: String,
    pub secret: String,
}

impl SystemFile {
    pub fn parse(text: &str) -> Result<SystemFile, SystemError> {
        let s: SystemFile = toml::from_str(text).map_err(|e| SystemError(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    pub fn load(path: &std::path::Path) -> Result<SystemFile, SystemError> {
        let text = std::fs::read_to_string(path).map_err(|e| SystemError(format!("{}: {e}", path.display())))?;
        SystemFile::parse(&text)
    }

    pub fn validate(&self) -> Result<(), SystemError> {
        if self.zenoh.mode != "router" {
            return Err(SystemError(format!("zenoh.mode {} — router 고정", self.zenoh.mode)));
        }
        if !self.zenoh.qos_enabled {
            return Err(SystemError("zenoh.qos_enabled = false — true 고정(레인 분리 전제)".into()));
        }
        if self.zenoh.lowlatency {
            return Err(SystemError("zenoh.lowlatency = true — false 고정(레인 분리 전제)".into()));
        }
        if self.zenoh.namespace.is_empty() || self.zenoh.namespace.contains(['/', '*', '$', '?', '#']) {
            return Err(SystemError(format!("zenoh.namespace {:?} — 키 조각이 될 수 없다", self.zenoh.namespace)));
        }
        if self.supervisor.backoff_ms.is_empty() {
            return Err(SystemError("supervisor.backoff_ms 가 비었다".into()));
        }
        let mut ids = std::collections::HashSet::new();
        for u in &self.unit {
            if !ids.insert(u.id.as_str()) {
                return Err(SystemError(format!("unit id {} 가 둘", u.id)));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(SystemError(format!("unit {} — process 인데 cmd 가 없다", u.id)));
            }
        }
        Ok(())
    }

    pub fn first_sfu(&self) -> Option<&Unit> {
        self.unit.iter().find(|u| u.role == UnitRole::Sfu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = r#"
[hub]
listen = "127.0.0.1:1974"
log_dir = "/tmp/x"
[hub.auth]
jwt_secret = "s"
[[hub.auth.api_keys]]
key = "k"
secret = "s"
participant_types = [0, 1]
hidden_allowed = true
ops_allowed = true
hook_allowed = true
[[unit]]
id = "log-a"
kind = "process"
order = 1
role = "log"
cmd = ["./oxlogd"]
[[unit]]
id = "node-a"
kind = "process"
order = 1
role = "sfu"
addr = "127.0.0.1:50061"
cmd = ["./oxsfud"]
[zenoh]
mode = "router"
listen = ["tcp/127.0.0.1:7447"]
connect = []
multicast_scouting = false
qos_enabled = true
lowlatency = false
namespace = "oxdev"
"#;

    #[test]
    fn dev_shape_parses_and_unknown_key_fails() {
        let s = SystemFile::parse(DEV).expect("dev");
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.first_sfu().map(|u| u.id.as_str()), Some("node-a"));
        assert_eq!(s.supervisor.intensity_max, 5);
        assert!(SystemFile::parse(&DEV.replace("namespace = \"oxdev\"", "namespace = \"oxdev\"\nbogus = 1")).is_err());
        assert!(SystemFile::parse(&DEV.replace("lowlatency = false", "lowlatency = true")).is_err());
    }
}
