// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-BOOT-ARGS · 정:CAND-OPS-BUILD-DIRTY-MARK · model: claude-opus-5-5
//! 빌드 신원 — `{git SHA 앞 12자}{-dirty}.{빌드 시각 unix 초}` · `--build` 라벨이 있으면 `+{라벨}`.

pub const SHA: &str = env!("OX_BUILD_SHA");
pub const DIRTY: &str = env!("OX_BUILD_DIRTY");
pub const AT: &str = env!("OX_BUILD_AT");

pub fn identity(label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{SHA}{DIRTY}.{AT}+{l}"),
        _ => format!("{SHA}{DIRTY}.{AT}"),
    }
}

pub fn new_epoch() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    let mut s = String::with_capacity(16);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_shape() {
        let id = identity(Some("dev"));
        assert!(id.ends_with("+dev"));
        assert!(id.starts_with(SHA));
        assert_eq!(new_epoch().len(), 16);
        assert_ne!(new_epoch(), new_epoch());
    }
}
