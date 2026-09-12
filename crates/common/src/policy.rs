// author: kodeholic (powered by Claude)
// spec: v1.1 · 정책서 §2 · §3 · 정§18-1 · model: claude-opus-5

//! 정책 파일 — hub·sfud **공통**. ★**값이 같아야 하는 것**이 여기 산다(프로세스마다 달라야 하면 인자다).
//!
//! ★**모르는 키는 기동 실패다.** 오타를 조용히 넘기면 *"바꿨는데 왜 안 먹나"* 만 남는다.
//! ★**부팅 1회 로드** — 런타임 리로드를 두지 않는다(재기동이 반영 경로이고, 그 대가로 경쟁이 없다).

use serde::Deserialize;

/// 값이 범위 밖이거나 모르는 키가 있다 — ★**기동을 세운다.**
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
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Logging {
    pub level: Level,
    /// ★**날짜 경계와 이 크기 둘 다** 본다 — 쓰는 순간 판정이라 둘 다 볼 수 있다.
    pub rotate_max_bytes: u64,
    /// `0` = 안 지움. ★**회전으로 닫힌 파일만** 센다.
    pub retention_days: u16,
}

impl Default for Logging {
    fn default() -> Self {
        Self { level: Level::Info, rotate_max_bytes: 268_435_456, retention_days: 14 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BweMode {
    Twcc,
    Remb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoLayer {
    Off,
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Media {
    pub bwe_mode: BweMode,
    /// 받기 m-section 예산의 ★**고정 분할** — 무전 슬롯 몫을 먼저 뗀다.
    pub slot_mid_reserve: u16,
    /// ★**키 부재 = 미설정 사이트** — 명시 opt-in 이다.
    pub auto_layer: AutoLayer,
    pub max_bitrate_bps: u32,
}

impl Default for Media {
    fn default() -> Self {
        Self {
            bwe_mode: BweMode::Twcc,
            slot_mid_reserve: 40,
            auto_layer: AutoLayer::Off,
            max_bitrate_bps: 800_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Floor {
    /// 넘치면 `DENY(7)`.
    pub queue_max: u8,
    /// ★정본은 연§8-4 `T2` — 여기는 손잡이다.
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
    pub heartbeat_interval_ms: u32,
    pub heartbeat_timeout_ms: u32,
    /// ★**백오프 합계(44~52s)보다 길어야 한다** — 짧으면 정상 재접속이 세션을 잃는다.
    pub resume_window_ms: u32,
    /// 범위 1~10 은 wire 가 정하고, 그 안에서 고르는 값이 정책이다.
    pub ws_flow_window: u8,
    pub token_ttl_secs: u32,
    pub room_unused_ttl_secs: u32,
    pub room_departure_ttl_secs: u32,
    pub room_default_capacity: u32,
    /// 토큰 `metadata` 직렬화 길이 상한 — 넘으면 발급이 `1002` 다.
    pub metadata_max_bytes: u32,
    /// ★`["*"]` = 전부 · ★**빈 목록 = CORS 헤더 없음**(브라우저 클라를 막는다).
    pub allowed_origins: Vec<String>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            heartbeat_interval_ms: 10_000,
            heartbeat_timeout_ms: 30_000,
            resume_window_ms: 60_000,
            ws_flow_window: 8,
            token_ttl_secs: 3_600,
            room_unused_ttl_secs: 300,
            room_departure_ttl_secs: 60,
            room_default_capacity: 1_000,
            metadata_max_bytes: 2_048,
            allowed_origins: vec!["*".to_string()],
        }
    }
}

/// ★**`0` = 상한 없음**(검사하지 않는다). 이 배포 전체의 상한이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Quota {
    pub max_sessions: u32,
    pub max_rooms: u32,
}

/// 대외 발행(webhook) — ★**hub 가 낸다.**
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Hook {
    pub retry_backoff_ms: Vec<u32>,
    /// 넘으면 ★**버리고 센다**(조용한 drop 금지).
    pub retry_max: u8,
    /// ★**3rd 마다 따로다.** 넘치면 오래된 것부터 버리고 센다.
    pub queue_max: u32,
}

impl Default for Hook {
    fn default() -> Self {
        Self { retry_backoff_ms: vec![1_000, 2_000, 4_000, 8_000, 16_000], retry_max: 5, queue_max: 10_000 }
    }
}

impl Policy {
    /// 읽고 ★**범위까지 본다.** 형만 맞고 값이 밖이면 기동을 세운다.
    pub fn parse(text: &str) -> Result<Policy, PolicyError> {
        let p: Policy = toml::from_str(text).map_err(|e| PolicyError(e.to_string()))?;
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.logging.rotate_max_bytes < 1024 * 1024 {
            return bad(format!(
                "logging.rotate_max_bytes {} 는 1MiB 미만 — 회전이 쓰는 족족 일어난다",
                self.logging.rotate_max_bytes
            ));
        }
        if !(1..=10).contains(&self.hub.ws_flow_window) {
            return bad(format!("hub.ws_flow_window {} 는 1~10 밖(연§3-2)", self.hub.ws_flow_window));
        }
        if self.hub.heartbeat_timeout_ms < 2 * self.hub.heartbeat_interval_ms {
            return bad("hub.heartbeat_timeout_ms 는 interval 의 2배 이상이어야 한다".to_string());
        }
        // ★백오프 합계(연§8-2 44~52s)보다 짧으면 정상 재접속이 세션을 잃는다.
        if self.hub.resume_window_ms < 52_000 {
            return bad(format!(
                "hub.resume_window_ms {} 는 백오프 합계(52,000)보다 짧다 — 재접속이 이어받지 못한다",
                self.hub.resume_window_ms
            ));
        }
        if !(100_000..=5_000_000).contains(&self.media.max_bitrate_bps) {
            return bad(format!("media.max_bitrate_bps {} 는 범위 밖", self.media.max_bitrate_bps));
        }
        if self.media.slot_mid_reserve > 128 {
            return bad(format!("media.slot_mid_reserve {} 는 0~128 밖", self.media.slot_mid_reserve));
        }
        // ★`v2` 는 `twcc` 전제다(정§10-2) — 어기면 기동 실패.
        if self.media.auto_layer == AutoLayer::V2 && self.media.bwe_mode != BweMode::Twcc {
            return bad("media.auto_layer=v2 는 bwe_mode=twcc 전제다(정§10-2)".to_string());
        }
        if self.floor.queue_max == 0 {
            return bad("floor.queue_max 는 1 이상이어야 한다".to_string());
        }
        if !(5..=600).contains(&self.floor.t2_stop_talking_secs) {
            return bad(format!("floor.t2_stop_talking_secs {} 는 5~600 밖", self.floor.t2_stop_talking_secs));
        }
        if self.hook.retry_max as usize > self.hook.retry_backoff_ms.len() {
            return bad("hook.retry_max 가 backoff 배열보다 길다 — 마지막 칸을 반복할지가 안 정해졌다".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 빈_파일은_기본값이다() {
        let p = Policy::parse("").expect("parse");
        assert_eq!(p.logging.level, Level::Info);
        assert_eq!(p.media.auto_layer, AutoLayer::Off, "★키 부재 = 미설정 사이트");
        assert_eq!(p.quota.max_sessions, 0, "★0 = 상한 없음");
    }

    #[test]
    fn 모르는_키는_기동_실패다() {
        // ★오타를 조용히 넘기면 "바꿨는데 왜 안 먹나" 만 남는다.
        let e = Policy::parse("[hub]\nheartbeat_interval_mss = 1000\n").unwrap_err();
        assert!(e.0.contains("heartbeat_interval_mss"), "{}", e.0);
    }

    #[test]
    fn v2_는_twcc_전제다() {
        let e = Policy::parse("[media]\nauto_layer = \"v2\"\nbwe_mode = \"remb\"\n").unwrap_err();
        assert!(e.0.contains("twcc"), "{}", e.0);
    }

    #[test]
    fn 창이_백오프보다_짧으면_선다() {
        let e = Policy::parse("[hub]\nresume_window_ms = 30000\n").unwrap_err();
        assert!(e.0.contains("백오프"), "{}", e.0);
    }

    #[test]
    fn 윈도우는_wire_범위_안이다() {
        assert!(Policy::parse("[hub]\nws_flow_window = 11\n").is_err());
        assert!(Policy::parse("[hub]\nws_flow_window = 10\n").is_ok());
    }

    #[test]
    fn 재시도_상한은_배열을_넘지_않는다() {
        let e = Policy::parse("[hook]\nretry_max = 9\n").unwrap_err();
        assert!(e.0.contains("backoff"), "{}", e.0);
    }
}
