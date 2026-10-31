// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 맹검 독립 1층 — 자동 층 상수의 성립 조건(유예 > 재강등 창) · AIMD 벡터를 시각 인자로 그대로 넣는다 · 같은 입력 같은 출력.

use oxsfud::sim::core::{Aimd, Trendline, Usage};
use oxsfud::sim::{COLLAPSE_MS, GRACE_MS};
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    let spec = match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    };
    spec.join("server/out/vectors")
}

fn load(name: &str) -> serde_json::Value {
    let p = vectors_dir().join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("vector file {} unreadable: {e}", p.display()));
    serde_json::from_str(&text).expect("vector json")
}

#[test]
fn grace_exceeds_collapse_window() {
    assert_eq!(COLLAPSE_MS, 5_000, "S-sym-RULE-SIM-TICK-1 re-demote (collapse) window 5,000 ms");
    assert_eq!(GRACE_MS, 10_000, "S-sym-RULE-SIM-TICK-1 post-promotion estimate grace 10,000 ms");
    let gap = std::hint::black_box(GRACE_MS).saturating_sub(COLLAPSE_MS);
    assert!(gap > 0, "S-sym-RULE-SIM-TICK-1 grace must be longer than the collapse window or capacity shortfall reads as collapse");
}

#[test]
fn aimd_matches_vectors_with_time_argument() {
    let v = load("sim.json");
    let rtt = v["constants"]["rtt_ms"].as_u64().expect("rtt");
    for case in v["aimd"].as_array().expect("aimd") {
        let name = case["name"].as_str().unwrap_or("");
        let mut a = Aimd::new(case["start_bps"].as_u64().expect("start"), rtt);
        for st in case["steps"].as_array().expect("steps") {
            let usage = Usage::parse(st["usage"].as_str().expect("usage")).expect("usage name");
            let out = a.update(st["at_ms"].as_u64().expect("at"), usage, st["acked_bps"].as_u64().expect("acked"));
            assert_eq!(out, st["out_bps"].as_u64().expect("out"), "S-sym-RULE-CONC-SANS-IO-1 AIMD rate from the at_ms argument · {name} · at {}", st["at_ms"]);
            assert_eq!(a.near(), st["out_near"].as_bool().expect("near"), "S-sym-RULE-CONC-SANS-IO-1 AIMD link estimate presence · {name} · at {}", st["at_ms"]);
        }
    }
}

#[test]
fn estimators_are_deterministic_in_their_arguments() {
    let run = || {
        let mut a = Aimd::new(500_000, 200);
        let mut t = Trendline::new();
        let mut out = Vec::new();
        for i in 0..200u64 {
            let recv = 20.0 + (i % 7) as f64 * 0.8;
            let u = t.update(recv, 20.0, i as f64 * 20.0);
            out.push((u, a.update(i * 20, u, 400_000 + (i % 5) * 10_000)));
        }
        out
    };
    let first = run();
    std::thread::sleep(std::time::Duration::from_millis(25));
    assert_eq!(first, run(), "S-sym-RULE-CONC-SANS-IO-1 trendline and AIMD give the same output for the same arguments");
    let mut a = Aimd::new(300_000, 200);
    let mut b = Aimd::new(300_000, 200);
    a.update(0, Usage::Normal, 300_000);
    b.update(0, Usage::Normal, 300_000);
    let x = a.update(500, Usage::Normal, 300_000);
    let y = b.update(1_500, Usage::Normal, 300_000);
    assert!(y > x, "S-sym-RULE-CONC-SANS-IO-1 increase follows elapsed time taken from the argument ({x} at 500ms vs {y} at 1500ms)");
}
