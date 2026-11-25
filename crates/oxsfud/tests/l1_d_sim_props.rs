// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 1층 — 대역 추정 자취를 제품 추정기로 다시 돌려 지연 기울기 판정 · 지연 기반 속도의 성질을 본다(context 202611/20261124b 넷째 갈래). 자취는 원천이 지은 `server/out/vectors/sim.json` 의 `congestion` 판 입력 그대로이고, 판정은 제품 `Congestion` 이 낸다.

use std::path::{Path, PathBuf};

use oxsfud::sim::bwe::*;
use serde_json::Value;

fn spec() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec")
}

fn load(name: &str) -> Value {
    let p = spec().join("server/out/vectors").join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).expect("벡터 JSON")
}

struct Obs {
    op: String,
    us: i64,
    usage: String,
    aimd: i64,
}

fn replay(v: &Value, starts: &str) -> Vec<Obs> {
    let c = v["congestion"].as_array().expect("congestion").iter().find(|c| c["name"].as_str().unwrap_or("").starts_with(starts)).unwrap_or_else(|| panic!("자취 {starts} 없음"));
    let mut g: Option<Congestion> = None;
    let mut out = Vec::new();
    for st in c["steps"].as_array().expect("steps") {
        let us = st["us"].as_i64().unwrap_or(0);
        let op = st["op"].as_str().unwrap_or("");
        if op == "new" {
            g = Some(Congestion::new(st["start_bps"].as_i64().unwrap_or(0), us));
            continue;
        }
        let c = g.as_mut().expect("new first");
        match op {
            "sent" => c.on_sent(st["size"].as_i64().unwrap_or(0), us, us),
            "shot" => {
                let _ = c.prober.shot(us);
            }
            "media" => {
                let _ = c.prober.media(us, st["size"].as_i64().unwrap_or(0));
            }
            "twccless" => {
                let _ = c.twccless(us);
            }
            "padding" => {
                let _ = c.padding(us);
            }
            "rtt" => c.on_rtt(st["rtt_us"].as_i64().unwrap_or(0)),
            "shot_sent" => c.prober.sent(us, st["bytes"].as_i64().unwrap_or(0)),
            "network" => {
                let _ = c.set_network(true, us);
            }
            "desired" => {
                let _ = c.set_desired(st["bps"].as_i64().unwrap_or(0), us);
            }
            "process" => {
                let _ = c.process(us);
            }
            "feedback" => {
                let fed: Vec<Fed> = st["pk"]
                    .as_array()
                    .expect("pk")
                    .iter()
                    .map(|p| {
                        let id = p[4].as_i64().unwrap_or(0);
                        Fed {
                            seq: p[0].as_i64().unwrap_or(0),
                            send_us: p[1].as_i64().unwrap_or(0),
                            size: p[2].as_i64().unwrap_or(0),
                            recv_us: p[3].as_i64(),
                            tag: if id != 0 { Some(Tag { id, min_probes: p[5].as_i64().unwrap_or(0), min_bytes: p[6].as_i64().unwrap_or(0), bitrate: p[7].as_i64().unwrap_or(0) }) } else { None },
                        }
                    })
                    .collect();
                let _ = c.on_feedback(&fed, us, st["base_us"].as_i64().unwrap_or(0));
            }
            other => panic!("모르는 걸음 {other}"),
        }
        out.push(Obs { op: op.into(), us, usage: c.delay.trend.state().name().to_string(), aimd: c.delay.aimd.rate });
    }
    out
}

#[test]
fn a_frame_burst_spread_by_the_link_raises_the_estimate_without_underuse() {
    let v = load("sim.json");
    let obs = replay(&v, "묶음이 도는 동안 나가는 미디어");
    let fb: Vec<&Obs> = obs.iter().filter(|o| o.op == "feedback").collect();
    assert!(fb.len() >= 50, "stand check — the trace carries feedback over its whole run ({})", fb.len());
    let under: Vec<i64> = fb.iter().filter(|o| o.usage == "underuse").map(|o| o.us).collect();
    assert!(under.is_empty(), "S-sym-RULE-SIM-TICK-7 frames sent in a burst and spread by a 5 Mbit/s link never read as underuse while warming up: underuse at {under:?}");
    let start = obs.iter().find(|o| o.op == "network").map(|o| o.aimd).unwrap_or(0);
    let last = fb.last().map(|o| o.aimd).unwrap_or(0);
    assert!(start > 0, "stand check — the start rate is set");
    assert!(last > start * 2, "S-sym-RULE-SIM-TICK-7 the delay-based estimate climbs from the start rate during the warm-up ({start} → {last})");
}

#[test]
fn the_first_probe_opens_on_its_initial_state_even_after_an_early_overuse() {
    let mut c = Congestion::new(600_000, 0);
    let mut sent = Vec::new();
    let mut seen = false;
    for k in 0..400i64 {
        let at = k * 5_000;
        c.on_sent(1_200, at, at);
        sent.push(Fed { seq: k, send_us: at, size: 1_200, recv_us: Some(at + 100 + k * 1_500), tag: None });
        if k % 20 == 19 {
            let now = at + 100 + k * 1_500 + 1_000;
            let base = sent[0].recv_us.unwrap_or(0) / 64_000 * 64_000;
            let _ = c.on_feedback(&sent, now, base);
            sent.clear();
            if c.delay.trend.state().name() == "overuse" {
                seen = true;
                break;
            }
        }
    }
    assert!(seen, "stand check — a growing queue read as overuse before the network came up");
    let step = c.set_network(true, 2_000_000);
    let init: Vec<i64> = step.opened.iter().filter(|o| o.why.name() == "init").map(|o| o.bitrate).collect();
    assert!(!init.is_empty(), "S-sym-RULE-SIM-TICK-10② the connection-start probe opens on its initial state alone — an overuse that came first does not keep it shut: {:?}", step.opened.iter().map(|o| (o.why.name(), o.bitrate)).collect::<Vec<_>>());
}

#[test]
fn an_unmeasured_probe_cluster_is_none_and_never_a_zero() {
    let tag = |id: i64| Tag { id, min_probes: 5, min_bytes: 5_000, bitrate: 3_000_000 };
    let mut few = ProbeEstimator::default();
    let got: Vec<Option<i64>> = (0..2).map(|k| few.handle(tag(1), k * 2_000, 10_000 + k * 2_000, 1_000)).collect();
    assert_eq!(got, vec![None, None], "S-sym-RULE-SIM-TICK-2② too few probes of a cluster measure nothing");
    assert!(few.last.is_none(), "S-sym-RULE-SIM-TICK-2② an unmeasured cluster leaves no result — it is not filled with 0: {:?}", few.last);
    let mut odd = ProbeEstimator::default();
    let got: Vec<Option<i64>> = (0..5).map(|k| odd.handle(tag(2), k * 2_500, 20_000 + k * 250, 1_000)).collect();
    assert!(got.iter().all(Option::is_none), "S-sym-RULE-SIM-TICK-2② a cluster received far faster than it was sent is not a measurement: {got:?}");
    let last = odd.last.map(|m| (m.id, m.result));
    assert_eq!(last, Some((2, None)), "S-sym-RULE-SIM-TICK-2② the rejected cluster is recorded with no result — None, not 0");
    let mut fine = ProbeEstimator::default();
    let got: Vec<Option<i64>> = (0..5).map(|k| fine.handle(tag(3), k * 2_500, 30_000 + k * 2_500, 1_000)).collect();
    assert!(got.last().copied().flatten().is_some_and(|b| b > 0), "positive control — a clean cluster measures a rate: {got:?}");
}
