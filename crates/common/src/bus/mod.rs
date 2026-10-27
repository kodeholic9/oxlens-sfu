// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-REQUEST · 정:RULE-BUS-EVENTS · model: claude-opus-5-5
//! B 평면의 공용 — 키 공간 · 배치 해시(순수) · 봉투(node 안 Handle 과 node 사이 `q/node` 가 같은 형) · zenoh 설정.

pub mod env;
pub mod keys;
pub mod place;
pub mod zcfg;

pub const REQUEST_TIMEOUT_MS: u64 = 10_000;
pub const SESS_GET_TIMEOUT_MS: u64 = 2_000;
pub const SETTLE_MS: u64 = 1_000;
pub const SHUTDOWN_LEAVE_WAIT_MS: u64 = 200;
