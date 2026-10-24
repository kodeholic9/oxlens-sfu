// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:CAND-OPS-BUILD-DIRTY-MARK · 정:IMPL-ROOM-VERSION-ISSUE · 운영 규격서 §8-2 · model: claude-opus-5-5
//! 빌드 신원 · 기동 신원 — 빌드는 `{sha12}{-dirty}.{unix 초}[+라벨]` · 기동 신원(epoch)은 프로세스마다 새 불투명 값.

use sha2::{Digest, Sha256};

pub const BUILD_ID: &str = env!("OX_BUILD_ID");

pub fn build(label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{BUILD_ID}+{l}"),
        _ => BUILD_ID.to_string(),
    }
}

pub fn fresh_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let mut h = Sha256::new();
        h.update(now.to_be_bytes());
        h.update(std::process::id().to_be_bytes());
        let d = h.finalize();
        for (i, b) in buf.iter_mut().enumerate() {
            let v = d[i % d.len()];
            let _ = std::mem::replace(b, v);
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn epoch() -> String {
    format!("sfu-{}", fresh_hex(8))
}

pub fn fingerprint(session_id: &str) -> String {
    let d = Sha256::digest(session_id.as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities() {
        assert!(build(Some("dev")).ends_with("+dev"));
        assert_ne!(epoch(), epoch());
        assert_eq!(fingerprint("abc"), "ba7816bf");
        assert_eq!(fresh_hex(11).len(), 22);
    }
}
