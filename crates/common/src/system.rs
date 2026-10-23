// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-OPS-SURFACE · 정:RULE-OPS-LOGGER · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량 strict 파싱. 모르는 키 · 전제 둘(qos · lowlatency) 위반은 기동 실패.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFile {
    pub hub: Hub,
    #[serde(default)]
    pub unit: Vec<Unit>,
    pub zenoh: Zenoh,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default)]
    pub hook: Option<HookFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hub {
    pub listen: String,
    #[serde(default = "base_path_default")]
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

fn base_path_default() -> String {
    "/media".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    #[serde(default)]
    pub jwt_secret: Option<String>,
    #[serde(default)]
    pub api_keys: Vec<ApiKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "types_default")]
    pub participant_types: Vec<u8>,
    #[serde(default)]
    pub hidden_allowed: bool,
    #[serde(default)]
    pub ops_allowed: bool,
    #[serde(default)]
    pub hook_allowed: bool,
}

fn types_default() -> Vec<u8> {
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
    Bus,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    #[serde(default)]
    pub order: u32,
    #[serde(default = "role_default")]
    pub role: UnitRole,
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn role_default() -> UnitRole {
    UnitRole::Sfu
}

fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zenoh {
    #[serde(default = "mode_default")]
    pub mode: String,
    #[serde(default)]
    pub listen: Vec<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub multicast_scouting: bool,
    #[serde(default = "enabled_default")]
    pub qos_enabled: bool,
    #[serde(default)]
    pub lowlatency: bool,
    #[serde(default = "namespace_default")]
    pub namespace: String,
    #[serde(default)]
    pub base_config: Option<String>,
}

fn mode_default() -> String {
    "router".to_string()
}

fn namespace_default() -> String {
    "ox".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
        Supervisor {
            backoff_ms: vec![200, 500, 1_000, 2_000, 5_000],
            intensity_max: 5,
            intensity_window_secs: 60,
            hello_timeout_ms: 10_000,
            node_grace_secs: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HookFile {
    #[serde(default)]
    pub webhooks: Vec<Webhook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    pub url: String,
    pub secret: String,
}

impl SystemFile {
    pub fn parse(text: &str) -> Result<SystemFile, String> {
        let s: SystemFile = toml::from_str(text).map_err(|e| format!("system: {e}"))?;
        s.validate()?;
        Ok(s)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.zenoh.mode != "router" {
            return Err(format!("system: zenoh.mode {} — router 고정", self.zenoh.mode));
        }
        if !self.zenoh.qos_enabled {
            return Err("system: zenoh.qos_enabled 는 true 고정(정:RULE-BUS-SHAPE)".to_string());
        }
        if self.zenoh.lowlatency {
            return Err("system: zenoh.lowlatency 는 false 고정(정:RULE-BUS-SHAPE)".to_string());
        }
        if self.zenoh.namespace.is_empty() || self.zenoh.namespace.contains(['/', '*', '$', '?', '#']) {
            return Err(format!("system: zenoh.namespace {:?} 는 키 조각이 될 수 없다", self.zenoh.namespace));
        }
        let mut ids = std::collections::BTreeSet::new();
        for u in &self.unit {
            if !ids.insert(u.id.clone()) {
                return Err(format!("system: unit id {} 겹침", u.id));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(format!("system: process 유닛 {} 에 cmd 가 없다", u.id));
            }
        }
        Ok(())
    }

    pub fn first_sfu(&self) -> Option<&Unit> {
        self.unit.iter().find(|u| u.role == UnitRole::Sfu)
    }

    pub fn account(&self, key: &str) -> Option<&ApiKey> {
        self.hub.auth.api_keys.iter().find(|a| a.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"
[hub]
listen = "127.0.0.1:1974"
[zenoh]
listen = ["tcp/127.0.0.1:7447"]
namespace = "oxdev"
"#;

    #[test]
    fn parses_minimum_and_rejects_unknown_key() {
        let s = SystemFile::parse(MIN).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.hub.base_path, "/media");
        assert!(SystemFile::parse(&format!("{MIN}\nbogus = 1\n")).is_err());
    }

    #[test]
    fn bus_premises_are_fixed() {
        let bad = MIN.replace("namespace = \"oxdev\"", "namespace = \"oxdev\"\nlowlatency = true");
        assert!(SystemFile::parse(&bad).is_err());
    }
}
