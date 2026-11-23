// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-fable-5-2
//! 맹검 독립 1층 — v1 층 판단 상수의 성립 조건(재강등 창 < backoff) · AIMD 벡터를 시각 인자로 그대로 넣는다 · 같은 입력 같은 출력.

use oxsfud::sim::core::{Aimd, Trendline, Usage};
use oxsfud::sim::core::{P_BACKOFF_MS as BACKOFF_MS, P_COLLAPSE_MS as COLLAPSE_MS};
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    let spec = match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec"),
    };
    spec.join("server/out/vectors")
}

fn load(name: &str) -> serde_json::Value {
    let p = vectors_dir().join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("vector file {} unreadable: {e}", p.display()));
    serde_json::from_str(&text).expect("vector json")
}

#[test]
fn collapse_window_is_shorter_than_the_backoff() {
    assert_eq!(COLLAPSE_MS, 5_000, "S-sym-RULE-SIM-TICK-1 re-demote (collapse) window 5,000 ms");
    assert_eq!(BACKOFF_MS, 15_000, "S-sym-RULE-SIM-TICK-1 first promotion backoff 15,000 ms");
    let gap = std::hint::black_box(BACKOFF_MS).saturating_sub(COLLAPSE_MS);
    assert!(gap > 0, "S-sym-RULE-SIM-TICK-1 the backoff must outlast the collapse window or every promotion re-demotes inside it");
}

#[test]
fn aimd_matches_vectors_with_time_argument() {
    let v = load("sim.json");
    for case in v["aimd"].as_array().expect("aimd") {
        let name = case["name"].as_str().unwrap_or("");
        let mut a = Aimd::new();
        if let Some(start) = case["start_bps"].as_i64() {
            a.set_start(start);
        }
        for st in case["steps"].as_array().expect("steps") {
            let at = st["at_us"].as_i64().expect("at");
            match st["probe_bps"].as_i64() {
                Some(p) => a.set_estimate(p, at),
                None => {
                    let usage = Usage::parse(st["usage"].as_str().expect("usage")).expect("usage name");
                    a.update(at, usage, st["throughput_bps"].as_i64());
                }
            }
            assert_eq!(a.rate, st["out_bps"].as_i64().expect("out"), "S-sym-RULE-CONC-SANS-IO-1 AIMD rate from the at_us argument · {name} · at {at}");
            assert_eq!(a.near(), st["out_near"].as_bool().expect("near"), "S-sym-RULE-CONC-SANS-IO-1 AIMD link estimate presence · {name} · at {at}");
        }
    }
}

#[test]
fn estimators_are_deterministic_in_their_arguments() {
    let run = || {
        let mut a = Aimd::new();
        a.set_start(500_000);
        let mut t = Trendline::new();
        let mut out = Vec::new();
        for i in 0..200i64 {
            let recv = 20.0 + (i % 7) as f64 * 0.8;
            let u = t.update(recv, 20.0, i as f64 * 20.0);
            out.push((u, a.update(i * 20_000, u, Some(400_000 + (i % 5) * 10_000))));
        }
        out
    };
    let first = run();
    std::thread::sleep(std::time::Duration::from_millis(25));
    assert_eq!(first, run(), "S-sym-RULE-CONC-SANS-IO-1 trendline and AIMD give the same output for the same arguments");
    let mut a = Aimd::new();
    let mut b = Aimd::new();
    a.set_start(300_000);
    b.set_start(300_000);
    a.update(0, Usage::Normal, Some(300_000));
    b.update(0, Usage::Normal, Some(300_000));
    let x = a.update(500_000, Usage::Normal, Some(300_000));
    let y = b.update(1_500_000, Usage::Normal, Some(300_000));
    assert!(y > x, "S-sym-RULE-CONC-SANS-IO-1 increase follows elapsed time taken from the argument ({x} at 500ms vs {y} at 1500ms)");
}

#[test]
fn a_probe_answers_no_more_than_it_asked() {
    use oxsfud::sim::bwe::{ProbeEstimator, Tag};
    let asked = Tag { id: 1, min_probes: 5, min_bytes: 3_375, bitrate: 1_800_000 };
    let mut burst = ProbeEstimator::default();
    let mut out = None;
    for (i, size) in [67i64, 439, 1_177, 1_177, 1_177, 1_177].iter().enumerate() {
        out = burst.handle(asked, 1_000_000 + i as i64 * 150, 2_000_000 + i as i64 * 120, *size);
    }
    let last = burst.last.expect("measured");
    assert!(last.send > 20 * asked.bitrate, "media forwarded in a burst leaves far faster than the cluster asked ({} bps)", last.send);
    assert_eq!(out, Some(asked.bitrate), "S-sym-RULE-SIM-TICK-22 a cluster that left faster than it asked still answers no more than the rate it asked");
    let wide = Tag { id: 2, min_probes: 5, min_bytes: 6_750, bitrate: 3_600_000 };
    let mut paced = ProbeEstimator::default();
    let mut out = None;
    for i in 0..16i64 {
        out = paced.handle(wide, 1_000_000 + i * 2_000, 2_000_000 + i * 2_000, 450);
    }
    assert_eq!(out, Some(1_800_000), "S-sym-RULE-SIM-TICK-22 a result below the asked rate is left as measured");
}
