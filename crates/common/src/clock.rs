// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 시계 — 벽시계 ms(운영 표면의 시각 축) · 프로세스 단조 µs(로그 `mono`). 판정 함수는 이것을 부르지 않고 인자로 받는다.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

pub fn start() -> &'static Instant {
    START.get_or_init(Instant::now)
}

pub fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn wall_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

pub fn mono_us() -> u64 {
    start().elapsed().as_micros() as u64
}

pub fn mono_ms() -> u64 {
    start().elapsed().as_millis() as u64
}
