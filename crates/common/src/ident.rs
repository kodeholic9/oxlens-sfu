// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-BOOT-ARGS · 운영:BOOT.http.HEALTH_READY · 정:IMPL-ROOM-VERSION-ISSUE · model: claude-opus-5-5
//! 빌드 신원 `{SHA 12}{-dirty}.{빌드 시각}` (+ `+{라벨}`) · 기동 신원(프로세스 기동마다 새 불투명 값).

pub const COMPILED: &str = env!("OX_BUILD");

pub fn build_id(compiled: &str, label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{compiled}+{l}"),
        _ => compiled.to_string(),
    }
}

pub fn new_epoch() -> String {
    crate::random_hex(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_joined() {
        assert_eq!(build_id("abc123def456.17", Some("x")), "abc123def456.17+x");
        assert_eq!(build_id("abc123def456.17", None), "abc123def456.17");
        assert_eq!(new_epoch().len(), 16);
    }
}
