// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-OPS-LOGGER · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! 프로세스 사이 공통 — 값을 두는 곳(정책 · 시스템 파일 · 인자) · 버스 키 · node 안 Handle 봉투 · 빌드 · 기동 신원 · 시계 · 로깅.

pub mod args;
pub mod bus;
pub mod clock;
pub mod handle;
pub mod ident;
pub mod log;
pub mod policy;
pub mod system;
