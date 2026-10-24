// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CFG-THREE-PLACES · 정:RULE-CFG-INDEX · 정:IMPL-ROOM-LIFE · 정:IMPL-SES-ADMISSION · 정:IMPL-FLR-POLICY · 정:RULE-SIM-MODES · 정책:hub.room_unused_ttl_secs · 정책:hub.room_departure_ttl_secs · model: claude-opus-5-5
//! 정책 파일(hub · sfud 공통) — 형 · 기본값 · 범위 · 교차 규칙은 정책 원천(policy/keys.ts)의 생성 구조체 그대로다. 모르는 키 · 범위 밖은 기동 실패.
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyError(pub String);
impl std::fmt::Display for PolicyError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "policy: {}", self.0) } }
impl std::error::Error for PolicyError {}
fn bad<T>(msg: impl Into<String>) -> Result<T, PolicyError> { Err(PolicyError(msg.into())) }

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Policy {
    pub logging: Logging,
    pub media: Media,
    pub floor: Floor,
    pub hub: Hub,
    pub quota: Quota,
    pub hook: Hook,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoggingLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Logging {
    pub level: LoggingLevel,
    pub rotate_max_bytes: u64,
    pub retention_days: u16,
    pub line_max_bytes: u16,
    pub queue_max_lines: u32,
    pub spool_max_bytes: u64,
    pub socket_rcvbuf_bytes: u32,
    pub stat_interval_ms: u32,
}
impl Default for Logging {
    fn default() -> Self {
        Self {
            level: LoggingLevel::Info,
            rotate_max_bytes: 268_435_456,
            retention_days: 14,
            line_max_bytes: 1_024,
            queue_max_lines: 8_192,
            spool_max_bytes: 67_108_864,
            socket_rcvbuf_bytes: 4_194_304,
            stat_interval_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaBwemode {
    Twcc,
    Remb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaAutolayer {
    Off,
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Media {
    pub bwe_mode: MediaBwemode,
    pub slot_mid_reserve: u16,
    pub auto_layer: MediaAutolayer,
    pub max_bitrate_bps: u32,
}
impl Default for Media {
    fn default() -> Self {
        Self {
            bwe_mode: MediaBwemode::Twcc,
            slot_mid_reserve: 40,
            auto_layer: MediaAutolayer::Off,
            max_bitrate_bps: 800_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Floor {
    pub queue_max: u8,
    pub t2_stop_talking_secs: u16,
}
impl Default for Floor {
    fn default() -> Self {
        Self {
            queue_max: 10,
            t2_stop_talking_secs: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Hub {
    pub token_ttl_secs: u32,
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    pub resume_window_ms: u32,
    pub ws_flow_window: u8,
    pub room_unused_ttl_secs: u32,
    pub room_departure_ttl_secs: u32,
    pub room_default_capacity: u32,
    pub metadata_max_bytes: u32,
    pub join_admit_per_sec: u32,
    pub join_retry_after_ms: u32,
    pub allowed_origins: Vec<String>,
}
impl Default for Hub {
    fn default() -> Self {
        Self {
            token_ttl_secs: 3_600,
            heartbeat_interval_ms: 10_000,
            heartbeat_timeout_ms: 30_000,
            resume_window_ms: 60_000,
            ws_flow_window: 8,
            room_unused_ttl_secs: 0,
            room_departure_ttl_secs: 0,
            room_default_capacity: 1_000,
            metadata_max_bytes: 2_048,
            join_admit_per_sec: 0,
            join_retry_after_ms: 5_000,
            allowed_origins: vec!["*".to_string()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Quota {
    pub max_sessions: u32,
    pub max_rooms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Hook {
    pub retry_backoff_ms: Vec<u32>,
    pub retry_max: u8,
    pub queue_max: u32,
}
impl Default for Hook {
    fn default() -> Self {
        Self {
            retry_backoff_ms: vec![1000, 2000, 4000, 8000, 16000],
            retry_max: 5,
            queue_max: 10_000,
        }
    }
}

impl Policy {
    pub fn parse(text: &str) -> Result<Policy, PolicyError> {
        let p: Policy = toml::from_str(text).map_err(|e| PolicyError(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.hub.token_ttl_secs < 60 { return bad(format!("hub.token_ttl_secs {} 는 60 미만", self.hub.token_ttl_secs)); }
        if !(1000..=60000).contains(&self.hub.heartbeat_interval_ms) { return bad(format!("hub.heartbeat_interval_ms {} 는 1000~60000 밖", self.hub.heartbeat_interval_ms)); }
        if self.hub.heartbeat_timeout_ms < 2000 { return bad(format!("hub.heartbeat_timeout_ms {} 는 2000 미만", self.hub.heartbeat_timeout_ms)); }
        if self.hub.resume_window_ms < 52000 { return bad(format!("hub.resume_window_ms {} 는 52000 미만", self.hub.resume_window_ms)); }
        if !(1..=10).contains(&self.hub.ws_flow_window) { return bad(format!("hub.ws_flow_window {} 는 1~10 밖", self.hub.ws_flow_window)); }
        if self.hub.room_default_capacity < 1 { return bad(format!("hub.room_default_capacity {} 는 1 미만", self.hub.room_default_capacity)); }
        if self.hub.metadata_max_bytes < 256 { return bad(format!("hub.metadata_max_bytes {} 는 256 미만", self.hub.metadata_max_bytes)); }
        if self.hub.join_retry_after_ms < 100 { return bad(format!("hub.join_retry_after_ms {} 는 100 미만", self.hub.join_retry_after_ms)); }
        if self.logging.rotate_max_bytes < 1048576 { return bad(format!("logging.rotate_max_bytes {} 는 1048576 미만", self.logging.rotate_max_bytes)); }
        if !(256..=2047).contains(&self.logging.line_max_bytes) { return bad(format!("logging.line_max_bytes {} 는 256~2047 밖", self.logging.line_max_bytes)); }
        if self.logging.queue_max_lines < 256 { return bad(format!("logging.queue_max_lines {} 는 256 미만", self.logging.queue_max_lines)); }
        if self.logging.spool_max_bytes < 1048576 { return bad(format!("logging.spool_max_bytes {} 는 1048576 미만", self.logging.spool_max_bytes)); }
        if self.logging.socket_rcvbuf_bytes < 65536 { return bad(format!("logging.socket_rcvbuf_bytes {} 는 65536 미만", self.logging.socket_rcvbuf_bytes)); }
        if !(200..=60000).contains(&self.logging.stat_interval_ms) { return bad(format!("logging.stat_interval_ms {} 는 200~60000 밖", self.logging.stat_interval_ms)); }
        if !(0..=128).contains(&self.media.slot_mid_reserve) { return bad(format!("media.slot_mid_reserve {} 는 0~128 밖", self.media.slot_mid_reserve)); }
        if !(100000..=5000000).contains(&self.media.max_bitrate_bps) { return bad(format!("media.max_bitrate_bps {} 는 100000~5000000 밖", self.media.max_bitrate_bps)); }
        if !(1..=255).contains(&self.floor.queue_max) { return bad(format!("floor.queue_max {} 는 1~255 밖", self.floor.queue_max)); }
        if !(5..=600).contains(&self.floor.t2_stop_talking_secs) { return bad(format!("floor.t2_stop_talking_secs {} 는 5~600 밖", self.floor.t2_stop_talking_secs)); }
        if self.hook.queue_max < 1 { return bad(format!("hook.queue_max {} 는 1 미만", self.hook.queue_max)); }
        if self.hub.heartbeat_timeout_ms < 2 * self.hub.heartbeat_interval_ms { return bad("hb-timeout: hub.heartbeat_timeout_ms ≥ 2 × hub.heartbeat_interval_ms".to_string()); }
        if self.hub.resume_window_ms < 52_000 { return bad("resume-window: hub.resume_window_ms ≥ 52,000 — 연:RULE-SES-BACKOFF-LADDER 백오프 합계(44~52초)보다 길어야 정상 재접속이 세션을 잃지 않는다".to_string()); }
        if self.media.auto_layer == MediaAutolayer::V2 && self.media.bwe_mode != MediaBwemode::Twcc { return bad("v2-twcc: media.auto_layer = v2 면 media.bwe_mode = twcc(정:RULE-SIM-TICK v2 = TWCC send-side)".to_string()); }
        if self.hook.retry_max as usize > self.hook.retry_backoff_ms.len() { return bad("hook-retry: hook.retry_max ≤ hook.retry_backoff_ms 길이 — 넘으면 마지막 칸을 반복할지가 안 정해졌다".to_string()); }
        Ok(())
    }
}

impl Policy {
    pub fn load(path: &std::path::Path) -> Result<Policy, PolicyError> {
        let text = std::fs::read_to_string(path).map_err(|e| PolicyError(format!("{}: {e}", path.display())))?;
        Policy::parse(&text)
    }
}

impl LoggingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LoggingLevel::Trace => "trace",
            LoggingLevel::Debug => "debug",
            LoggingLevel::Info => "info",
            LoggingLevel::Warn => "warn",
            LoggingLevel::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<LoggingLevel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(LoggingLevel::Trace),
            "debug" => Some(LoggingLevel::Debug),
            "info" => Some(LoggingLevel::Info),
            "warn" => Some(LoggingLevel::Warn),
            "error" => Some(LoggingLevel::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_defaults_are_policy_source() {
        let p = Policy::parse("").expect("empty");
        assert_eq!(p.hub.room_unused_ttl_secs, 0);
        assert_eq!(p.hub.room_departure_ttl_secs, 0);
        assert_eq!(p.media.auto_layer, MediaAutolayer::Off);
        assert!(Policy::parse("[hub]\nnope = 1\n").is_err());
        assert!(Policy::parse("[media]\nauto_layer = \"v2\"\nbwe_mode = \"remb\"\n").is_err());
    }
}
