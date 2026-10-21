// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-CFG-SYSTEM-FILE 정:RULE-CFG-THREE-PLACES 정:RULE-BUS-SHAPE · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량 strict 파싱. 모르는 키 · 전제 둘(qos · lowlatency) 위반은 기동 실패.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemError(pub String);
impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "system: {}", self.0)
    }
}
impl std::error::Error for SystemError {}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct System {
    pub hub: HubSection,
    #[serde(default)]
    pub unit: Vec<Unit>,
    pub zenoh: Zenoh,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default)]
    pub hook: Option<HookSection>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubSection {
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    #[serde(default)]
    pub jwt_secret: Option<String>,
    #[serde(default)]
    pub api_keys: Vec<ApiKey>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_participant_types")]
    pub participant_types: Vec<u8>,
    #[serde(default)]
    pub hidden_allowed: bool,
    #[serde(default)]
    pub ops_allowed: bool,
    #[serde(default)]
    pub hook_allowed: bool,
}

fn default_participant_types() -> Vec<u8> {
    vec![0]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    Process,
    Remote,
    Inproc,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    #[serde(default)]
    pub order: u32,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_role() -> String {
    "sfu".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zenoh {
    pub mode: String,
    #[serde(default)]
    pub listen: Vec<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub multicast_scouting: bool,
    pub qos_enabled: bool,
    pub lowlatency: bool,
    pub namespace: String,
    #[serde(default)]
    pub base_config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Supervisor {
    pub backoff_ms: u64,
    pub intensity_max: u32,
    pub intensity_window_secs: u64,
    pub hello_timeout_ms: u64,
    pub node_grace_secs: u64,
}

impl Default for Supervisor {
    fn default() -> Self {
        Supervisor { backoff_ms: 500, intensity_max: 5, intensity_window_secs: 60, hello_timeout_ms: 10_000, node_grace_secs: 10 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookSection {
    #[serde(default)]
    pub webhooks: Vec<Webhook>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    pub url: String,
    pub secret: String,
}

const KEY_FORBIDDEN: &[char] = &['/', '*', '$', '?', '#'];

impl System {
    pub fn parse(text: &str) -> Result<System, SystemError> {
        let s: System = toml::from_str(text).map_err(|e| SystemError(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    pub fn load(path: &std::path::Path) -> Result<System, SystemError> {
        let text = std::fs::read_to_string(path).map_err(|e| SystemError(format!("{}: {e}", path.display())))?;
        System::parse(&text)
    }

    pub fn validate(&self) -> Result<(), SystemError> {
        if self.zenoh.mode != "router" {
            return Err(SystemError(format!("zenoh.mode = {} — router 고정", self.zenoh.mode)));
        }
        if !self.zenoh.qos_enabled {
            return Err(SystemError("zenoh.qos_enabled = false — true 고정(정:RULE-BUS-SHAPE)".into()));
        }
        if self.zenoh.lowlatency {
            return Err(SystemError("zenoh.lowlatency = true — false 고정(정:RULE-BUS-SHAPE)".into()));
        }
        if self.zenoh.namespace.is_empty() || self.zenoh.namespace.contains(KEY_FORBIDDEN) {
            return Err(SystemError(format!("zenoh.namespace = {:?} — 빈 값 · 키 금지 문자", self.zenoh.namespace)));
        }
        for u in &self.unit {
            if u.id.is_empty() || u.id.contains(KEY_FORBIDDEN) {
                return Err(SystemError(format!("unit.id = {:?} — 빈 값 · 키 금지 문자", u.id)));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() && u.enabled {
                return Err(SystemError(format!("unit {} — process 인데 cmd 가 없다", u.id)));
            }
        }
        Ok(())
    }

    pub fn first_sfu(&self) -> Option<&Unit> {
        self.unit.iter().find(|u| u.role == "sfu")
    }

    pub fn account(&self, key: &str) -> Option<&ApiKey> {
        self.hub.auth.api_keys.iter().find(|a| a.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[hub]
listen = "127.0.0.1:1974"
[[hub.auth.api_keys]]
key = "k"
secret = "s"
[[unit]]
id = "node-a"
kind = "process"
cmd = ["./x"]
[zenoh]
mode = "router"
qos_enabled = true
lowlatency = false
namespace = "oxdev"
"#;

    #[test]
    fn parses_and_defaults() {
        let s = System::parse(SAMPLE).expect("parse");
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.hub.auth.api_keys[0].participant_types, vec![0]);
        assert!(s.unit[0].enabled);
        assert_eq!(s.first_sfu().map(|u| u.id.as_str()), Some("node-a"));
    }

    #[test]
    fn strict() {
        assert!(System::parse(&format!("{SAMPLE}\n[extra]\nx=1\n")).is_err());
        assert!(System::parse(&SAMPLE.replace("lowlatency = false", "lowlatency = true")).is_err());
    }
}
