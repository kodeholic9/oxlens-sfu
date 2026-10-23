// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-LOGGER · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 로거 판정(순수) — 쓸 파일 이름 · 회전(날짜 경계 ∨ 크기) · 보존(닫힌 파일만 · 날 수).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    pub date: String,
    pub part: u32,
    pub size: u64,
}

pub fn file_name(date: &str, part: u32) -> String {
    if part == 0 {
        format!("ox.log.{date}")
    } else {
        format!("ox.log.{date}.{part}")
    }
}

pub fn next(cur: &Current, today: &str, incoming: u64, max_bytes: u64) -> Option<Current> {
    if cur.date != today {
        return Some(Current { date: today.to_string(), part: 0, size: 0 });
    }
    if cur.size > 0 && cur.size + incoming > max_bytes {
        return Some(Current { date: cur.date.clone(), part: cur.part + 1, size: 0 });
    }
    None
}

pub fn closed_log(name: &str, current: &str) -> bool {
    name != current && (name.starts_with("ox.log.") || name.ends_with(".spool.log"))
}

pub fn expired(age_secs: u64, retention_days: u16) -> bool {
    retention_days != 0 && age_secs > retention_days as u64 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_on_date_or_size() {
        let c = Current { date: "2026-10-24".into(), part: 0, size: 90 };
        assert_eq!(next(&c, "2026-10-25", 1, 100), Some(Current { date: "2026-10-25".into(), part: 0, size: 0 }));
        assert_eq!(next(&c, "2026-10-24", 20, 100), Some(Current { date: "2026-10-24".into(), part: 1, size: 0 }));
        assert_eq!(next(&c, "2026-10-24", 5, 100), None);
        assert_eq!(file_name("2026-10-24", 1), "ox.log.2026-10-24.1");
    }

    #[test]
    fn retention_counts_closed_files_only() {
        assert!(!closed_log("ox.log.2026-10-24", "ox.log.2026-10-24"));
        assert!(closed_log("ox.log.2026-10-23", "ox.log.2026-10-24"));
        assert!(closed_log("node-a.sfu.spool.log", "ox.log.2026-10-24"));
        assert!(!expired(10 * 86_400, 0));
        assert!(expired(15 * 86_400, 14));
    }
}
