// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:LOG.log.LINE · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 시계 둘 — 프로세스 단조 시계(기동부터 µs)와 벽시계. 판정 함수는 이 값을 인자로 받는다.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

pub fn start() -> Instant {
    Instant::clone(START.get_or_init(Instant::now))
}

pub fn mono_us() -> u64 {
    start().elapsed().as_micros() as u64
}

pub fn mono_ms() -> u64 {
    start().elapsed().as_millis() as u64
}

pub fn wall_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn wall_secs() -> u64 {
    wall_ms() / 1000
}

pub fn iso_now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
