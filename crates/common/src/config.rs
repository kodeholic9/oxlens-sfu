// author: kodeholic (powered by Claude)
//! 설정 — 정§18-1 "값의 자리 셋". 시스템 파일(hub 전용)·정책 파일(hub·sfud 공통).
//! ★모르는 키는 기동 실패(strict) · 범위는 정책서 §3 · 부팅 1회 로드.

use std::fmt;
use std::path::Path;

use serde::Deserialize;

// ───────────── 시스템 파일 ─────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemConfig {
    #[serde(default)]
    pub dirs: Dirs,
    pub hub: HubSystem,
    #[serde(default)]
    pub supervisor: Supervisor,
    #[serde(default, rename = "unit")]
    pub units: Vec<Unit>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dirs {
    /// 비어 있으면 콘솔.
    #[serde(default)]
    pub log: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubSystem {
    pub listen: String,
    /// 연§5-0 `{base}` 의 경로 부분. 예: `/media`.
    #[serde(default)]
    pub base_path: String,
    #[serde(default)]
    pub tls: Tls,
    pub auth: HubAuth,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert_path: String,
    #[serde(default)]
    pub key_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubAuth {
    pub jwt_secret: String,
    /// 정§3-4 계정 목록 — `api_key` → `api_secret` · 허용 role.
    #[serde(default)]
    pub api_keys: Vec<ApiKey>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub key: String,
    pub secret: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
}

fn default_roles() -> Vec<String> {
    vec![crate::auth::ROLE_USER.to_owned(), crate::auth::ROLE_ADMIN.to_owned()]
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supervisor {
    #[serde(default)]
    pub enabled: bool,
}

/// 정§18-1 유닛 목록 — 노드 등재 자리는 이것 하나. `cmd` 없음 = 원격(dial 만).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub role: String,
    /// `node_id` — 안정 배치 키(정§15-1). `sfu_id`(기동값)와 다르다.
    pub id: String,
    pub addr: String,
    #[serde(default)]
    pub cmd: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_restart")]
    pub restart: String,
    #[serde(default = "default_timeout_stop")]
    pub timeout_stop_sec: u64,
}

fn default_restart() -> String {
    "on-failure".to_owned()
}
fn default_timeout_stop() -> u64 {
    10
}

// ───────────── 정책 파일 ─────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    #[serde(default)]
    pub logging: Logging,
    #[serde(default)]
    pub media: Media,
    #[serde(default)]
    pub floor: Floor,
    #[serde(default)]
    pub hub: HubPolicy,
    #[serde(default)]
    pub quota: Quota,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    #[serde(default = "default_level")]
    pub level: String,
}
impl Default for Logging {
    fn default() -> Self {
        Self { level: default_level() }
    }
}
fn default_level() -> String {
    "info".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    #[serde(default = "default_bwe")]
    pub bwe_mode: String,
    #[serde(default = "default_auto_layer")]
    pub auto_layer: String,
    /// 정본 = 정§4-2 `server_config` 값.
    #[serde(default = "default_max_bitrate")]
    pub max_bitrate_bps: u32,
}
impl Default for Media {
    fn default() -> Self {
        Self { bwe_mode: default_bwe(), auto_layer: default_auto_layer(), max_bitrate_bps: default_max_bitrate() }
    }
}
fn default_bwe() -> String {
    "twcc".to_owned()
}
fn default_auto_layer() -> String {
    "off".to_owned()
}
fn default_max_bitrate() -> u32 {
    800_000
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Floor {
    #[serde(default = "default_queue_max")]
    pub queue_max: u8,
    /// 정본 = 연§8-4 `T2`. wire 시계 중 정책 손잡이를 갖는 유일한 것.
    #[serde(default = "default_t2")]
    pub t2_stop_talking_secs: u16,
}
impl Default for Floor {
    fn default() -> Self {
        Self { queue_max: default_queue_max(), t2_stop_talking_secs: default_t2() }
    }
}
fn default_queue_max() -> u8 {
    10
}
fn default_t2() -> u16 {
    30
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubPolicy {
    #[serde(default = "default_token_ttl")]
    pub token_ttl_secs: u32,
    #[serde(default = "default_hb_interval")]
    pub heartbeat_interval_ms: u32,
    #[serde(default = "default_hb_timeout")]
    pub heartbeat_timeout_ms: u32,
    #[serde(default = "default_resume_window")]
    pub resume_window_ms: u32,
    #[serde(default = "default_flow_window")]
    pub ws_flow_window: u8,
    /// 없으면 영구(연§5-4).
    #[serde(default)]
    pub room_unused_ttl_secs: Option<u32>,
    #[serde(default)]
    pub room_departure_ttl_secs: Option<u32>,
    #[serde(default = "default_capacity")]
    pub room_default_capacity: u32,
    /// 연§5-1 — 브라우저 클라가 다른 origin 에서 붙을 때 허용할 origin.
    /// `["*"]` 은 전부. 빈 목록이면 CORS 헤더를 아예 안 낸다(같은 origin 배포 전제).
    #[serde(default = "default_origins")]
    pub allowed_origins: Vec<String>,
}
impl Default for HubPolicy {
    fn default() -> Self {
        Self {
            token_ttl_secs: default_token_ttl(),
            heartbeat_interval_ms: default_hb_interval(),
            heartbeat_timeout_ms: default_hb_timeout(),
            resume_window_ms: default_resume_window(),
            ws_flow_window: default_flow_window(),
            room_unused_ttl_secs: None,
            room_departure_ttl_secs: None,
            room_default_capacity: default_capacity(),
            allowed_origins: default_origins(),
        }
    }
}
fn default_token_ttl() -> u32 {
    3_600
}
fn default_hb_interval() -> u32 {
    10_000
}
fn default_hb_timeout() -> u32 {
    30_000
}
fn default_resume_window() -> u32 {
    60_000
}
fn default_flow_window() -> u8 {
    8
}
/// 연§5-1 — 웹 SDK 는 고객 앱에 들어가는 물건이라 다른 origin 이 기본이다.
/// 좁히려면 목록을 적는다(적으면 그것만, 비우면 헤더를 안 낸다).
fn default_origins() -> Vec<String> {
    vec!["*".to_owned()]
}

fn default_capacity() -> u32 {
    1_000
}

/// 정책서 §2 — `0` = 상한 없음(검사하지 않는다).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quota {
    #[serde(default)]
    pub max_sessions_per_account: u32,
    #[serde(default)]
    pub max_rooms_per_account: u32,
}

// ───────────── 로드·검증 ─────────────

#[derive(Debug)]
pub enum ConfigError {
    Io(String, std::io::Error),
    Parse(String, toml::de::Error),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(p, e) => write!(f, "{p}: {e}"),
            ConfigError::Parse(p, e) => write!(f, "{p}: {e}"),
            ConfigError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ConfigError {}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(shown.clone(), e))?;
    toml::from_str(&text).map_err(|e| ConfigError::Parse(shown, e))
}

impl SystemConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let cfg: SystemConfig = read(path)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.hub.jwt_secret_is_weak() {
            return Err(ConfigError::Invalid("hub.auth.jwt_secret 이 비어 있다".into()));
        }
        if self.hub.tls.enabled {
            return Err(ConfigError::Invalid("hub.tls.enabled=true 는 미지원 — 평문 위장 방지로 기동 거부".into()));
        }
        if !self.hub.base_path.is_empty() && !self.hub.base_path.starts_with('/') {
            return Err(ConfigError::Invalid("hub.base_path 는 '/' 로 시작한다".into()));
        }
        for u in &self.units {
            if u.id.is_empty() || u.addr.is_empty() {
                return Err(ConfigError::Invalid(format!("unit '{}' 에 id/addr 가 없다", u.id)));
            }
        }
        Ok(())
    }
}

