// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:IMPL-SIM-SWITCH · model: claude-opus-5-5
//! 1층 — 층 전략(context 202611/20261122a S5). 벡터 sim.json `points` · `wait_temporal` · `point_decode` 를 제품 함수와 맞댄다 · 지금 점을 (공간, 시간) 쌍으로 찾는다 · 배분기가 값이 내려가는 점을 탄다 · 시험 단위 S-sym-RULE-SIM-TICK-24 · 25 · S-sym-IMPL-SIM-SWITCH-10.

use std::path::{Path, PathBuf};

use common::policy::MediaLayerStrategy;
use oxsfud::sim::alloc::{Allocator, Candidate, Point};
use oxsfud::sim::current_point;
use oxsfud::sim::strategy::{TL_SLOTS, WaitWhy, layer_points, point_code, point_of, wait_temporal};
use serde_json::Value;

fn spec() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn load(name: &str) -> Value {
    let p = spec().join("server/out/vectors").join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).expect("벡터 JSON")
}

fn strat(n: &str) -> MediaLayerStrategy {
    match n {
        "all_points" => MediaLayerStrategy::AllPoints,
        "lowest_temporal" => MediaLayerStrategy::LowestTemporal,
        "halve_wait" => MediaLayerStrategy::HalveWait,
        x => panic!("전략 {x}"),
    }
}

fn why(n: &str) -> WaitWhy {
    match n {
        "none" => WaitWhy::None,
        "hold" => WaitWhy::Hold,
        "halve1" => WaitWhy::Halve1,
        "halve2" => WaitWhy::Halve2,
        x => panic!("사유 {x}"),
    }
}

#[test]
fn points_vectors_match_product() {
    let v = load("sim.json");
    let cases = v["points"].as_array().expect("points");
    assert!(cases.len() >= 7);
    for c in cases {
        let raw: Vec<[i64; TL_SLOTS]> = c["raw"].as_array().unwrap().iter().map(|r| std::array::from_fn(|i| r[i].as_f64().unwrap() as i64)).collect();
        let (cs, ct) = (c["cap_s"].as_u64().unwrap() as u8, c["cap_t"].as_u64().unwrap() as u8);
        for (name, want) in c["out"].as_object().unwrap() {
            let got: Vec<(i64, i64)> = layer_points(strat(name), &raw, cs, ct).iter().map(|p| (p.bitrate, p.layer as i64)).collect();
            let want: Vec<(i64, i64)> = want.as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap() as i64, p[1].as_i64().unwrap())).collect();
            assert_eq!(got, want, "S-sym-RULE-SIM-TICK-24① ② ③ ④ ⑤ layer_points · {} · {name}", c["name"]);
        }
    }
}

#[test]
fn wait_temporal_vectors_match_product() {
    let v = load("sim.json");
    for c in v["wait_temporal"].as_array().expect("wait_temporal") {
        let g = |k: &str| c[k].as_u64().unwrap();
        for (name, want) in c["out"].as_object().unwrap() {
            let got = wait_temporal(strat(name), g("layer") as u8, g("target") as u8, g("pause") as u8, g("elapsed_ms"), g("window_ms"), g("current") as u8, g("auto") as u8);
            let w = (want[0].as_u64().unwrap() as u8, why(want[1].as_str().unwrap()));
            assert_eq!(got, w, "S-sym-IMPL-SIM-SWITCH-10①②③④ wait_temporal · {} · {name}", c["name"]);
        }
    }
    for p in v["point_decode"].as_array().unwrap() {
        let code = p[0].as_i64().unwrap() as i8;
        let want = p[1].as_array().map(|x| (x[0].as_u64().unwrap() as u8, x[1].as_u64().unwrap() as u8));
        assert_eq!(point_of(code), want, "정:RULE-SIM-TICK point_of {code}");
        if let Some((s, t)) = want {
            assert_eq!(point_code(s, t), code);
        }
    }
}

