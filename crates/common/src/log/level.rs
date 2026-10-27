// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-LEVEL · 정:RULE-OPS-LOGGER · 정:CAND-OPS-LOG-ENV · 정:CAND-OPS-FOREIGN-LOG · model: claude-opus-5-5
//! 수준 — `diag` 줄만 거른다. 기동값 = 정책 `logging.level` · `OX_LOG_LEVEL` 이 덮는다 · 조사 중 일시 덮기(`ttl_secs` 만료로 되돌림).

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::policy::LoggingLevel;

pub const ENV: &str = "OX_LOG_LEVEL";

static BASE: AtomicU8 = AtomicU8::new(2);
static OVERRIDE: AtomicU8 = AtomicU8::new(u8::MAX);
static UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn code(l: LoggingLevel) -> u8 {
    match l {
        LoggingLevel::Trace => 0,
        LoggingLevel::Debug => 1,
        LoggingLevel::Info => 2,
        LoggingLevel::Warn => 3,
        LoggingLevel::Error => 4,
    }
}

fn level(c: u8) -> LoggingLevel {
    match c {
        0 => LoggingLevel::Trace,
        1 => LoggingLevel::Debug,
        2 => LoggingLevel::Info,
        3 => LoggingLevel::Warn,
        _ => LoggingLevel::Error,
    }
}

pub fn boot(policy_level: LoggingLevel) -> LoggingLevel {
    let l = std::env::var(ENV).ok().and_then(|v| LoggingLevel::parse(v.trim())).unwrap_or(policy_level);
    BASE.store(code(l), Ordering::Relaxed);
    l
}

pub fn set_override(l: LoggingLevel, ttl_secs: u64, now_ms: u64) {
    UNTIL_MS.store(now_ms.saturating_add(ttl_secs.saturating_mul(1000)), Ordering::Relaxed);
    OVERRIDE.store(code(l), Ordering::Relaxed);
}

pub fn clear_override() {
    OVERRIDE.store(u8::MAX, Ordering::Relaxed);
}

pub fn base() -> LoggingLevel {
    level(BASE.load(Ordering::Relaxed))
}

pub fn current_at(now_ms: u64) -> LoggingLevel {
    let o = OVERRIDE.load(Ordering::Relaxed);
    if o != u8::MAX {
        if now_ms < UNTIL_MS.load(Ordering::Relaxed) {
            return level(o);
        }
        clear_override();
    }
    base()
}

pub fn override_left_secs(now_ms: u64) -> Option<u64> {
    if OVERRIDE.load(Ordering::Relaxed) == u8::MAX {
        return None;
    }
    let until = UNTIL_MS.load(Ordering::Relaxed);
    (now_ms < until).then(|| (until - now_ms).div_ceil(1000))
}

pub fn allows(l: &tracing::Level, now_ms: u64) -> bool {
    let want = current_at(now_ms);
    let have = match *l {
        tracing::Level::TRACE => LoggingLevel::Trace,
        tracing::Level::DEBUG => LoggingLevel::Debug,
        tracing::Level::INFO => LoggingLevel::Info,
        tracing::Level::WARN => LoggingLevel::Warn,
        _ => LoggingLevel::Error,
    };
    have >= want
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_expires_back_to_base() {
        BASE.store(code(LoggingLevel::Info), Ordering::Relaxed);
        assert!(!allows(&tracing::Level::DEBUG, 0));
        set_override(LoggingLevel::Debug, 2, 1_000);
        assert!(allows(&tracing::Level::DEBUG, 2_000));
        assert_eq!(override_left_secs(2_000), Some(1));
        assert!(!allows(&tracing::Level::DEBUG, 3_000));
        assert_eq!(override_left_secs(3_000), None);
    }
}
