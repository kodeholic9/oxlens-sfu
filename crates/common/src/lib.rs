// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CFG-THREE-PLACES · 정:RULE-DOC-CODE-MIRROR · 정:CAND-DOC-GUARD-PLACE · model: claude-opus-5-5
//! 공용 — 값을 두는 곳 셋 · 빌드/기동 신원 · 시계 · 로깅 · 토큰 · B 평면 · 부모 생존 · 규격 위치.

pub mod args;
pub mod bus;
pub mod identity;
pub mod lifeline;
pub mod log;
pub mod policy;
pub mod system;
pub mod time;
pub mod token;

pub const SPEC_ENV: &str = "OXLENS_SPEC";

pub fn spec_root() -> std::path::PathBuf {
    match std::env::var(SPEC_ENV) {
        Ok(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    }
}
