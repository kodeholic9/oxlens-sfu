// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 운영 규격서 §8-2 `--version` · 정:IMPL-ROOM-VERSION-ISSUE 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 빌드 신원(`{git SHA(+dirty)}.{빌드 시각}+{라벨}`)과 기동 신원(`epoch` = `sfu_id` = `{inst}`).

pub fn build_identity(sha: &str, built_at: &str, label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{sha}.{built_at}+{l}"),
        _ => format!("{sha}.{built_at}"),
    }
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if getrandom::fill(&mut buf).is_ok() {
        return buf;
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seed = t.to_le_bytes();
    (0..n).map(|i| seed[i % seed.len()] ^ (i as u8).wrapping_mul(31)).collect()
}

pub fn random_hex(n: usize) -> String {
    random_bytes(n).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_epoch() -> String {
    random_hex(8)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_form() {
        assert_eq!(build_identity("abc", "17", Some("oxe2epy")), "abc.17+oxe2epy");
        assert_eq!(build_identity("abc", "17", None), "abc.17");
        assert_eq!(new_epoch().len(), 16);
        assert_ne!(new_epoch(), new_epoch());
    }
}
