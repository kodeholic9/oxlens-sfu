// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CFG-THREE-PLACES · 정책:logging · 정책:media · 정책:floor · 정책:hub · 정책:quota · 정책:hook · model: claude-opus-5-5
//! 정책 파일 — hub · sfud 공통. 키 · 기본값 · 범위 · 교차 규칙은 정책서 생성본(`policy/out/policy.rs`) 그대로 · 모르는 키 · 범위 밖은 기동 실패.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyError(pub String);
impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "policy: {}", self.0)
    }
}
impl std::error::Error for PolicyError {}
fn bad<T>(msg: impl Into<String>) -> Result<T, PolicyError> {
    Err(PolicyError(msg.into()))
}

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
        match s {
            "trace" => Some(LoggingLevel::Trace),
            "debug" => Some(LoggingLevel::Debug),
            "info" => Some(LoggingLevel::Info),
            "warn" => Some(LoggingLevel::Warn),
            "error" => Some(LoggingLevel::Error),
            _ => None,
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            LoggingLevel::Trace => 0,
            LoggingLevel::Debug => 1,
            LoggingLevel::Info => 2,
            LoggingLevel::Warn => 3,
            LoggingLevel::Error => 4,
        }
    }
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
            stat_interval_ms: 5_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaBwemode {
    Twcc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaAutolayer {
    Off,
    V1,
    V2,
}

impl MediaAutolayer {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaAutolayer::Off => "off",
            MediaAutolayer::V1 => "v1",
            MediaAutolayer::V2 => "v2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Media {
    pub bwe_mode: MediaBwemode,
    pub slot_mid_reserve: u16,
    pub auto_layer: MediaAutolayer,
    pub max_bitrate_bps: u32,
    pub pli_throttle_high_ms: u32,
    pub pli_throttle_low_ms: u32,
}
impl Default for Media {
    fn default() -> Self {
        Self {
            bwe_mode: MediaBwemode::Twcc,
            slot_mid_reserve: 40,
            auto_layer: MediaAutolayer::Off,
            max_bitrate_bps: 2_000_000,
            pli_throttle_high_ms: 1_000,
            pli_throttle_low_ms: 500,
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
        Self { queue_max: 10, t2_stop_talking_secs: 30 }
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
        Self { retry_backoff_ms: vec![1000, 2000, 4000, 8000, 16000], retry_max: 5, queue_max: 10_000 }
    }
}

impl Policy {
    pub fn parse(text: &str) -> Result<Policy, PolicyError> {
        let p: Policy = toml::from_str(text).map_err(|e| PolicyError(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }

    pub fn load(path: &std::path::Path) -> Result<Policy, PolicyError> {
        let text = std::fs::read_to_string(path).map_err(|e| PolicyError(format!("{}: {e}", path.display())))?;
        Policy::parse(&text)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.hub.token_ttl_secs < 60 {
            return bad(format!("hub.token_ttl_secs {} 는 60 미만", self.hub.token_ttl_secs));
        }
        if !(1000..=60000).contains(&self.hub.heartbeat_interval_ms) {
            return bad(format!("hub.heartbeat_interval_ms {} 는 1000~60000 밖", self.hub.heartbeat_interval_ms));
        }
        if self.hub.heartbeat_timeout_ms < 2000 {
            return bad(format!("hub.heartbeat_timeout_ms {} 는 2000 미만", self.hub.heartbeat_timeout_ms));
        }
        if self.hub.resume_window_ms < 52000 {
            return bad(format!("hub.resume_window_ms {} 는 52000 미만", self.hub.resume_window_ms));
        }
        if !(1..=10).contains(&self.hub.ws_flow_window) {
            return bad(format!("hub.ws_flow_window {} 는 1~10 밖", self.hub.ws_flow_window));
        }
        if self.hub.room_default_capacity < 1 {
            return bad(format!("hub.room_default_capacity {} 는 1 미만", self.hub.room_default_capacity));
        }
        if self.hub.metadata_max_bytes < 256 {
            return bad(format!("hub.metadata_max_bytes {} 는 256 미만", self.hub.metadata_max_bytes));
        }
        if self.hub.join_retry_after_ms < 100 {
            return bad(format!("hub.join_retry_after_ms {} 는 100 미만", self.hub.join_retry_after_ms));
        }
        if self.logging.rotate_max_bytes < 1048576 {
            return bad(format!("logging.rotate_max_bytes {} 는 1048576 미만", self.logging.rotate_max_bytes));
        }
        if !(256..=2047).contains(&self.logging.line_max_bytes) {
            return bad(format!("logging.line_max_bytes {} 는 256~2047 밖", self.logging.line_max_bytes));
        }
        if self.logging.queue_max_lines < 256 {
            return bad(format!("logging.queue_max_lines {} 는 256 미만", self.logging.queue_max_lines));
        }
        if self.logging.spool_max_bytes < 1048576 {
            return bad(format!("logging.spool_max_bytes {} 는 1048576 미만", self.logging.spool_max_bytes));
        }
        if self.logging.socket_rcvbuf_bytes < 65536 {
            return bad(format!("logging.socket_rcvbuf_bytes {} 는 65536 미만", self.logging.socket_rcvbuf_bytes));
        }
        if !(200..=60000).contains(&self.logging.stat_interval_ms) {
            return bad(format!("logging.stat_interval_ms {} 는 200~60000 밖", self.logging.stat_interval_ms));
        }
        if self.media.slot_mid_reserve > 128 {
            return bad(format!("media.slot_mid_reserve {} 는 0~128 밖", self.media.slot_mid_reserve));
        }
        if !(100000..=5000000).contains(&self.media.max_bitrate_bps) {
            return bad(format!("media.max_bitrate_bps {} 는 100000~5000000 밖", self.media.max_bitrate_bps));
        }
        if !(300..=10000).contains(&self.media.pli_throttle_high_ms) {
            return bad(format!("media.pli_throttle_high_ms {} 는 300~10000 밖", self.media.pli_throttle_high_ms));
        }
        if !(300..=10000).contains(&self.media.pli_throttle_low_ms) {
            return bad(format!("media.pli_throttle_low_ms {} 는 300~10000 밖", self.media.pli_throttle_low_ms));
        }
        if self.floor.queue_max < 1 {
            return bad(format!("floor.queue_max {} 는 1~255 밖", self.floor.queue_max));
        }
        if !(5..=600).contains(&self.floor.t2_stop_talking_secs) {
            return bad(format!("floor.t2_stop_talking_secs {} 는 5~600 밖", self.floor.t2_stop_talking_secs));
        }
        if self.hook.queue_max < 1 {
            return bad(format!("hook.queue_max {} 는 1 미만", self.hook.queue_max));
        }
        if self.hub.heartbeat_timeout_ms < 2 * self.hub.heartbeat_interval_ms {
            return bad("hb-timeout: hub.heartbeat_timeout_ms ≥ 2 × hub.heartbeat_interval_ms");
        }
        if self.media.auto_layer == MediaAutolayer::V2 && self.media.bwe_mode != MediaBwemode::Twcc {
            return bad("v2-twcc: media.auto_layer = v2 면 media.bwe_mode = twcc");
        }
        if self.media.max_bitrate_bps < 1_900_000 {
            return bad("bitrate-layers: media.max_bitrate_bps ≥ 1,900,000");
        }
        if self.hook.retry_max as usize > self.hook.retry_backoff_ms.len() {
            return bad("hook-retry: hook.retry_max ≤ hook.retry_backoff_ms 길이");
        }
        Ok(())
    }

    pub fn banner_fields_sfu(&self) -> Vec<(String, String)> {
        let h = &self.hub;
        let mut v = vec![
            ("hub.room_unused_ttl_secs".to_string(), h.room_unused_ttl_secs.to_string()),
            ("hub.room_departure_ttl_secs".to_string(), h.room_departure_ttl_secs.to_string()),
            ("hub.room_default_capacity".to_string(), h.room_default_capacity.to_string()),
        ];
        v.extend(self.logging_fields());
        v.extend(self.media_fields());
        v.push(("floor.queue_max".into(), self.floor.queue_max.to_string()));
        v.push(("floor.t2_stop_talking_secs".into(), self.floor.t2_stop_talking_secs.to_string()));
        v
    }

    pub fn banner_fields_hub(&self) -> Vec<(String, String)> {
        let mut v = self.logging_fields();
        let h = &self.hub;
        for (k, x) in [
            ("hub.token_ttl_secs", h.token_ttl_secs.to_string()),
            ("hub.heartbeat_interval_ms", h.heartbeat_interval_ms.to_string()),
            ("hub.heartbeat_timeout_ms", h.heartbeat_timeout_ms.to_string()),
            ("hub.resume_window_ms", h.resume_window_ms.to_string()),
            ("hub.ws_flow_window", h.ws_flow_window.to_string()),
            ("hub.room_unused_ttl_secs", h.room_unused_ttl_secs.to_string()),
            ("hub.room_departure_ttl_secs", h.room_departure_ttl_secs.to_string()),
            ("hub.room_default_capacity", h.room_default_capacity.to_string()),
            ("hub.metadata_max_bytes", h.metadata_max_bytes.to_string()),
            ("hub.join_admit_per_sec", h.join_admit_per_sec.to_string()),
            ("hub.join_retry_after_ms", h.join_retry_after_ms.to_string()),
            ("hub.allowed_origins", h.allowed_origins.join(",")),
            ("quota.max_sessions", self.quota.max_sessions.to_string()),
            ("quota.max_rooms", self.quota.max_rooms.to_string()),
            (
                "hook.retry_backoff_ms",
                self.hook.retry_backoff_ms.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
            ),
            ("hook.retry_max", self.hook.retry_max.to_string()),
            ("hook.queue_max", self.hook.queue_max.to_string()),
        ] {
            v.push((k.to_string(), x));
        }
        v
    }

    fn logging_fields(&self) -> Vec<(String, String)> {
        let l = &self.logging;
        vec![
            ("logging.level".into(), l.level.as_str().into()),
            ("logging.rotate_max_bytes".into(), l.rotate_max_bytes.to_string()),
            ("logging.retention_days".into(), l.retention_days.to_string()),
            ("logging.line_max_bytes".into(), l.line_max_bytes.to_string()),
            ("logging.queue_max_lines".into(), l.queue_max_lines.to_string()),
            ("logging.spool_max_bytes".into(), l.spool_max_bytes.to_string()),
            ("logging.socket_rcvbuf_bytes".into(), l.socket_rcvbuf_bytes.to_string()),
            ("logging.stat_interval_ms".into(), l.stat_interval_ms.to_string()),
        ]
    }

    fn media_fields(&self) -> Vec<(String, String)> {
        let m = &self.media;
        vec![
            ("media.bwe_mode".into(), "twcc".into()),
            ("media.slot_mid_reserve".into(), m.slot_mid_reserve.to_string()),
            ("media.auto_layer".into(), m.auto_layer.as_str().into()),
            ("media.max_bitrate_bps".into(), m.max_bitrate_bps.to_string()),
            ("media.pli_throttle_high_ms".into(), m.pli_throttle_high_ms.to_string()),
            ("media.pli_throttle_low_ms".into(), m.pli_throttle_low_ms.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_valid_unknown_key_fails() {
        Policy::default().validate().unwrap();
        assert!(Policy::parse("[media]\nnope = 1\n").is_err());
        assert!(Policy::parse("[media]\nmax_bitrate_bps = 800000\n").is_err());
        let p = Policy::parse("[media]\nauto_layer = \"v2\"\n").unwrap();
        assert_eq!(p.media.auto_layer, MediaAutolayer::V2);
    }
}
