// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! 1층 — 대역 추정 자취의 성질(context 202611/20261124b 서버 규칙 갈래). `vectors.rs` 의 sim 재생은 걸음마다 제품 값 = 원천 값만 맞대서, 원천과 제품이 같이 판정 조건을 어기면 초록이다. 여기서는 같은 입력(sim.json `congestion` · `aimd`)을 제품으로 다시 돌려 ★원천 값과 견주지 않고 판정 조건이 그 자취에서 일어나는지만 단언한다.

use std::path::{Path, PathBuf};

use oxsfud::sim::bwe::*;
use oxsfud::sim::core::Aimd;
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

#[derive(Debug, Clone)]
struct Obs {
    op: String,
    us: i64,
    target: i64,
    by: &'static str,
    loss: &'static str,
    aimd: i64,
    padding: Option<i64>,
    twccless: Option<bool>,
    opened: usize,
    opened_at: Vec<(i64, i64)>,
    judged: Option<(i64, i64)>,
    throughput: Option<i64>,
    shot: Option<(i64, i64, i64)>,
}

fn trace(v: &Value, starts: &str) -> (Vec<Obs>, i64) {
    let c = v["congestion"].as_array().expect("congestion").iter().find(|c| c["name"].as_str().unwrap_or("").starts_with(starts)).unwrap_or_else(|| panic!("자취 {starts} 없음"));
    let mut g: Option<Congestion> = None;
    let mut out = Vec::new();
    let mut desired = 0;
    for st in c["steps"].as_array().expect("steps") {
        let us = st["us"].as_i64().unwrap_or(0);
        let op = st["op"].as_str().unwrap_or("");
        if op == "new" {
            g = Some(Congestion::new(st["start_bps"].as_i64().unwrap_or(0), us));
            continue;
        }
        let c = g.as_mut().expect("new first");
        let (mut padding, mut twccless) = (None, None);
        let mut by = "";
        let mut opened = 0;
        let mut shot = None;
        let mut step: Option<Step> = None;
        match op {
            "sent" => c.on_sent(st["size"].as_i64().unwrap_or(0), us, us),
            "shot" => {
                shot = c.prober.shot(us).map(|x| (x.tag.id, x.tag.min_bytes, x.bytes));
            }
            "media" => {
                let _ = c.prober.media(us, st["size"].as_i64().unwrap_or(0));
            }
            "twccless" => twccless = Some(c.twccless(us)),
            "padding" => padding = Some(c.padding(us)),
            "rtt" => c.on_rtt(st["rtt_us"].as_i64().unwrap_or(0)),
            "shot_sent" => c.prober.sent(us, st["bytes"].as_i64().unwrap_or(0)),
            "network" => step = Some(c.set_network(true, us)),
            "desired" => {
                if desired == 0 {
                    desired = st["bps"].as_i64().unwrap_or(0);
                }
                step = Some(c.set_desired(st["bps"].as_i64().unwrap_or(0), us));
            }
            "process" => step = Some(c.process(us)),
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
                step = Some(c.on_feedback(&fed, us, st["base_us"].as_i64().unwrap_or(0)));
            }
            other => panic!("모르는 걸음 {other}"),
        }
        let (mut opened_at, mut judged) = (Vec::new(), None);
        if let Some(x) = &step {
            (by, opened) = (x.by.name(), x.opened.len());
            opened_at = x.opened.iter().map(|o| (o.id, o.bitrate)).collect();
            judged = x.judged.map(|j| (j.id, j.send));
        }
        out.push(Obs { op: op.into(), us, target: c.available(), by, loss: c.rate.loss_state.name(), aimd: c.delay.aimd.rate, padding, twccless, opened, opened_at, judged, throughput: c.throughput.bitrate(), shot });
    }
    (out, desired)
}

