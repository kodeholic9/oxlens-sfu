// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CONC-SIX · 운영 규격서 §7-3 · 운영 규격서 §3-9 · model: claude-opus-5-5
//! 시계 둘 — 벽시계 ms(관측 · 운영 출력의 시각 축) · 단조 시계(기동부터 — 순서 · 간격). 판정 함수는 시각을 인자로 받는다.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

pub fn start() -> Instant {
    START.get_or_init(Instant::now).to_owned()
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
