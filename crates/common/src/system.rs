// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-MESH-SHAPE · 정:RULE-CHAT-GATEWAY-MODULE · 정:IMPL-CHAT-MEMBERSHIP · 정:IMPL-CHAT-STORE-FORMS · 정:IMPL-OPS-REC-UNIT · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! 시스템 파일(hub 전용) — 키 전량 strict(모르는 키 = 기동 실패) · `[bus]` 는 oxmesh router 의 접속 좌표와 배포 이름공간 · `[chat] db` 값의 꼴이 채팅 저장소를 고른다(모르는 꼴 = 기동 실패 · 배너는 비밀번호를 가린다) · `[data]` 는 남아야 할 파일의 뿌리(녹음 유닛이 있으면 필수) · 계정 `subscribe`(꼴이 틀린 항목 · 연동 허용 없는 계정 · 키 조각이 못 되는 계정 키 · 채팅 없음 = 기동 실패) · `[hook] zenoh_*`(zenoh 입구 — `tls/` 꼴 · 인증서 · 키 · CA 넷이 함께).

use serde::Deserialize;

use crate::extsub::{ExtSubs, Subscribe};

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
    pub bus: Bus,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default)]
    pub hook: Option<HookSection>,
    #[serde(default)]
    pub chat: Option<ChatSection>,
    #[serde(default)]
    pub data: Option<DataSection>,
}

pub const REC_KEEP_DAYS_DEFAULT: u32 = 30;
pub const ROLE_REC: &str = "rec";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataSection {
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub rec_keep_days: Option<u32>,
}

impl DataSection {
    pub fn keep_days(&self) -> u32 {
        self.rec_keep_days.unwrap_or(REC_KEEP_DAYS_DEFAULT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Visibility {
    #[default]
    #[serde(rename = "shared")]
    Shared,
    #[serde(rename = "joined")]
    Joined,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Shared => "shared",
            Visibility::Joined => "joined",
        }
    }
}

pub const DB_POOL_DEFAULT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatDb {
    Memory,
    Sqlite(String),
    Postgres(String),
}

impl ChatDb {
    pub fn parse(v: &str) -> Result<ChatDb, String> {
        if v == "memory:" {
            Ok(ChatDb::Memory)
        } else if let Some(p) = v.strip_prefix("sqlite:")
            && !p.is_empty()
        {
            Ok(ChatDb::Sqlite(p.to_string()))
        } else if v.starts_with("postgres://") {
            Ok(ChatDb::Postgres(v.to_string()))
        } else {
            Err(format!("chat.db {:?} — memory: · sqlite:<경로> · postgres://… 중 하나여야 한다", ChatDb::shown_of(v)))
        }
    }

