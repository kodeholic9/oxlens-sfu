// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량 strict(모르는 키 = 기동 실패) · zenoh 전제 둘은 다른 값이면 기동 실패.

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

impl UnitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UnitKind::Process => "process",
            UnitKind::Remote => "remote",
            UnitKind::Inproc => "inproc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    pub order: i32,
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
    #[serde(default = "default_true")]
    pub qos_enabled: bool,
    #[serde(default)]
    pub lowlatency: bool,
    pub namespace: String,
    #[serde(default)]
    pub base_config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
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
            return Err(SystemError(format!("zenoh.mode {} — router 고정", self.zenoh.mode)));
        }
        if !self.zenoh.qos_enabled {
            return Err(SystemError("zenoh.qos_enabled false — true 고정".into()));
        }
        if self.zenoh.lowlatency {
            return Err(SystemError("zenoh.lowlatency true — false 고정".into()));
        }
        if !crate::keyspace::valid_piece(&self.zenoh.namespace) {
            return Err(SystemError(format!("zenoh.namespace {:?} 는 키 조각이 못 된다", self.zenoh.namespace)));
        }
        let mut seen = std::collections::HashSet::new();
        for u in &self.unit {
            if !seen.insert(u.id.clone()) {
                return Err(SystemError(format!("unit id {} 중복", u.id)));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(SystemError(format!("unit {} process 인데 cmd 가 없다", u.id)));
            }
        }
        Ok(())
    }

    pub fn first_sfu_unit(&self) -> Option<&Unit> {
        self.unit.iter().find(|u| u.role == "sfu")
    }

    pub fn banner_fields(&self) -> Vec<(String, String)> {
        let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
        let h = &self.hub;
        let mut v: Vec<(String, String)> = vec![
            ("hub.listen".into(), h.listen.clone()),
            ("hub.base_path".into(), h.base_path.clone()),
            ("hub.log_dir".into(), opt(&h.log_dir)),
            ("hub.log_socket".into(), opt(&h.log_socket)),
            ("hub.tls.cert".into(), h.tls.as_ref().map(|t| t.cert.clone()).unwrap_or_else(|| "-".into())),
            ("hub.tls.key".into(), h.tls.as_ref().map(|t| t.key.clone()).unwrap_or_else(|| "-".into())),
            ("hub.auth.jwt_secret".into(), if h.auth.jwt_secret.is_some() { "***".into() } else { "-".into() }),
        ];
        for (i, k) in h.auth.api_keys.iter().enumerate() {
            let p = format!("hub.auth.api_keys[{i}]");
            v.push((format!("{p}.key"), k.key.clone()));
            v.push((format!("{p}.secret"), "***".into()));
            v.push((format!("{p}.name"), opt(&k.name)));
            v.push((
                format!("{p}.participant_types"),
                k.participant_types.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
            ));
            v.push((format!("{p}.hidden_allowed"), k.hidden_allowed.to_string()));
            v.push((format!("{p}.ops_allowed"), k.ops_allowed.to_string()));
            v.push((format!("{p}.hook_allowed"), k.hook_allowed.to_string()));
        }
        for (i, u) in self.unit.iter().enumerate() {
            let p = format!("unit[{i}]");
            v.push((format!("{p}.id"), u.id.clone()));
            v.push((format!("{p}.kind"), u.kind.as_str().into()));
            v.push((format!("{p}.order"), u.order.to_string()));
            v.push((format!("{p}.role"), u.role.clone()));
            v.push((format!("{p}.addr"), opt(&u.addr)));
            v.push((format!("{p}.cmd"), if u.cmd.is_empty() { "-".into() } else { u.cmd.join(" ") }));
            v.push((format!("{p}.enabled"), u.enabled.to_string()));
        }
        let z = &self.zenoh;
        v.push(("zenoh.mode".into(), z.mode.clone()));
        v.push(("zenoh.listen".into(), z.listen.join(",")));
        v.push(("zenoh.connect".into(), if z.connect.is_empty() { "-".into() } else { z.connect.join(",") }));
        v.push(("zenoh.multicast_scouting".into(), z.multicast_scouting.to_string()));
        v.push(("zenoh.qos_enabled".into(), z.qos_enabled.to_string()));
        v.push(("zenoh.lowlatency".into(), z.lowlatency.to_string()));
        v.push(("zenoh.namespace".into(), z.namespace.clone()));
        v.push(("zenoh.base_config".into(), opt(&z.base_config)));
        let s = &self.supervisor;
        v.push(("supervisor.backoff_ms".into(), s.backoff_ms.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")));
        v.push(("supervisor.intensity_max".into(), s.intensity_max.to_string()));
        v.push(("supervisor.intensity_window_secs".into(), s.intensity_window_secs.to_string()));
        v.push(("supervisor.hello_timeout_ms".into(), s.hello_timeout_ms.to_string()));
        v.push(("supervisor.node_grace_secs".into(), s.node_grace_secs.to_string()));
        match &self.hook {
            Some(hk) if !hk.webhooks.is_empty() => {
                for (i, w) in hk.webhooks.iter().enumerate() {
                    v.push((format!("hook.webhooks[{i}].url"), w.url.clone()));
                    v.push((format!("hook.webhooks[{i}].secret"), "***".into()));
                }
            }
            _ => v.push(("hook.webhooks".into(), "-".into())),
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = "[hub]\nlisten = \"0.0.0.0:1974\"\n[zenoh]\nmode = \"router\"\nnamespace = \"ox\"\n";

    #[test]
    fn strict_and_fixed() {
        let s = System::parse(MIN).unwrap();
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.supervisor.node_grace_secs, 30);
        assert!(System::parse(&format!("{MIN}typo = 1\n")).is_err());
        assert!(System::parse(&MIN.replace("router", "peer")).is_err());
        assert!(System::parse(&MIN.replace("\"ox\"", "\"o/x\"")).is_err());
        assert!(s.banner_fields().iter().any(|(k, v)| k == "hub.auth.jwt_secret" && v == "-"));
    }
}
