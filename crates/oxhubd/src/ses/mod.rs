// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-SES-GATEWAY-SCOPE · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! SES 영역 — 게이트웨이가 세션을 무엇으로 들고 어떻게 이행하나(core 판정 · token · shell I/O · HTTP 입구).

pub mod core;
pub mod http;
pub mod shell;
pub mod token;

pub use shell::Ses;