    pub fn shown_of(v: &str) -> String {
        let Some((scheme, rest)) = v.split_once("://") else { return v.to_string() };
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (auth, tail) = rest.split_at(end);
        let Some((info, host)) = auth.rsplit_once('@') else { return v.to_string() };
        match info.split_once(':') {
            Some((user, _)) => format!("{scheme}://{user}:***@{host}{tail}"),
            None => v.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSection {
    pub db: String,
    #[serde(default)]
    pub db_pool: Option<usize>,
    #[serde(default)]
    pub history_visibility: Visibility,
    #[serde(default = "default_undelivered_ttl_days")]
    pub undelivered_ttl_days: u64,
    #[serde(default = "default_catchup_bundle_max_msgs")]
    pub catchup_bundle_max_msgs: usize,
    #[serde(default = "default_catchup_bundle_max_bytes")]
    pub catchup_bundle_max_bytes: usize,
    #[serde(default = "default_live_bundle_max_bytes")]
    pub live_bundle_max_bytes: usize,
}

impl ChatSection {
    pub fn form(&self) -> Result<ChatDb, String> {
        ChatDb::parse(&self.db)
    }

    pub fn pool(&self) -> usize {
        self.db_pool.unwrap_or(DB_POOL_DEFAULT)
    }
}

fn default_live_bundle_max_bytes() -> usize {
    65_536
}

fn default_undelivered_ttl_days() -> u64 {
    7
}

fn default_catchup_bundle_max_msgs() -> usize {
    100
}

fn default_catchup_bundle_max_bytes() -> usize {
    524_288
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
    #[serde(default = "default_send_queue_max_bytes")]
    pub send_queue_max_bytes: usize,
}

fn default_send_queue_max_bytes() -> usize {
    4_194_304
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
    #[serde(default)]
    pub subscribe: Option<Subscribe>,
}

impl ApiKey {
    pub fn subscribes(&self) -> bool {
        self.subscribe.as_ref().is_some_and(|s| !s.is_empty())
    }
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
pub struct Bus {
    #[serde(default)]
    pub listen: Vec<String>,
    #[serde(default)]
    pub advertise: Vec<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    pub namespace: String,
    #[serde(default = "default_follow_sessions")]
    pub follow_sessions: bool,
}

fn default_follow_sessions() -> bool {
    true
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
    #[serde(default)]
    pub zenoh_listen: Option<String>,
    #[serde(default)]
    pub zenoh_cert: Option<String>,
    #[serde(default)]
    pub zenoh_key: Option<String>,
    #[serde(default)]
    pub zenoh_ca: Option<String>,
}

pub const ZENOH_SCHEME: &str = "tls/";

impl HookSection {
    pub fn zenoh_files(&self) -> [(&'static str, Option<&str>); 3] {
        [("zenoh_cert", self.zenoh_cert.as_deref()), ("zenoh_key", self.zenoh_key.as_deref()), ("zenoh_ca", self.zenoh_ca.as_deref())]
    }

    fn validate(&self) -> Result<(), SystemError> {
        let files = self.zenoh_files();
        match self.zenoh_listen.as_deref() {
            Some(l) => {
                if !l.starts_with(ZENOH_SCHEME) || l.len() == ZENOH_SCHEME.len() {
                    return Err(SystemError(format!("hook.zenoh_listen {l:?} — tls/<host>:<port> 꼴만 받는다")));
                }
                if let Some((k, _)) = files.iter().find(|(_, v)| v.is_none_or(str::is_empty)) {
                    return Err(SystemError(format!("hook.zenoh_listen 이 있는데 hook.{k} 가 없다")));
                }
            }
            None => {
                if let Some((k, _)) = files.iter().find(|(_, v)| v.is_some()) {
                    return Err(SystemError(format!("hook.{k} 가 있는데 hook.zenoh_listen 이 없다 — 읽지 않는 키")));
                }
            }
        }
        Ok(())
    }
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
        if !crate::keyspace::valid_piece(&self.bus.namespace) {
            return Err(SystemError(format!("bus.namespace {:?} 는 키 조각이 못 된다", self.bus.namespace)));
        }
        let mut seen = std::collections::HashSet::new();
        for u in &self.unit {
            if !seen.insert(u.id.clone()) {
                return Err(SystemError(format!("unit id {} 중복", u.id)));
            }
            if u.kind == UnitKind::Process && u.cmd.is_empty() {
                return Err(SystemError(format!("unit {} process 인데 cmd 가 없다", u.id)));
            }
            if u.role == ROLE_REC && u.kind != UnitKind::Process {
                return Err(SystemError(format!("unit {} role = rec 는 kind = process 다", u.id)));
            }
        }
        if self.unit.iter().any(|u| u.role == ROLE_REC && u.enabled) && self.data_root().is_none() {
            return Err(SystemError("role = rec 유닛이 있는데 [data] root 가 없다".into()));
        }
        for k in &self.hub.auth.api_keys {
            if !k.subscribes() {
                continue;
            }
            if !k.hook_allowed {
                return Err(SystemError(format!("계정 {:?} — subscribe 는 hook_allowed 계정에만 둔다", k.key)));
            }
            if !crate::keyspace::valid_piece(&k.key) {
                return Err(SystemError(format!("계정 {:?} — subscribe 가 있는 계정의 key 는 버스 키 조각이 못 된다(빈 값 · / * $ ? #)", k.key)));
            }
        }
        if let Some(h) = &self.hook {
            h.validate()?;
        }
        if self.chat.is_none() {
            if let Some(k) = self.hub.auth.api_keys.iter().find(|k| k.subscribes()) {
                return Err(SystemError(format!("계정 {:?} — subscribe 가 있는데 [chat] 이 없다", k.key)));
            }
            if self.zenoh_listen().is_some() {
                return Err(SystemError("hook.zenoh_listen 이 있는데 [chat] 이 없다".into()));
            }
        }
        if let Some(c) = &self.chat {
            let form = c.form().map_err(SystemError)?;
            match (c.db_pool, &form) {
                (Some(_), ChatDb::Memory | ChatDb::Sqlite(_)) => return Err(SystemError("chat.db_pool 은 postgres:// 꼴에서만 읽는다".into())),
                (Some(0), _) => return Err(SystemError("chat.db_pool 은 1 이상".into())),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn zenoh_listen(&self) -> Option<&str> {
        self.hook.as_ref().and_then(|h| h.zenoh_listen.as_deref())
    }

    pub fn ext_subs(&self) -> ExtSubs {
        ExtSubs::new(
            self.hub
                .auth
                .api_keys
                .iter()
                .filter(|k| k.hook_allowed)
                .filter_map(|k| k.subscribe.clone().map(|s| (k.key.clone(), s)))
                .collect(),
        )
    }

    pub fn first_sfu_unit(&self) -> Option<&Unit> {
        self.unit.iter().find(|u| u.role == "sfu")
    }

    pub fn data_root(&self) -> Option<&str> {
        self.data.as_ref().and_then(|d| d.root.as_deref()).filter(|r| !r.is_empty())
    }

    pub fn rec_keep_days(&self) -> u32 {
        self.data.as_ref().map(|d| d.keep_days()).unwrap_or(REC_KEEP_DAYS_DEFAULT)
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
            ("hub.send_queue_max_bytes".into(), h.send_queue_max_bytes.to_string()),
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
            v.push((
                format!("{p}.subscribe"),
                match &k.subscribe {
                    Some(s) => format!("[{}]", s.raw().iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(",")),
                    None => "-".into(),
                },
            ));
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
        let b = &self.bus;
        v.push(("bus.listen".into(), b.listen.join(",")));
        v.push(("bus.advertise".into(), if b.advertise.is_empty() { "-".into() } else { b.advertise.join(",") }));
        v.push(("bus.connect".into(), if b.connect.is_empty() { "-".into() } else { b.connect.join(",") }));
        v.push(("bus.namespace".into(), b.namespace.clone()));
        v.push(("bus.follow_sessions".into(), b.follow_sessions.to_string()));
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
        let hk = self.hook.clone().unwrap_or_default();
        v.push(("hook.zenoh_listen".into(), hk.zenoh_listen.clone().unwrap_or_else(|| "-".into())));
        for (k, val) in hk.zenoh_files() {
            v.push((format!("hook.{k}"), val.unwrap_or("-").to_string()));
        }
        match &self.chat {
            Some(c) => {
                v.push(("chat.db".into(), ChatDb::shown_of(&c.db)));
                let pg = matches!(c.form(), Ok(ChatDb::Postgres(_)));
                v.push(("chat.db_pool".into(), if pg { c.pool().to_string() } else { "-".into() }));
                v.push(("chat.history_visibility".into(), c.history_visibility.as_str().into()));
                v.push(("chat.undelivered_ttl_days".into(), c.undelivered_ttl_days.to_string()));
                v.push(("chat.catchup_bundle_max_msgs".into(), c.catchup_bundle_max_msgs.to_string()));
                v.push(("chat.catchup_bundle_max_bytes".into(), c.catchup_bundle_max_bytes.to_string()));
                v.push(("chat.live_bundle_max_bytes".into(), c.live_bundle_max_bytes.to_string()));
            }
            None => v.push(("chat".into(), "-".into())),
        }
        v.push(("data.root".into(), self.data_root().unwrap_or("-").to_string()));
        v.push(("data.rec_keep_days".into(), self.rec_keep_days().to_string()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = "[hub]\nlisten = \"0.0.0.0:1974\"\n[bus]\nnamespace = \"ox\"\n";

    #[test]
    fn strict_and_fixed() {
        let s = System::parse(MIN).unwrap();
        assert_eq!(s.hub.base_path, "/media");
        assert_eq!(s.supervisor.node_grace_secs, 30);
        assert!(System::parse(&format!("{MIN}typo = 1\n")).is_err());
        assert!(System::parse(&MIN.replace("[bus]", "[zenoh]")).is_err());
        assert!(System::parse(&format!("{MIN}mode = \"router\"\n")).is_err());
        assert!(System::parse(&MIN.replace("\"ox\"", "\"o/x\"")).is_err());
        assert!(s.banner_fields().iter().any(|(k, v)| k == "hub.auth.jwt_secret" && v == "-"));
        assert!(s.banner_fields().iter().any(|(k, v)| k == "hub.send_queue_max_bytes" && v == "4194304"));
        let c = System::parse(&format!("{MIN}[chat]\ndb = \"memory:\"\n")).unwrap();
        assert!(c.banner_fields().iter().any(|(k, v)| k == "chat.live_bundle_max_bytes" && v == "65536"));
        assert!(c.banner_fields().iter().any(|(k, v)| k == "chat.db_pool" && v == "-"));
        let p = System::parse(&format!("{MIN}[chat]\ndb = \"postgres://u@h/d\"\n")).unwrap();
        assert!(p.banner_fields().iter().any(|(k, v)| k == "chat.db_pool" && v == "16"));
        assert!(System::parse(&format!("{MIN}[chat]\ndb = \"x\"\n")).is_err());
        assert!(System::parse(&format!("{MIN}[chat]\ndb = \"sqlite:/x\"\ndb_pool = 4\n")).is_err());
    }

    #[test]
    fn rec_unit_needs_data_root_and_process() {
        let rec = "[[unit]]\nid = \"r1\"\nkind = \"process\"\norder = 3\nrole = \"rec\"\ncmd = [\"oxtapd\"]\n";
        assert!(System::parse(&format!("{MIN}{rec}")).is_err());
        let s = System::parse(&format!("{MIN}{rec}[data]\nroot = \"/d\"\n")).unwrap();
        assert_eq!((s.data_root(), s.rec_keep_days()), (Some("/d"), 30));
        assert!(s.banner_fields().iter().any(|(k, v)| k == "data.root" && v == "/d"));
        let k = System::parse(&format!("{MIN}{rec}[data]\nroot = \"/d\"\nrec_keep_days = 7\n")).unwrap();
        assert_eq!(k.rec_keep_days(), 7);
        let inproc = rec.replace("kind = \"process\"", "kind = \"inproc\"");
        assert!(System::parse(&format!("{MIN}{inproc}[data]\nroot = \"/d\"\n")).is_err());
        assert!(System::parse(&format!("{MIN}[data]\nroots = \"/d\"\n")).is_err());
        let m = System::parse(MIN).unwrap();
        assert!(m.banner_fields().iter().any(|(k, v)| k == "data.root" && v == "-"));
    }

    #[test]
    fn subscribe_and_zenoh_gate_boot() {
        let chat = "[chat]\ndb = \"memory:\"\n";
        let acct = |extra: &str| format!("[[hub.auth.api_keys]]\nkey = \"acme\"\nsecret = \"s\"\nhook_allowed = true\n{extra}");
        let ok = System::parse(&format!("{MIN}{}{chat}", acct("subscribe = [\"\", \"com.acme.*\", \"talk\", \"*\"]\n"))).unwrap();
        assert_eq!(ok.ext_subs().keys().collect::<Vec<_>>(), vec!["acme"]);
        assert!(ok.banner_fields().iter().any(|(k, v)| k == "hub.auth.api_keys[0].subscribe" && v == "[\"\",\"com.acme.*\",\"talk\",\"*\"]"));
        assert!(ok.banner_fields().iter().any(|(k, v)| k == "hook.zenoh_listen" && v == "-"));
        for bad in ["a*b", ".*", "x.*.y", "**"] {
            assert!(System::parse(&format!("{MIN}{}{chat}", acct(&format!("subscribe = [\"{bad}\"]\n")))).is_err(), "정:IMPL-CFG-SYSTEM-FILE — 꼴이 틀린 항목 {bad}");
        }
        let no_hook = format!("{MIN}[[hub.auth.api_keys]]\nkey = \"acme\"\nsecret = \"s\"\nsubscribe = [\"\"]\n{chat}");
        assert!(System::parse(&no_hook).is_err(), "정:IMPL-CFG-SYSTEM-FILE — hook_allowed 아닌 계정의 subscribe");
        let slash = format!("{MIN}[[hub.auth.api_keys]]\nkey = \"ac/me\"\nsecret = \"s\"\nhook_allowed = true\nsubscribe = [\"\"]\n{chat}");
        assert!(System::parse(&slash).is_err(), "정:RULE-BUS-KEYSPACE — subscribe 계정 키가 버스 금지 문자를 품음");
        let slash_plain = format!("{MIN}[[hub.auth.api_keys]]\nkey = \"ac/me\"\nsecret = \"s\"\n");
        assert!(System::parse(&slash_plain).is_ok(), "형상 — 구독 없는 계정 키는 보지 않는다");
        assert!(System::parse(&format!("{MIN}{}", acct("subscribe = [\"\"]\n"))).is_err(), "정:IMPL-CFG-SYSTEM-FILE — [chat] 없이 subscribe");
        assert!(System::parse(&format!("{MIN}{}", acct("subscribe = []\n"))).is_ok(), "정:IMPL-CFG-SYSTEM-FILE — 빈 배열은 없는 것과 같다");
        let z = "[hook]\nzenoh_listen = \"tls/0.0.0.0:7447\"\nzenoh_cert = \"c.pem\"\nzenoh_key = \"k.pem\"\nzenoh_ca = \"ca.pem\"\n";
        let zs = System::parse(&format!("{MIN}{z}{chat}")).unwrap();
        assert_eq!(zs.zenoh_listen(), Some("tls/0.0.0.0:7447"));
        assert!(zs.banner_fields().iter().any(|(k, v)| k == "hook.zenoh_ca" && v == "ca.pem"));
        assert!(System::parse(&format!("{MIN}{z}")).is_err(), "정:IMPL-CFG-SYSTEM-FILE — [chat] 없이 zenoh_listen");
        assert!(System::parse(&format!("{MIN}{}{chat}", z.replace("tls/", "tcp/"))).is_err(), "정:IMPL-CFG-SYSTEM-FILE — tls/ 꼴만");
        assert!(System::parse(&format!("{MIN}{}{chat}", z.replace("zenoh_ca = \"ca.pem\"\n", ""))).is_err(), "정:IMPL-CFG-SYSTEM-FILE — 입구가 있는데 CA 가 없음");
        assert!(System::parse(&format!("{MIN}[hook]\nzenoh_cert = \"c.pem\"\n{chat}")).is_err(), "정:IMPL-CFG-SYSTEM-FILE — 입구 없이 인증서(읽지 않는 키)");
        assert!(System::parse(&format!("{MIN}[hook]\nzenoh_port = 1\n{chat}")).is_err(), "형상 — 모르는 키");
    }
}