#[test]
fn the_current_point_is_found_by_spatial_and_temporal() {
    let raw = [[40_000, 40_000, 80_000], [112_500, 112_500, 225_000], [412_500, 412_500, 825_000]];
    let lt = layer_points(MediaLayerStrategy::LowestTemporal, &raw, 2, 2);
    assert_eq!(current_point(&lt, 0, 1), Some(2), "S-sym-RULE-SIM-TICK-25② (l, T1) is the third point");
    assert_eq!(current_point(&lt, 1, 2), Some(4), "S-sym-RULE-SIM-TICK-25② (m, T2)");
    assert_eq!(current_point(&lt, 2, 0), None, "정:RULE-SIM-TICK (h, T0) is not a point of lowest_temporal — no hysteresis anchor");
    let untagged = [[160_000, 0, 0], [450_000, 0, 0], [1_650_000, 0, 0]];
    let up = layer_points(MediaLayerStrategy::AllPoints, &untagged, 2, 2);
    assert_eq!(current_point(&up, 2, 2), Some(3), "S-sym-RULE-SIM-TICK-25② a stream without temporal marks still finds its spatial point (t ≤ auto)");
}

#[test]
fn the_allocator_climbs_through_a_cheaper_point_in_preference_order() {
    let raw = [[40_000, 40_000, 80_000], [112_500, 112_500, 225_000], [412_500, 412_500, 825_000]];
    let ap = layer_points(MediaLayerStrategy::AllPoints, &raw, 2, 2);
    assert!(ap.windows(2).any(|w| w[1].bitrate < w[0].bitrate), "S-sym-RULE-SIM-TICK-25③ all_points keeps preference order — (m, T0) is cheaper than (l, T2)");
    let mut a = Allocator::default();
    let cand = Candidate { id: 1, weight: 1, points: ap.clone(), current: None };
    let (g, _) = a.distribute(500_000, &[cand]);
    let chosen: &Point = &ap[g[0].point];
    assert_eq!(point_of(chosen.layer), Some((2, 0)), "S-sym-RULE-SIM-TICK-25③ all_points walks preference order by absolute price — 500 kbps passes (m, T2) 450k and lands on the cheaper later (h, T0) 412.5k (LiveKit property: resolution before fps)");
    let lt = layer_points(MediaLayerStrategy::LowestTemporal, &raw, 2, 2);
    let (g, _) = Allocator::default().distribute(500_000, &[Candidate { id: 1, weight: 1, points: lt.clone(), current: None }]);
    assert_eq!(point_of(lt[g[0].point].layer), Some((1, 2)), "S-sym-RULE-SIM-TICK-25③ lowest_temporal at 500 kbps — (m, T2): upper layers keep full fps");
}

fn row() -> oxsfud::fwd::DownRow {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let tr = oxsfud::link::Transport::new(9, "s", "", oxsfud::link::Role::Both, 0, false);
    let r = oxsfud::fwd::DownRow::create(tr, "1".into(), oxsig::msg::Kind::Video, false, 1, None, Arc::new(AtomicU64::new(9)));
    r.since.store(1, Ordering::Release);
    oxsfud::fwd::apply_row_cmd(&r, 1, true);
    r
}

#[test]
fn a_reassign_clears_the_automatic_temporal_cap_and_the_temporal_state() {
    use std::sync::atomic::Ordering;
    use oxsfud::sim::temporal::{TL_TOP, TemporalSel};
    let r = row();
    r.t_auto.store(0, Ordering::Release);
    let mut s = TemporalSel::default();
    let _ = s.step(oxsfud::sim::temporal::Tl { tid: 0, sync: 1, pid: 100 }, true, 0);
    r.tsel.store(s.pack(), Ordering::Release);
    r.since.store(2, Ordering::Release);
    assert!(oxsfud::fwd::apply_row_cmd(&r, 2, true));
    assert_eq!(r.t_auto.load(Ordering::Acquire), TL_TOP, "정:IMPL-SIM-SWITCH a reassigned row does not keep the old source's automatic temporal cap (Fable S5 H2)");
    assert_eq!(r.tsel.load(Ordering::Acquire), TemporalSel::default().pack(), "정:IMPL-SIM-SWITCH nor its temporal selection state");
}

#[test]
fn the_halve_second_step_waits_two_throttle_windows_of_the_target_layer() {
    let p = &oxsfud::ops::policy().media;
    assert_eq!(oxsfud::sim::wait_window_ms(0), 2 * p.pli_throttle_low_ms as u64, "S-sym-IMPL-SIM-SWITCH-10③ target l — the deferred PLI leaves at the window end and its keyframe gets one more window (Fable S5 M3)");
    assert_eq!(oxsfud::sim::wait_window_ms(2), 2 * p.pli_throttle_high_ms as u64, "S-sym-IMPL-SIM-SWITCH-10③ target m · h — high window");
}
