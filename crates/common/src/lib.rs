// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 정:RULE-BUS-SHAPE · 정:RULE-DOC-CODE-MIRROR · 운영:RULE-LOG-SENDER · model: claude-opus-5-5
//! 두 프로세스(hub · sfud)와 로거 · 운영 도구가 함께 쓰는 것 — 값의 자리 셋 · 버스 키와 봉투 · node 안 Handle · 로그 줄 · 빌드 신원 · 프로세스 수명.

pub mod args;
pub mod banner;
pub mod build;
pub mod bus;
pub mod grpc;
pub mod log;
pub mod policy;
pub mod proc;
pub mod system;
pub mod time;