impl HubSystem {
    fn jwt_secret_is_weak(&self) -> bool {
        self.auth.jwt_secret.is_empty()
    }
}

impl PolicyConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let cfg: PolicyConfig = read(path)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// 정책서 §3 범위 전량. 어기면 기동 실패.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Invalid(m));
        if !["trace", "debug", "info", "warn", "error"].contains(&self.logging.level.as_str()) {
            return bad(format!("logging.level '{}' 는 5값 밖", self.logging.level));
        }
        if !["twcc", "remb"].contains(&self.media.bwe_mode.as_str()) {
            return bad(format!("media.bwe_mode '{}'", self.media.bwe_mode));
        }
        if !["off", "v1", "v2"].contains(&self.media.auto_layer.as_str()) {
            return bad(format!("media.auto_layer '{}'", self.media.auto_layer));
        }
        if self.media.auto_layer == "v2" && self.media.bwe_mode != "twcc" {
            return bad("media.auto_layer=v2 는 bwe_mode=twcc 전제(정§10-2)".into());
        }
        if !(100_000..=5_000_000).contains(&self.media.max_bitrate_bps) {
            return bad(format!("media.max_bitrate_bps {} 는 100,000~5,000,000 밖", self.media.max_bitrate_bps));
        }
        if self.floor.queue_max == 0 {
            return bad("floor.queue_max 는 1~255".into());
        }
        if !(5..=600).contains(&self.floor.t2_stop_talking_secs) {
            return bad(format!("floor.t2_stop_talking_secs {} 는 5~600 밖", self.floor.t2_stop_talking_secs));
        }
        let h = &self.hub;
        if !(1_000..=60_000).contains(&h.heartbeat_interval_ms) {
            return bad(format!("hub.heartbeat_interval_ms {} 는 1,000~60,000 밖", h.heartbeat_interval_ms));
        }
        if h.heartbeat_timeout_ms < 2 * h.heartbeat_interval_ms {
            return bad("hub.heartbeat_timeout_ms 는 interval 의 2배 이상".into());
        }
        let backoff_sum: u64 = oxsig::timers::BACKOFF_MS.iter().sum();
        if u64::from(h.resume_window_ms) < backoff_sum {
            return bad(format!("hub.resume_window_ms {} 는 백오프 합계 {}ms 이상(연§8-2)", h.resume_window_ms, backoff_sum));
        }
        let w = usize::from(h.ws_flow_window);
        if !(oxsig::timers::WINDOW_DEFAULT..=oxsig::timers::WINDOW_MAX).contains(&w) {
            return bad(format!("hub.ws_flow_window {} 는 1~10 밖(연§3-2)", h.ws_flow_window));
        }
        if !(1..=1_000).contains(&h.room_default_capacity) {
            return bad(format!("hub.room_default_capacity {} 는 1~1000 밖(연§5-4)", h.room_default_capacity));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYS: &str = r#"
[hub]
listen = "127.0.0.1:1974"
base_path = "/media"
[hub.auth]
jwt_secret = "s"
[[hub.auth.api_keys]]
key = "k"
secret = "p"
[[unit]]
role = "sfu"
id = "sfu-1"
addr = "127.0.0.1:50051"
"#;

    #[test]
    fn strict_unknown_key_fails() {
        let sys: Result<SystemConfig, _> = toml::from_str(&format!("{SYS}\n[hub.extra]\nx = 1\n"));
        assert!(sys.is_err());
        let pol: Result<PolicyConfig, _> = toml::from_str("[hub]\nws_max_message_bytes = 1\n");
        assert!(pol.is_err());
    }

    #[test]
    fn system_defaults_and_validation() {
        let sys: SystemConfig = toml::from_str(SYS).unwrap();
        assert!(sys.validate().is_ok());
        assert_eq!(sys.hub.auth.api_keys[0].roles, vec!["user", "admin"]);
        assert!(sys.units[0].cmd.is_none());
        let tls: SystemConfig = toml::from_str(&SYS.replace("[hub.auth]", "[hub.tls]\nenabled = true\n[hub.auth]")).unwrap();
        assert!(tls.validate().is_err());
    }

    #[test]
    fn policy_defaults_are_spec_values_and_ranges_hold() {
        let p = PolicyConfig::default();
        assert!(p.validate().is_ok());
        assert_eq!((p.hub.heartbeat_interval_ms, p.hub.heartbeat_timeout_ms, p.hub.resume_window_ms), (10_000, 30_000, 60_000));
        assert_eq!(p.hub.token_ttl_secs, 3_600);
        assert_eq!(p.floor.t2_stop_talking_secs, 30);
        let bad: PolicyConfig = toml::from_str("[hub]\nresume_window_ms = 30000\n").unwrap();
        assert!(bad.validate().is_err());
        let bad2: PolicyConfig = toml::from_str("[media]\nauto_layer = \"v2\"\nbwe_mode = \"remb\"\n").unwrap();
        assert!(bad2.validate().is_err());
        let v2: PolicyConfig = toml::from_str("[media]\nauto_layer = \"v2\"\nbwe_mode = \"twcc\"\n").unwrap();
        assert!(v2.validate().is_ok(), "v2 + twcc 는 성립하는 짝이다");
        let ok: PolicyConfig = toml::from_str("[hub]\nws_flow_window = 1\n[quota]\nmax_sessions_per_account = 0\n").unwrap();
        assert!(ok.validate().is_ok());
    }
}
