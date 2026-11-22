// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:IMPL-SIM-SWITCH · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 층 전략 — 배분 점(공간 × 시간 · TID ≤ t 누적 · 선호 순) · 공간 전환 키프레임을 기다리는 동안의 자동 시간 상한 · 점 번호. 원천 server/areas/sim/behavior/strategy.ts.

use common::policy::MediaLayerStrategy;

use crate::sim::alloc::{LAYER_NONE, Point};
use crate::sim::temporal::TL_TOP;

pub const TL_SLOTS: usize = TL_TOP as usize + 1;

pub fn point_code(s: u8, t: u8) -> i8 {
    (s as usize * TL_SLOTS + t as usize) as i8
}

pub fn point_of(code: i8) -> Option<(u8, u8)> {
    if code < 0 { None } else { Some(((code as usize / TL_SLOTS) as u8, (code as usize % TL_SLOTS) as u8)) }
}

pub fn layer_points(strategy: MediaLayerStrategy, raw: &[[i64; TL_SLOTS]], cap_s: u8, cap_t: u8) -> Vec<Point> {
    let mut out = vec![Point { bitrate: 0, layer: LAYER_NONE }];
    let t_top = cap_t.min(TL_TOP);
    let mut found = false;
    for (s, row) in raw.iter().enumerate().take(cap_s as usize + 1) {
        let start = out.len();
        let mut cum = 0i64;
        for t in 0..=t_top {
            let b = row[t as usize];
            if b <= 0 {
                break;
            }
            cum += b;
            let p = Point { bitrate: cum, layer: point_code(s as u8, t) };
            if strategy != MediaLayerStrategy::AllPoints && found && out.len() > start {
                out.pop();
            }
            out.push(p);
        }
        if out.len() > start {
            found = true;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitWhy {
    None,
    Hold,
    Halve1,
    Halve2,
}

impl WaitWhy {
    pub fn name(self) -> &'static str {
        match self {
            WaitWhy::None => "none",
            WaitWhy::Hold => "hold",
            WaitWhy::Halve1 => "halve1",
            WaitWhy::Halve2 => "halve2",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn wait_temporal(strategy: MediaLayerStrategy, layer: u8, target: u8, pause: u8, elapsed_ms: u64, window_ms: u64, current: u8, auto: u8) -> (u8, WaitWhy) {
    let waiting = layer != target && layer != pause && target != pause;
    if !waiting || strategy == MediaLayerStrategy::AllPoints {
        return (auto, WaitWhy::None);
    }
    if strategy == MediaLayerStrategy::HalveWait && target < layer {
        return if elapsed_ms >= window_ms { (0, WaitWhy::Halve2) } else { (current.min(1), WaitWhy::Halve1) };
    }
    (current, WaitWhy::Hold)
}