#[test]
fn the_start_rate_is_valid_at_once_and_the_first_probe_reaches_the_desired_rate() {
    let mut a = Aimd::new();
    assert!(!a.valid, "stand check — no rate before the start");
    a.set_start(600_000);
    assert!(a.valid, "S-sym-RULE-SIM-TICK-6 the rate is valid the moment the start rate is given — no separate measured mark");
    let v = load("sim.json");
    let (obs, desired) = trace(&v, "연결 직후 탐색");
    assert!(desired > 0, "stand check — the trace sets a desired rate");
    let first = obs.iter().find(|o| o.op == "feedback" && o.us <= 1_000_000 && o.by == "probe");
    assert!(
        first.is_some_and(|o| o.target >= desired),
        "S-sym-RULE-SIM-TICK-6 within 1s of connecting a probe sets the target at or above the desired rate {desired} (the upper layer price): {:?}",
        obs.iter().filter(|o| o.op == "feedback" && o.us <= 1_000_000).map(|o| (o.us, o.by, o.target)).collect::<Vec<_>>()
    );
}

#[test]
fn padding_climbs_only_in_its_state_and_stops_on_the_delay_estimate() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "조임");
    let mut state = "delay";
    let mut padded = 0;
    let mut handover = 0;
    for o in &obs {
        if let Some(p) = o.padding {
            if p > 0 {
                padded += 1;
                assert_eq!(state, "padding", "정:RULE-SIM-TICK padding leaves only while loss-based is climbing by padding · {} µs", o.us);
            }
        }
        if state == "padding" && o.loss == "delay" {
            handover += 1;
            assert_eq!(o.target, o.aimd, "S-sym-RULE-SIM-TICK-17② when the estimate reaches the delay-based value padding stops on that value · {} µs", o.us);
        }
        state = o.loss;
    }
    assert!(padded > 0, "stand check — the squeeze trace pads ({padded})");
    assert!(handover > 0, "S-sym-RULE-SIM-TICK-17② the padding climb ends on the delay-based estimate — padding stops ({handover} hand-backs)");
}

#[test]
fn twccless_takes_over_after_the_stream_limit_and_never_after_feedback() {
    let v = load("sim.json");
    let (none, _) = trace(&v, "TWCC 없는 구독자");
    let marks: Vec<(i64, bool)> = none.iter().filter_map(|o| o.twccless.map(|t| (o.us, t))).collect();
    let flip = marks.iter().position(|(_, t)| *t);
    assert!(
        flip.is_some_and(|i| i > 0 && marks[i..].iter().all(|(_, t)| *t) && marks[i].0 >= 2_000_000),
        "S-sym-RULE-SIM-TICK-19① a subscriber that never sends feedback is handed to v1 once the stream limit after the first packet passes and stays there: {marks:?}"
    );
    let (gap, _) = trace(&v, "영상이 멈춰");
    let seen: Vec<bool> = gap.iter().filter_map(|o| o.twccless).collect();
    assert!(!seen.is_empty() && seen.iter().all(|t| !t), "S-sym-RULE-SIM-TICK-19③ a transport that sent feedback once does not go to v1 even through a feedback gap");
}

#[test]
fn loss_that_is_not_the_rate_keeps_the_delay_estimate_and_the_floor_holds() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "고유 손실");
    assert!(obs.iter().all(|o| o.loss == "delay"), "S-sym-RULE-SIM-TICK-8② inherent loss (one in 25 dropped on an open link) does not move loss-based out of the delay state");
    let (obs, _) = trace(&v, "바닥");
    let held: Vec<&Obs> = obs.iter().filter(|o| o.op == "feedback" && o.loss != "delay").collect();
    assert!(!held.is_empty(), "stand check — the floor trace enters a loss state");
    for o in held {
        if let Some(tp) = o.throughput {
            assert!(o.target >= tp.min(o.aimd), "S-sym-RULE-SIM-TICK-8③ in a loss state the target stays at or above min(throughput, delay estimate) · {} µs target {} throughput {tp} aimd {}", o.us, o.target, o.aimd);
        }
    }
}

#[test]
fn a_feedback_gap_with_media_flowing_does_not_raise_the_target() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "피드백만 끊김");
    let gap: Vec<&Obs> = obs.iter().filter(|o| o.us >= 3_000_000 && o.us < 6_000_000 && o.op == "process").collect();
    assert!(gap.len() > 10, "stand check — the clock keeps stepping through the gap");
    assert!(gap.windows(2).all(|w| w[1].target <= w[0].target), "S-sym-RULE-SIM-TICK-13 while feedback is cut the target does not rise: {:?}", gap.iter().map(|o| o.target).collect::<Vec<_>>());
    assert!(obs.iter().filter_map(|o| o.twccless).all(|t| !t), "S-sym-RULE-SIM-TICK-13 nor does it switch to the estimate-less judgment");
}

