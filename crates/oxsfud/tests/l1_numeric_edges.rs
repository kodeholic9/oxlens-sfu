// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-SIM-TICK · 정:RULE-FWD-MEDIA-WORKERS · model: claude-opus-5-5
//! 수치 경계 1층 — 대역 추정 코어(trendline · AIMD)가 0 간격 · 음수 간격 · 거대값 · 확인 비트레이트 0 과 최대 · 시계 역행에도 panic 하지 않고 NaN 을 퍼뜨리지 않으며 비트레이트가 바닥 위 유한값에 머문다(선례 mediasoup 3.11.21 Transport-CC 0 나누기 · 3.5.7 u64→i64 · context 202611/20261113f F2).

use oxsfud::sim::core::{Aimd, BWE_MIN, TREND_WINDOW, Trendline, Usage};

const EDGES: [f64; 9] = [0.0, -0.0, -1.0, 1e-12, -1e12, 1e12, f64::MAX, f64::MIN, 33.0];

#[test]
fn trendline_survives_edge_deltas_and_recovers() {
    let mut t = Trendline::new();
    let mut at = 0.0;
    for (i, r) in EDGES.iter().enumerate() {
        for s in EDGES {
            at += if i % 2 == 0 { 0.0 } else { 20.0 };
            let _ = t.update(*r, s, at);
        }
    }
    for k in 0..TREND_WINDOW * 3 {
        at += 20.0;
        let u = t.update(20.0, 20.0, at + k as f64);
        assert!(matches!(u, Usage::Normal | Usage::Overuse | Usage::Underuse));
    }
    let mut calm = Trendline::new();
    let mut a2 = 0.0;
    let mut last = Usage::Normal;
    for _ in 0..TREND_WINDOW * 3 {
        a2 += 20.0;
        last = calm.update(20.0, 20.0, a2);
    }
    assert_eq!(last, Usage::Normal, "고른 간격이면 정상 — 극단값 판의 비교 기준");
}

#[test]
fn aimd_stays_finite_above_floor_on_edge_inputs() {
    for start in [0i64, 1, 5_000, 300_000, i64::MAX] {
        let mut a = Aimd::new();
        a.set_start(start);
        a.rtt_us = 300_000;
        let mut now = 1_000_000i64;
        for acked in [None, Some(0i64), Some(1), Some(5_000), Some(2_000_000), Some(i64::MAX)] {
            for u in [Usage::Normal, Usage::Overuse, Usage::Underuse] {
                let bps = a.update(now, u, acked);
                assert!(bps >= BWE_MIN, "바닥({BWE_MIN}) 아래 — 시작 {start} · 확인 {acked:?} · {u:?} → {bps}");
                let _ = a.time_to_reduce_further(now, acked.unwrap_or(0));
                let _ = a.initial_time_to_reduce_further(now);
                now = now.wrapping_add(if acked.unwrap_or(0) % 2 == 0 { 0 } else { 50_000 });
            }
        }
        a.set_estimate(0, 0);
        let after = a.update(10_000, Usage::Overuse, Some(0));
        assert!(after >= BWE_MIN, "추정 0 · 확인 0 뒤에도 바닥({BWE_MIN}) 위 — 시작 {start} · 뒤 {after}");
    }
}

#[test]
fn aimd_clock_going_backwards_does_not_panic() {
    let mut a = Aimd::new();
    a.set_start(300_000);
    for now in [10_000_000i64, 5_000_000, 0, i64::MAX, 1, i64::MAX / 2, i64::MIN] {
        for u in [Usage::Overuse, Usage::Normal, Usage::Underuse] {
            let _ = a.update(now, u, Some(500_000));
        }
    }
    assert!(a.rate > 0);
}
