// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-CFG-THREE-PLACES 정:RULE-BUS-SHAPE 정:RULE-OPS-LOGGER 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! hub · sfud · 로거 공용 — 빌드 신원 · 정책 파일 · 시스템 파일 · 로그 보내는 쪽 · 버스 키와 배치 · node 안 Handle · 토큰 · 무작위 id.

pub mod bus;
pub mod handle;
pub mod id;
pub mod jwt;
pub mod log;
pub mod policy;
pub mod system;

pub const BUILD_ID: &str = env!("OX_BUILD_ID");

pub fn build_id(label: Option<&str>) -> String {
    match label {
        Some(l) if !l.is_empty() => format!("{BUILD_ID}+{l}"),
        _ => BUILD_ID.to_string(),
    }
}