#[test]
fn a_late_cluster_is_dropped_and_loss_decreasing_opens_no_probe() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "늦은 묶음");
    let shots: Vec<(i64, i64, i64)> = obs.iter().filter_map(|o| o.shot).collect();
    let first = shots.first().map(|s| s.0).unwrap_or(0);
    let sent: i64 = shots.iter().filter(|s| s.0 == first).map(|s| s.2).sum();
    let min = shots.first().map(|s| s.1).unwrap_or(0);
    assert!(sent < min && shots.iter().any(|s| s.0 > first), "S-sym-RULE-SIM-TICK-14② a cluster that fell too late is dropped before its minimum bytes ({sent} < {min}) and the next cluster takes over: {shots:?}");
    let (obs, _) = trace(&v, "조임 중 바라는 양 올림");
    let probed: Vec<(i64, &str)> = obs.iter().filter(|o| o.opened > 0 && (o.loss == "decreasing" || o.loss == "padding")).map(|o| (o.us, o.loss)).collect();
    assert!(probed.is_empty(), "S-sym-RULE-SIM-TICK-14③ no probe cluster opens while loss-based is decreasing (or padding): {probed:?}");
    assert!(obs.iter().any(|o| o.op == "desired" && o.us == 7_500_000), "stand check — the desired rate rises during the squeeze");
}

#[test]
fn padding_that_cannot_climb_in_time_falls_back_and_a_long_gap_reopens_probing() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "패딩 시한");
    let back = obs.windows(2).any(|w| w[0].loss == "padding" && w[1].loss == "decreasing");
    assert!(back, "S-sym-RULE-SIM-TICK-17③ padding that cannot climb within its time returns to decreasing");
    let (obs, _) = trace(&v, "영상이 멈춰 오래 끊김");
    let reopened: Vec<i64> = obs.iter().filter(|o| o.opened > 0 && o.us >= 6_000_000 && o.us < 12_000_000).map(|o| o.us).collect();
    assert!(!reopened.is_empty(), "S-sym-RULE-SIM-TICK-18① once feedback is gone over 2s the clock step resets the delay detector and probing goes out again before media resumes");
}

#[test]
fn rate_caused_loss_steps_down_recovers_and_returns_to_delay() {
    let v = load("sim.json");
    let (obs, _) = trace(&v, "조임 —");
    let dec = obs.iter().position(|o| o.loss == "decreasing");
    let Some(d) = dec else { panic!("S-sym-RULE-SIM-TICK-8① rate-caused loss puts loss-based into decreasing — never happened") };
    assert!(d > 0 && obs[d].target < obs[d - 1].target, "S-sym-RULE-SIM-TICK-8① entering decreasing lowers the target · {} µs {} → {}", obs[d].us, obs[d - 1].target, obs[d].target);
    let rec = obs[d..].iter().position(|o| o.loss == "padding" || o.loss == "increasing").map(|i| i + d);
    assert!(rec.is_some_and(|r| obs[r..].iter().any(|o| o.loss == "delay")), "S-sym-RULE-SIM-TICK-8① decreasing → recovering → delay-based in that order");
}

#[test]
fn every_judged_probe_was_sent_at_least_at_its_ordered_rate() {
    let v = load("sim.json");
    let mut judged = 0;
    for name in ["연결 직후 탐색", "여유 구간", "묶음이 도는 동안"] {
        let (obs, _) = trace(&v, name);
        let ordered: std::collections::HashMap<i64, i64> = obs.iter().flat_map(|o| o.opened_at.iter().copied()).collect();
        for (id, send) in obs.iter().filter_map(|o| o.judged) {
            let want = ordered.get(&id).copied().unwrap_or(0);
            judged += 1;
            assert!(send >= want, "S-sym-RULE-SIM-TICK-14① the next shot time counts the bytes sent since the cluster start, so a judged cluster went out at its ordered rate · {name} cluster {id} sent {send} < ordered {want}");
        }
    }
    assert!(judged >= 3, "stand check — clusters were judged ({judged})");
}
