// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-SUB-SERVER-SCOPE 정:RULE-SUB-STATE 정:IMPL-SUB-READY-GATE 정:IMPL-SUB-PIPELINE 정:IMPL-PUB-RID-SPATIAL 정:RULE-CONC-SIX 연:RULE-SUB-TGATE 연:RULE-SUB-READY-IDEMPOTENT 연:RULE-SUB-LAYER-PROC 연:SUB.S.ready · model: claude-opus-5-5
//! 구독 — 배정표(`table`) · egress 확장 다시 짓기(`ext`) · (user, 방) READY 게이트(5s 안전망 · 첫 READY 시각) · 레이어 상한.

pub mod ext;
pub mod table;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

pub const T_GATE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Created,
    Active,
}

pub struct Gate {
    gated: AtomicBool,
    pub since: AtomicU64,
    pub ready_at: AtomicU64,
    timed_out: AtomicBool,
}

pub enum Ready {
    First,
    Again,
}

impl Gate {
    pub fn new(now: u64) -> Gate {
        Gate { gated: AtomicBool::new(true), since: AtomicU64::new(now), ready_at: AtomicU64::new(0), timed_out: AtomicBool::new(false) }
    }

    pub fn open(&self) -> bool {
        !self.gated.load(Ordering::Acquire)
    }

    pub fn ready(&self, now: u64) -> Ready {
        self.gated.store(false, Ordering::Release);
        match self.ready_at.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => Ready::First,
            Err(_) => Ready::Again,
        }
    }

    pub fn timeout(&self, now: u64) -> bool {
        if self.open() || now.saturating_sub(self.since.load(Ordering::Acquire)) < T_GATE_MS {
            return false;
        }
        self.gated.store(false, Ordering::Release);
        !self.timed_out.swap(true, Ordering::AcqRel)
    }

    pub fn state(&self) -> State {
        if self.ready_at.load(Ordering::Acquire) != 0 { State::Active } else { State::Created }
    }
}

pub struct Prefs {
    pub spatial: AtomicU8,
    pub temporal: AtomicU8,
    pub paused: AtomicBool,
    pub priority: AtomicU8,
}

pub const MAX_SPATIAL: u8 = 1;
pub const MAX_TEMPORAL: u8 = 0;

impl Default for Prefs {
    fn default() -> Self {
        Prefs { spatial: AtomicU8::new(MAX_SPATIAL), temporal: AtomicU8::new(MAX_TEMPORAL), paused: AtomicBool::new(false), priority: AtomicU8::new(0) }
    }
}

impl Prefs {
    fn clamp_to(v: u8, max: u8) -> u8 {
        v.min(max)
    }

    pub fn clamp_spatial(v: u8) -> u8 {
        Self::clamp_to(v, MAX_SPATIAL)
    }

    pub fn clamp_temporal(v: u8) -> u8 {
        Self::clamp_to(v, MAX_TEMPORAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_at_only_first() {
        let g = Gate::new(0);
        assert!(!g.open());
        assert!(matches!(g.ready(10), Ready::First));
        assert!(matches!(g.ready(20), Ready::Again), "연:RULE-SUB-READY-IDEMPOTENT");
        assert_eq!(g.ready_at.load(Ordering::Acquire), 10, "정체 시작점은 첫 READY 만 찍는다");
    }

    #[test]
    fn tgate_latch_once() {
        let g = Gate::new(0);
        assert!(!g.timeout(4_999));
        assert!(g.timeout(5_000), "T-gate 5s 안전망");
        assert!(!g.timeout(9_000), "원샷 latch");
        assert!(g.open());
    }

    #[test]
    fn layer_caps_clamp() {
        assert_eq!(Prefs::clamp_spatial(5), 1);
        assert_eq!(Prefs::clamp_temporal(3), 0);
    }
}
