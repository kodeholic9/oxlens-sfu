// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-CFG-SYSTEM-FILE 정:RULE-CFG-THREE-PLACES 정:CAND-CFG-SFUD-DEFAULT-PORTS 정:CAND-OPS-SUPERVISOR-DEFAULTS 정:RULE-BUS-SHAPE 운영:RULE-BOOT-ARGS 운영:RULE-BOOT-PORTS · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — strict 파싱 · 키 전량 · 전제 둘(`qos_enabled` · `lowlatency`)은 다른 값이면 기동 실패.

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
pub struct System {
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
    #[serde(default = "default_ptypes")]
    pub participant_types: Vec<u8>,
    #[serde(default)]
    pub hidden_allowed: bool,
    #[serde(default)]
    pub ops_allowed: bool,
    #[serde(default)]
    pub hook_allowed: bool,
}

fn default_ptypes() -> Vec<u8> {
    vec![0]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    Process,
    Remote,
    Inproc,
}

impl UnitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UnitKind::Process => "process",
            UnitKind::Remote => "remote",
            UnitKind::Inproc => "inproc",
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct Supervisor {
    #[serde(default = "default_backoff")]
    pub backoff_ms: Vec<u64>,
    #[serde(default = "default_intensity_max")]
    pub intensity_max: u32,
    #[serde(default = "default_intensity_window")]
    pub intensity_window_secs: u64,
    #[serde(default = "default_hello_timeout")]
    pub hello_timeout_ms: u64,
    #[serde(default = "default_node_grace")]
    pub node_grace_secs: u64,
}

impl Default for Supervisor {
    fn default() -> Self {
        Supervisor {
            backoff_ms: default_backoff(),
            intensity_max: default_intensity_max(),
            intensity_window_secs: default_intensity_window(),
            hello_timeout_ms: default_hello_timeout(),
            node_grace_secs: default_node_grace(),
        }
    }
}

fn default_backoff() -> Vec<u64> {
    vec![200, 500, 1000, 2000, 5000]
}

fn default_intensity_max() -> u32 {
    5
}

fn default_intensity_window() -> u64 {
    60
}

fn default_hello_timeout() -> u64 {
    10_000
}

fn default_node_grace() -> u64 {
    30
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

impl System {
    pub fn parse(text: &str) -> Result<System, SystemError> {
        let s: System = toml::from_str(text).map_err(|e| SystemError(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    pub fn validate(&self) -> Result<(), SystemError> {
        if self.zenoh.mode != "router" {
            return Err(SystemError(format!(
                "zenoh.mode {} — router 고정(정:RULE-BUS-SHAPE)",
                self.zenoh.mode
            )));
        }
        if !self.zenoh.qos_enabled {
            return Err(SystemError(
                "zenoh.qos_enabled = false — 전제(정:RULE-BUS-SHAPE)".into(),
            ));
        }
        if self.zenoh.lowlatency {
            return Err(SystemError(
                "zenoh.lowlatency = true — 전제(정:RULE-BUS-SHAPE)".into(),
            ));
        }
        if self.zenoh.namespace.is_empty()
            || self.zenoh.namespace.contains(['/', '*', '$', '?', '#'])
        {
            return Err(SystemError(format!(
                "zenoh.namespace {:?} — 키를 깨는 값",
                self.zenoh.namespace
            )));
        }
        if self.supervisor.backoff_ms.is_empty() {
            return Err(SystemError("supervisor.backoff_ms 가 비었다".into()));
        }
        let mut ids: Vec<&str> = self.unit.iter().map(|u| u.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        if ids.len() != n {
            return Err(SystemError("unit.id 가 겹친다".into()));
        }
        for u in &self.unit {
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(SystemError(format!(
                    "unit {} — process 인데 cmd 가 없다",
                    u.id
                )));
            }
            if u.role == UnitRole::Sfu && u.kind != UnitKind::Inproc && u.addr.is_none() {
                return Err(SystemError(format!(
                    "unit {} — sfu 인데 addr 가 없다",
                    u.id
                )));
            }
        }
        if let Some(tls) = &self.hub.tls
            && (tls.cert.is_empty() || tls.key.is_empty())
        {
            return Err(SystemError("hub.tls cert · key 가 비었다".into()));
        }
        Ok(())
    }

    pub fn first_sfu_id(&self) -> Option<&str> {
        self.unit
            .iter()
            .find(|u| u.role == UnitRole::Sfu)
            .map(|u| u.id.as_str())
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
[[unit]]
id = "node-a"
kind = "process"
order = 1
addr = "127.0.0.1:50061"
cmd = ["./oxsfud"]
[zenoh]
mode = "router"
listen = ["tcp/127.0.0.1:7447"]
namespace = "ox"
"#;

    #[test]
    fn minimal_parses_with_defaults() {
        let s = System::parse(MIN).expect("parse");
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.supervisor.backoff_ms, vec![200, 500, 1000, 2000, 5000]);
        assert_eq!(s.supervisor.node_grace_secs, 30);
        assert_eq!(s.first_sfu_id(), Some("node-a"));
    }

    #[test]
    fn unknown_key_fails_boot() {
        let bad = MIN.replace("[zenoh]", "[zenoh]\nbogus = 1");
        assert!(System::parse(&bad).is_err());
    }

    #[test]
    fn premise_values_are_fixed() {
        assert!(
            System::parse(&MIN.replace(
                "namespace = \"ox\"",
                "namespace = \"ox\"\nlowlatency = true"
            ))
            .is_err()
        );
        assert!(
            System::parse(&MIN.replace(
                "namespace = \"ox\"",
                "namespace = \"ox\"\nqos_enabled = false"
            ))
            .is_err()
        );
        assert!(System::parse(&MIN.replace("namespace = \"ox\"", "namespace = \"o/x\"")).is_err());
    }
}
