// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:CAND-OPS-SUPERVISOR-DEFAULTS · 운영:RULE-BOOT-PORTS · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량 strict 파싱(모르는 키 = 기동 실패) · 버스 전제 둘 · 키 조각 검사.

use serde::Deserialize;

use crate::banner::Banner;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFile {
    pub hub: Hub,
    #[serde(default, rename = "unit")]
    pub units: Vec<Unit>,
    pub zenoh: Zenoh,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default)]
    pub hook: Option<Hook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
    pub auth: Auth,
}

fn default_base_path() -> String {
    "/media".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub cert: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

impl UnitRole {
    pub fn as_str(self) -> &'static str {
        match self {
            UnitRole::Sfu => "sfu",
            UnitRole::Log => "log",
        }
    }
}

fn default_role() -> UnitRole {
    UnitRole::Sfu
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub id: String,
    pub kind: UnitKind,
    pub order: i32,
    #[serde(default = "default_role")]
    pub role: UnitRole,
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
    fn default() -> Supervisor {
        Supervisor { backoff_ms: vec![200, 500, 1000, 2000, 5000], intensity_max: 5, intensity_window_secs: 60, hello_timeout_ms: 10_000, node_grace_secs: 30 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
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

    pub fn load(path: &str) -> Result<SystemFile, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("system: {path}: {e}"))?;
        SystemFile::parse(&text)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.zenoh.mode != "router" {
            return Err(format!("system: zenoh.mode = {} — router 고정", self.zenoh.mode));
        }
        if !self.zenoh.qos_enabled {
            return Err("system: zenoh.qos_enabled = false — true 고정(정:RULE-BUS-SHAPE)".into());
        }
        if self.zenoh.lowlatency {
            return Err("system: zenoh.lowlatency = true — false 고정(정:RULE-BUS-SHAPE)".into());
        }
        crate::bus::keys::check_fragment("zenoh.namespace", &self.zenoh.namespace)?;
        let mut seen = std::collections::BTreeSet::new();
        for u in &self.units {
            if !seen.insert(u.id.as_str()) {
                return Err(format!("system: unit id 겹침 {}", u.id));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(format!("system: unit {} — process 인데 cmd 가 없다", u.id));
            }
            if u.role == UnitRole::Sfu {
                crate::bus::keys::check_fragment("unit.id", &u.id)?;
            }
        }
        if self.supervisor.backoff_ms.is_empty() {
            return Err("system: supervisor.backoff_ms 가 비었다".into());
        }
        Ok(())
    }

    pub fn first_sfu(&self) -> Option<&Unit> {
        self.units.iter().find(|u| u.role == UnitRole::Sfu)
    }

    pub fn banner(&self, b: &mut Banner) {
        b.put("hub.listen", &self.hub.listen);
        b.put("hub.base_path", &self.hub.base_path);
        b.put_opt("hub.log_dir", self.hub.log_dir.as_ref());
        b.put_opt("hub.log_socket", self.hub.log_socket.as_ref());
        match &self.hub.tls {
            Some(t) => {
                b.put("hub.tls.cert", &t.cert);
                b.put("hub.tls.key", &t.key);
            }
            None => {
                b.put("hub.tls.cert", crate::banner::ABSENT);
                b.put("hub.tls.key", crate::banner::ABSENT);
            }
        }
        b.put_secret("hub.auth.jwt_secret", self.hub.auth.jwt_secret.is_some());
        for (i, k) in self.hub.auth.api_keys.iter().enumerate() {
            let p = format!("hub.auth.api_keys[{i}]");
            b.put(format!("{p}.key"), &k.key);
            b.put_secret(format!("{p}.secret"), true);
            b.put_opt(format!("{p}.name"), k.name.as_ref());
            b.put_list(format!("{p}.participant_types"), &k.participant_types);
            b.put(format!("{p}.hidden_allowed"), k.hidden_allowed);
            b.put(format!("{p}.ops_allowed"), k.ops_allowed);
            b.put(format!("{p}.hook_allowed"), k.hook_allowed);
        }
        for (i, u) in self.units.iter().enumerate() {
            let p = format!("unit[{i}]");
            b.put(format!("{p}.id"), &u.id);
            b.put(format!("{p}.kind"), u.kind.as_str());
            b.put(format!("{p}.order"), u.order);
            b.put(format!("{p}.role"), u.role.as_str());
            b.put_opt(format!("{p}.addr"), u.addr.as_ref());
            b.put(format!("{p}.cmd"), u.cmd.join(" "));
            b.put(format!("{p}.enabled"), u.enabled);
        }
        b.put("zenoh.mode", &self.zenoh.mode);
        b.put_list("zenoh.listen", &self.zenoh.listen);
        b.put_list("zenoh.connect", &self.zenoh.connect);
        b.put("zenoh.multicast_scouting", self.zenoh.multicast_scouting);
        b.put("zenoh.qos_enabled", self.zenoh.qos_enabled);
        b.put("zenoh.lowlatency", self.zenoh.lowlatency);
        b.put("zenoh.namespace", &self.zenoh.namespace);
        b.put_opt("zenoh.base_config", self.zenoh.base_config.as_ref());
        b.put_list("supervisor.backoff_ms", &self.supervisor.backoff_ms);
        b.put("supervisor.intensity_max", self.supervisor.intensity_max);
        b.put("supervisor.intensity_window_secs", self.supervisor.intensity_window_secs);
        b.put("supervisor.hello_timeout_ms", self.supervisor.hello_timeout_ms);
        b.put("supervisor.node_grace_secs", self.supervisor.node_grace_secs);
        match &self.hook {
            Some(h) => {
                for (i, w) in h.webhooks.iter().enumerate() {
                    b.put(format!("hook.webhooks[{i}].url"), &w.url);
                    b.put_secret(format!("hook.webhooks[{i}].secret"), !w.secret.is_empty());
                }
            }
            None => b.put("hook", crate::banner::ABSENT),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "s"
[[hub.auth.api_keys]]
key = "k"
secret = "x"
[[unit]]
id = "node-a"
kind = "process"
order = 1
cmd = ["oxsfud"]
[zenoh]
mode = "router"
namespace = "ox"
"#;

    #[test]
    fn defaults_fill_in() {
        let s = SystemFile::parse(BASE).expect("parse");
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.hub.auth.api_keys[0].participant_types, vec![0]);
        assert_eq!(s.units[0].role, UnitRole::Sfu);
        assert!(s.units[0].enabled);
        assert_eq!(s.supervisor.node_grace_secs, 30);
        assert_eq!(s.supervisor.backoff_ms, vec![200, 500, 1000, 2000, 5000]);
    }

    #[test]
    fn unknown_key_and_bad_bus_premises_fail() {
        assert!(SystemFile::parse(&format!("{BASE}\nunknown = 1\n")).is_err());
        assert!(SystemFile::parse(&BASE.replace("namespace = \"ox\"", "namespace = \"ox\"\nlowlatency = true")).is_err());
        assert!(SystemFile::parse(&BASE.replace("namespace = \"ox\"", "namespace = \"o*x\"")).is_err());
        assert!(SystemFile::parse(&BASE.replace("mode = \"router\"", "mode = \"peer\"")).is_err());
    }

    #[test]
    fn banner_hides_secrets() {
        let s = SystemFile::parse(BASE).expect("parse");
        let mut b = Banner::new();
        s.banner(&mut b);
        let r = b.render();
        assert!(r.contains("hub.auth.jwt_secret=***"));
        assert!(r.contains("hub.auth.api_keys[0].secret=***"));
        assert!(!r.contains("=x "));
    }
}
