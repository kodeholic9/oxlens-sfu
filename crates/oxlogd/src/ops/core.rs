// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:RULE-CONC-SIX · 운영 규격서 §7-1 · model: claude-opus-5-5
//! 로거의 판정(순수) — 회전(날짜 경계 ∨ 크기 상한, 쓰는 순간) · 보존(회전으로 닫힌 파일과 spool 만 센다).

pub const ACTIVE_PREFIX: &str = "ox.log.";
pub const SPOOL_SUFFIX: &str = ".spool.log";
pub const DAY_SECS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotate {
    No,
    Date,
    Size,
}

pub fn rotate(open_date: &str, now_date: &str, size: u64, add: u64, max: u64) -> Rotate {
    if open_date != now_date {
        Rotate::Date
    } else if size > 0 && size + add > max {
        Rotate::Size
    } else {
        Rotate::No
    }
}

pub fn active_name(date: &str) -> String {
    format!("{ACTIVE_PREFIX}{date}")
}

pub fn closed_name(date: &str, n: u32) -> String {
    format!("{ACTIVE_PREFIX}{date}.{n}")
}

pub fn counted(name: &str, active: &str) -> bool {
    name != active && (name.starts_with(ACTIVE_PREFIX) || name.ends_with(SPOOL_SUFFIX))
}

pub fn expired(mtime_s: u64, now_s: u64, retention_days: u16) -> bool {
    retention_days > 0 && now_s.saturating_sub(mtime_s) > u64::from(retention_days) * DAY_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_and_retention() {
        assert_eq!(rotate("2026-10-24", "2026-10-25", 10, 1, 100), Rotate::Date);
        assert_eq!(rotate("2026-10-24", "2026-10-24", 99, 2, 100), Rotate::Size);
        assert_eq!(rotate("2026-10-24", "2026-10-24", 0, 200, 100), Rotate::No);
        assert!(counted("ox.log.2026-10-23", "ox.log.2026-10-24"));
        assert!(counted("node-a.sfu.spool.log", "ox.log.2026-10-24"));
        assert!(!counted("ox.log.2026-10-24", "ox.log.2026-10-24"));
        assert!(expired(0, 15 * DAY_SECS, 14));
        assert!(!expired(0, 15 * DAY_SECS, 0));
    }
}
