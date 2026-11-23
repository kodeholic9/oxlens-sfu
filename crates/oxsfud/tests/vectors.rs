// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · 정:RULE-SIM-TICK · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/` 넷(fwd · rtcp · sim · flr)을 제품 함수에 그대로 넣는다. 판 묶음이 없거나 비면 빨강.

use std::path::{Path, PathBuf};

use oxsfud::flr::bind::{Effect, Slot};
use oxsfud::fwd::core::{Rewriter, covers, pt_byte1, serial_after, vp8_pic, vp8_write};
use oxsfud::rtcp::core::{RecvStats, SYM_LARGE, SYM_NONE, SYM_SMALL, TwccRecv};
use oxsfud::sim::core::{Trendline, Usage};
use oxsfud::sub::ext::{ExtInput, ExtTable, build};
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

fn cases<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = v[key].as_array().unwrap_or_else(|| panic!("판 묶음 {key} 없음"));
    assert!(!a.is_empty(), "판 묶음 {key} 비었다");
    a
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect()
}

#[test]
fn fwd_rewrite_vectors() {
    let v = load("fwd.json");
    for c in cases(&v, "rewrite") {
        let name = c["name"].as_str().unwrap_or("");
        let clock = c["clock"].as_u64().unwrap_or(48000) as u32;
        let mut rw = Rewriter::new(clock);
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let payload = hex(s["payload"].as_str().unwrap_or(""));
            let cov = covers(clock == 48000, &payload);
            if let Some(want) = s["covers"].as_u64() {
                assert_eq!(cov as u64, want, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 — 덮는 길이 {name} #{i}");
            }
            let m = rw.map(
                s["src"].as_u64().unwrap_or(0),
                s["seq"].as_u64().unwrap_or(0) as u16,
                s["ts"].as_u64().unwrap_or(0) as u32,
                s["at_ms"].as_u64().unwrap_or(0),
                cov,
            );
            assert_eq!(m.map(|x| x.seq as u64), s["out_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 · S-sym-IMPL-FWD-SOURCE-SWITCH-4①② — out_seq {name} #{i}");
            assert_eq!(m.map(|x| x.ts as u64), s["out_ts"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 — out_ts {name} #{i}");
        }
        if let Some(n) = c.get("early_nack") {
            let got = rw.unmap_nack(n["out_seq"].as_u64().unwrap_or(0) as u16).map(|x| x as u64);
            assert_eq!(got, n["in_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-5② — 창 안 경계 앞 NACK {name}");
        }
        if let Some(f) = c.get("flow") {
            let g = |k: &str| f[k].as_u64().unwrap_or(0);
            for i in 0..g("n") {
                let m = rw.map(g("src"), (g("seq0") + i) as u16, (g("ts0") + g("ts_step") * i) as u32, g("at0") + g("at_step") * i, 1);
                assert!(m.is_some(), "S-sym-IMPL-FWD-SOURCE-SWITCH-5① — 흐름은 버리지 않는다 {name} #{i}");
            }
        }
        for n in c["nack"].as_array().unwrap_or(&vec![]) {
            let got = rw.unmap_nack(n["out_seq"].as_u64().unwrap_or(0) as u16).map(|x| x as u64);
            assert_eq!(got, n["in_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 — NACK 되짚기 {name}");
        }
    }
    for c in cases(&v, "vp8") {
        let name = c["name"].as_str().unwrap_or("");
        let mut rw = Rewriter::new(90000);
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let payload = hex(s["payload"].as_str().unwrap_or(""));
            let pic = vp8_pic(&payload);
            assert!(pic.is_some(), "S-sym-IMPL-FWD-SOURCE-SWITCH-7 — VP8 descriptor 읽기 {name} #{i}");
            let m = rw.map_pic(
                s["src"].as_u64().unwrap_or(0),
                s["seq"].as_u64().unwrap_or(0) as u16,
                s["ts"].as_u64().unwrap_or(0) as u32,
                s["at_ms"].as_u64().unwrap_or(0),
                1,
                pic.as_ref(),
            );
            assert_eq!(m.map(|x| x.seq as u64), s["out_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 — out_seq {name} #{i}");
            assert_eq!(m.and_then(|x| x.pid).map(|x| x as u64), s["out_pid"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-7① — picture ID {name} #{i}");
            assert_eq!(m.and_then(|x| x.tl0).map(|x| x as u64), s["out_tl0"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-7① — TL0PICIDX {name} #{i}");
            if let (Some(m), Some(d), Some(want)) = (m, pic, s["out_payload"].as_str()) {
                let mut out = payload.clone();
                vp8_write(&mut out, &d, m.pid, m.tl0);
                assert_eq!(out, hex(want), "S-sym-IMPL-FWD-SOURCE-SWITCH-7② — 다시 쓴 descriptor {name} #{i}");
            }
        }
        for r in c["rtx"].as_array().unwrap_or(&vec![]) {
            let got = rw.unmap_nack(r["out_seq"].as_u64().unwrap_or(0) as u16).map(|x| x as u64);
            assert_eq!(got, r["in_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 — 재전송 되짚기 {name}");
            if let Some(want) = r["out_payload"].as_str() {
                let payload = hex(r["payload"].as_str().unwrap_or(""));
                let d = vp8_pic(&payload).expect("descriptor");
                let (pid, tl0) = rw.pic_out(Some(&d));
                let mut out = payload.clone();
                vp8_write(&mut out, &d, pid, tl0);
                assert_eq!(out, hex(want), "S-sym-IMPL-FWD-SOURCE-SWITCH-7③ — 재전송도 같은 값 {name}");
            }
        }
    }
    for s in cases(&v, "serial") {
        let (a, b) = (s["a"].as_u64().unwrap_or(0) as u16, s["b"].as_u64().unwrap_or(0) as u16);
        assert_eq!(serial_after(a, b), s["after"].as_bool().unwrap_or(false), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 — serial {a} {b}");
    }
    for p in cases(&v, "pt_byte1") {
        let out = pt_byte1(p["byte1"].as_u64().unwrap_or(0) as u8, p["pt"].as_u64().unwrap_or(0) as u8);
        assert_eq!(out as u64, p["out"].as_u64().unwrap_or(0), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 — PT 교체 marker 보존");
    }
    for s in cases(&v, "sr") {
        let mut rw = Rewriter::new(90000);
        rw.ts_off = s["ts_off"].as_u64().unwrap_or(0) as u32;
        let got = rw.translate_sr_ts(s["in"]["rtp_ts"].as_u64().unwrap_or(0) as u32);
        assert_eq!(got as u64, s["out"]["rtp_ts"].as_u64().unwrap_or(0), "S-sym-IMPL-RTCP-TERMINATE-1 — SR ts 번역");
        assert_eq!(s["out"]["ntp"], s["in"]["ntp"], "S-sym-IMPL-RTCP-TERMINATE-1 — NTP 원본");
        assert_eq!(s["out"]["ssrc"], s["downlink_ssrc"], "S-sym-IMPL-RTCP-TERMINATE-1 — 다운링크 SSRC");
    }
    for e in cases(&v, "ext") {
        let t = &e["table"];
        let id = |k: &str| t[k].as_u64().map(|x| x as u8);
        let table = ExtTable { mid: id("mid"), audio_level: id("audio_level"), abs_send_time: id("abs_send_time"), twcc: id("twcc") };
        let mid = e["mid"].as_str().unwrap_or("");
        let inp = ExtInput {
            mid: mid.as_bytes(),
            audio_level: e["audio_level_in"].as_u64().map(|x| x as u8),
            t_us: e["t_us"].as_u64().unwrap_or(0),
            twcc_seq: e["twcc_counter"].as_u64().map(|x| x as u16),
        };
        let mut out = Vec::new();
        build(&table, &inp, &mut out);
        let want = hex(e["hex"].as_str().unwrap_or(""));
        assert_eq!(out, want, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 — 확장 다시 짓기 {}", e["name"]);
        assert_eq!(table.one_byte(), e["form"] == "one", "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 — 꼴 {}", e["name"]);
    }
}

#[test]
fn rtcp_vectors() {
    let v = load("rtcp.json");
    for c in cases(&v, "rr") {
        let name = c["name"].as_str().unwrap_or("");
        let mut st = RecvStats::new(c["clock"].as_u64().unwrap_or(48000) as u32);
        for e in c["events"].as_array().unwrap_or(&vec![]) {
            if let Some(r) = e.get("report") {
                if r.is_null() {
                    assert!(st.report().is_none(), "S-sym-RULE-CONC-SANS-IO-1 — RR 출처 인정 전 블록 없음 {name}");
                    continue;
                }
                let got = st.report().unwrap_or_else(|| panic!("S-sym-RULE-CONC-SANS-IO-1 — RR 블록이 있어야 한다 {name}"));
                assert_eq!(got.fraction as u64, r["fraction"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — RR fraction {name}");
                assert_eq!(got.lost as i64, r["lost"].as_i64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — RR lost {name}");
                assert_eq!(got.extended_max as u64, r["extended_max"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — RR ext {name}");
                assert_eq!(got.jitter as u64, r["jitter"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — RR jitter {name}");
                continue;
            }
            let valid = st.update(
                e["seq"].as_u64().unwrap_or(0) as u16,
                e["ts"].as_u64().unwrap_or(0) as u32,
                e["arrival_ms"].as_u64().unwrap_or(0),
                100,
            );
            assert_eq!(valid as u64, e["valid"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — RR valid {name} seq {}", e["seq"]);
        }
    }
    for c in cases(&v, "twcc") {
        let name = c["name"].as_str().unwrap_or("");
        let mut t = TwccRecv::default();
        for e in c["events"].as_array().unwrap_or(&vec![]) {
            if let Some(b) = e.get("build") {
                let f = t.build().unwrap_or_else(|| panic!("S-sym-RULE-CONC-SANS-IO-1 — TWCC 피드백 없음 {name}"));
                let sym: String = f
                    .symbols
                    .iter()
                    .map(|s| match *s {
                        SYM_NONE => 'N',
                        SYM_SMALL => 'S',
                        SYM_LARGE => 'L',
                        _ => '?',
                    })
                    .collect();
                assert_eq!(f.base_seq as u64, b["base_seq"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — base {name}");
                assert_eq!(f.status_count as u64, b["status_count"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — count {name}");
                assert_eq!(f.reference_time as u64, b["reference_time"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — ref {name}");
                assert_eq!(f.fb_count as u64, b["fb_count"].as_u64().unwrap_or(0), "S-sym-RULE-CONC-SANS-IO-1 — fb_count {name}");
                assert_eq!(sym, b["symbols"].as_str().unwrap_or(""), "S-sym-RULE-CONC-SANS-IO-1 — symbols {name}");
                let want: Vec<i64> = b["deltas_250us"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
                let got: Vec<i64> = f.deltas_250us.iter().map(|d| *d as i64).collect();
                assert_eq!(got, want, "S-sym-RULE-CONC-SANS-IO-1 — deltas {name}");
                continue;
            }
            t.record(e["seq"].as_u64().unwrap_or(0) as u16, e["arrival_us"].as_i64().unwrap_or(0));
        }
    }
}

#[test]
fn sim_aimd_vectors() {
    let v = load("sim.json");
    use oxsfud::sim::core::*;
    let k = &v["constants"]["aimd"];
    assert_eq!(BWE_MIN, v["constants"]["bwe_min_bps"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 대역 추정 바닥 — 원천 SIM.model.BWE_MIN");
    assert_eq!(AIMD_BETA, k["beta"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK AIMD 수치 beta — 원천 SIM.model.AIMD");
    assert_eq!(AIMD_RTT_US, k["rttUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK AIMD 수치 rttUs");
    assert_eq!(AIMD_INIT_US, k["initUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK AIMD 수치 initUs");
    assert_eq!(AIMD_MAX_BPS, k["maxBps"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK AIMD 수치 maxBps");
    assert_eq!(AIMD_ALPHA, k["alpha"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK AIMD 수치 alpha");
    assert_eq!(AIMD_FRAME_US, k["frameUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK AIMD 수치 frameUs");
    for c in cases(&v, "aimd") {
        let name = c["name"].as_str().unwrap_or("");
        let mut a = Aimd::new();
        if let Some(start) = c["start_bps"].as_i64() {
            a.set_start(start);
        }
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let at = s["at_us"].as_i64().unwrap_or(0);
            if let Some(p) = s["probe_bps"].as_i64() {
                a.set_estimate(p, at);
            } else {
                let usage = Usage::parse(s["usage"].as_str().unwrap_or("normal")).unwrap_or(Usage::Normal);
                let k = s["throughput_bps"].as_i64();
                let reduce = match k {
                    Some(x) => a.time_to_reduce_further(at, x),
                    None => a.initial_time_to_reduce_further(at),
                };
                assert_eq!(reduce, s["out_reduce_further"].as_bool().unwrap_or(false), "정:RULE-SIM-TICK — 더 줄일 때인가 {name} 걸음 {i}");
                a.update(at, usage, k);
            }
            assert_eq!(a.rate, s["out_bps"].as_i64().unwrap_or(-1), "sim 벡터 AIMD {name} 걸음 {i}");
            assert_eq!(a.near(), s["out_near"].as_bool().unwrap_or(false), "sim 벡터 AIMD near {name} 걸음 {i}");
            assert_eq!(a.valid, s["out_valid"].as_bool().unwrap_or(false), "S-sym-RULE-SIM-TICK-6 — 속도 줄이 유효한가 {name} 걸음 {i}");
        }
    }
}

#[test]
fn sim_allocator_vectors() {
    use oxsfud::sim::alloc::*;
    let v = load("sim.json");
    assert_eq!(HYSTERESIS, v["constants"]["alloc"]["hysteresis"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 배분 수치 hysteresis — 원천 SIM.model.ALLOC");
    for c in cases(&v, "allocator") {
        let name = c["name"].as_str().unwrap_or("");
        let mut a = Allocator::default();
        for (i, st) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let cands: Vec<Candidate> = st["candidates"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|x| Candidate {
                    id: x["id"].as_u64().unwrap_or(0),
                    weight: x["weight"].as_u64().unwrap_or(0) as u8,
                    points: x["points"].as_array().unwrap_or(&vec![]).iter().map(|p| Point { bitrate: p["bitrate"].as_i64().unwrap_or(0), layer: p["layer"].as_i64().unwrap_or(0) as i8 }).collect(),
                    current: x["current"].as_u64().map(|n| n as usize),
                })
                .collect();
            let (grants, left) = a.distribute(st["available"].as_i64().unwrap_or(0), &cands);
            let want = st["out"]["grants"].as_array().cloned().unwrap_or_default();
            assert_eq!(grants.len(), want.len(), "S-sym-RULE-SIM-TICK-11 — 배분받은 줄 수 {name} 걸음 {i}");
            for (g, w) in grants.iter().zip(want.iter()) {
                assert_eq!(g.id, w["id"].as_u64().unwrap_or(u64::MAX), "S-sym-RULE-SIM-TICK-11 — 줄 차례 {name} 걸음 {i}");
                assert_eq!(g.layer as i64, w["layer"].as_i64().unwrap_or(9), "S-sym-RULE-SIM-TICK-11 · S-sym-RULE-SIM-TICK-12 — 층 {name} 걸음 {i} 줄 {}", g.id);
                assert_eq!(g.point as u64, w["point"].as_u64().unwrap_or(99), "S-sym-RULE-SIM-TICK-11 — 점 {name} 걸음 {i}");
                assert_eq!(g.changed, w["changed"].as_bool().unwrap_or(false), "정:RULE-SIM-TICK — 바뀌었나 {name} 걸음 {i}");
                assert_eq!(g.reason.name(), w["reason"].as_str().unwrap_or(""), "S-sym-RULE-SIM-TICK-12 — 못 오른 까닭 {name} 걸음 {i}");
                assert_eq!(g.need, w["need"].as_i64().unwrap_or(-1), "S-sym-RULE-SIM-TICK-12 — 위 점에 더 드는 양 {name} 걸음 {i}");
            }
            assert_eq!(left, st["out"]["left"].as_i64().unwrap_or(-1), "S-sym-RULE-SIM-TICK-11 — 남은 양 {name} 걸음 {i}");
        }
    }
}

#[test]
fn sim_congestion_vectors() {
    use oxsfud::sim::bwe::*;
    let v = load("sim.json");
    let k = &v["constants"];
    assert_eq!(CONGESTION_START_BPS, k["congestion"]["startBps"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 대역 추정 수치 startBps — 원천 SIM.model.CONGESTION");
    assert_eq!(CONGESTION_PROCESS_US, k["congestion"]["processUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 대역 추정 수치 processUs");
    assert_eq!(PROBE_FIRST, k["probe"]["first"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 탐색 수치 first — 원천 SIM.model.PROBE");
    assert_eq!(PROBE_SECOND, k["probe"]["second"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 탐색 수치 second");
    assert_eq!(PROBE_ALR_INTERVAL_US, k["probe"]["alrIntervalUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 탐색 수치 alrIntervalUs");
    assert_eq!(PROBE_DURATION_US, k["probe"]["durationUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 탐색 수치 durationUs");
    assert_eq!(PROBER_MAX_DELAY_US, k["prober"]["maxDelayUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 묶음 쏘기 수치 maxDelayUs — 원천 SIM.model.PROBER");
    assert_eq!(PE_UTILIZATION, k["probe_estimator"]["utilization"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 묶음 실측 수치 utilization — 원천 SIM.model.PROBE_EST");
    assert_eq!(ALR_USAGE, k["alr"]["usage"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 여유 구간 수치 usage — 원천 SIM.model.ALR");
    assert_eq!(TP_WINDOW_PACKETS as u64, k["throughput"]["windowPackets"].as_u64().unwrap_or(0), "정:RULE-SIM-TICK 처리량 수치 windowPackets — 원천 SIM.model.THROUGHPUT");
    assert_eq!(LOSS_MAX_INCREASE, k["loss"]["maxIncrease"].as_f64().unwrap_or(f64::NAN), "정:RULE-SIM-TICK 손실 기반 수치 maxIncrease — 원천 SIM.model.LOSS");
    assert_eq!(LOSS_WINDOW as u64, k["loss"]["window"].as_u64().unwrap_or(0), "정:RULE-SIM-TICK 손실 기반 수치 window");
    assert_eq!(TARGET_MAX_RTT_US, k["target"]["maxRttUs"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK 목표 속도 수치 maxRttUs — 원천 SIM.model.TARGET");
    for c in cases(&v, "congestion") {
        let name = c["name"].as_str().unwrap_or("");
        let mut g: Option<Congestion> = None;
        let mut shot: Option<Shot> = None;
        let mut sent_tags: Vec<Option<Tag>> = Vec::new();
        for (i, st) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let us = st["us"].as_i64().unwrap_or(0);
            let op = st["op"].as_str().unwrap_or("");
            if op == "new" {
                g = Some(Congestion::new(st["start_bps"].as_i64().unwrap_or(0), us));
                continue;
            }
            let Some(c) = g.as_mut() else { panic!("sim 벡터 대역 추정 {name} 걸음 {i} — new 가 먼저다") };
            let step = match op {
                "sent" => {
                    c.on_sent(st["size"].as_i64().unwrap_or(0), us, us);
                    sent_tags.push(shot.map(|s| s.tag));
                    continue;
                }
                "shot" => {
                    let s = c.prober.shot(us);
                    let o = &st["out"];
                    let got = s.map(|x| (x.tag.id, x.tag.min_probes, x.tag.min_bytes, x.tag.bitrate, x.bytes, x.first));
                    let want = Some((o["id"].as_i64().unwrap_or(-1), o["min_probes"].as_i64().unwrap_or(-1), o["min_bytes"].as_i64().unwrap_or(-1), o["bitrate"].as_i64().unwrap_or(-1), o["bytes"].as_i64().unwrap_or(-1), o["first"].as_bool().unwrap_or(false)));
                    assert_eq!(got, want, "S-sym-RULE-SIM-TICK-14 — 탐색 발 {name} 걸음 {i}");
                    shot = s;
                    continue;
                }
                "media" => {
                    let got = c.prober.media(us, st["size"].as_i64().unwrap_or(0)).map(|t| t.id).unwrap_or(0);
                    assert_eq!(got, st["out"].as_i64().unwrap_or(-1), "S-sym-RULE-SIM-TICK-20 — 묶음이 도는 동안 나간 패킷 {name} 걸음 {i}");
                    continue;
                }
                "twccless" => {
                    assert_eq!(c.twccless(us), st["out"].as_bool().unwrap_or(false), "S-sym-RULE-SIM-TICK-19 — TWCC 없는 구독자 {name} 걸음 {i}");
                    continue;
                }
                "padding" => {
                    assert_eq!(c.padding(us), st["out"].as_i64().unwrap_or(-1), "S-sym-RULE-SIM-TICK-17 — 패딩 예산 {name} 걸음 {i}");
                    continue;
                }
                "rtt" => {
                    c.on_rtt(st["rtt_us"].as_i64().unwrap_or(0));
                    continue;
                }
                "shot_sent" => {
                    c.prober.sent(us, st["bytes"].as_i64().unwrap_or(0));
                    shot = None;
                    continue;
                }
                "network" => c.set_network(true, us),
                "desired" => c.set_desired(st["bps"].as_i64().unwrap_or(0), us),
                "process" => c.process(us),
                "feedback" => {
                    let fed: Vec<Fed> = st["pk"]
                        .as_array()
                        .unwrap_or(&vec![])
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
                    c.on_feedback(&fed, us, st["base_us"].as_i64().unwrap_or(0))
                }
                other => panic!("sim 벡터 대역 추정 {name} 걸음 {i} — 모르는 걸음 {other}"),
            };
            let o = &st["out"];
            assert_eq!(c.available(), o["target"].as_i64().unwrap_or(-1), "S-sym-RULE-SIM-TICK-13 — 목표 속도 {name} 걸음 {i}({op} @{us})");
            assert_eq!(step.target, o["notify"].as_i64(), "정:RULE-SIM-TICK — 목표 알림 {name} 걸음 {i}({op} @{us})");
            assert_eq!(step.by.name(), o["by"].as_str().unwrap_or(""), "정:RULE-SIM-TICK — 목표를 정한 것 {name} 걸음 {i}({op} @{us})");
            let opened: Vec<(i64, i64, &str)> = step.opened.iter().map(|x| (x.id, x.bitrate, x.why.name())).collect();
            let want: Vec<(i64, i64, &str)> = o["opened"].as_array().map(|a| a.iter().map(|x| (x["id"].as_i64().unwrap_or(-1), x["bitrate"].as_i64().unwrap_or(-1), x["why"].as_str().unwrap_or(""))).collect()).unwrap_or_default();
            assert_eq!(opened, want, "S-sym-RULE-SIM-TICK-14 — 연 탐색 묶음 {name} 걸음 {i}({op} @{us})");
            let judged = step.judged.map(|j| (j.id, j.send, j.recv, j.result));
            let wj = if o["judged"].is_null() { None } else { Some((o["judged"]["id"].as_i64().unwrap_or(-1), o["judged"]["send"].as_i64().unwrap_or(-1), o["judged"]["recv"].as_i64().unwrap_or(-1), o["judged"]["result"].as_i64())) };
            assert_eq!(judged, wj, "S-sym-RULE-SIM-TICK-14 — 묶음 실측 {name} 걸음 {i}({op} @{us})");
            assert_eq!(c.delay.trend.state().name(), o["usage"].as_str().unwrap_or(""), "정:RULE-SIM-TICK — 지연 기울기 판정 {name} 걸음 {i}");
            assert_eq!(c.delay.aimd.rate, o["aimd"].as_i64().unwrap_or(-1), "정:RULE-SIM-TICK — 지연 기반 속도 {name} 걸음 {i}({op} @{us})");
            assert_eq!(c.rate.loss_state.name(), o["loss_state"].as_str().unwrap_or(""), "S-sym-RULE-SIM-TICK-8 — 손실 기반 상태 {name} 걸음 {i}({op} @{us})");
            assert_eq!(c.alr.started_at.is_some(), o["alr"].as_bool().unwrap_or(false), "정:RULE-SIM-TICK — 여유 구간 {name} 걸음 {i}({op} @{us})");
            assert_eq!(c.throughput.bitrate(), o["throughput"].as_i64(), "정:RULE-SIM-TICK — 처리량 {name} 걸음 {i}({op} @{us})");
        }
        assert!(!sent_tags.is_empty(), "sim 벡터 대역 추정 {name} — 보낸 패킷이 없다");
    }
}

#[test]
fn sim_arrival_vectors() {
    use oxsfud::sim::core::*;
    let v = load("sim.json");
    let k = &v["arrival_constants"];
    let n = |key: &str| k[key].as_i64().unwrap_or(-1);
    assert_eq!(IA_GROUP_US, n("groupUs"), "정:RULE-SIM-TICK 묶음 수치 groupUs — 원천 SIM.model.ARRIVAL");
    assert_eq!(IA_BURST_US, n("burstUs"), "정:RULE-SIM-TICK 묶음 수치 burstUs");
    assert_eq!(IA_MAX_BURST_US, n("maxBurstUs"), "정:RULE-SIM-TICK 묶음 수치 maxBurstUs");
    assert_eq!(IA_OFFSET_RESET_US, n("offsetResetUs"), "정:RULE-SIM-TICK 묶음 수치 offsetResetUs");
    assert_eq!(IA_REORDER_RESET as i64, n("reorderReset"), "정:RULE-SIM-TICK 묶음 수치 reorderReset");
    for c in cases(&v, "arrival") {
        let name = c["name"].as_str().unwrap_or("");
        let mut ia = InterArrival::default();
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let got = ia.push(s["send_us"].as_i64().unwrap_or(0), s["arrival_us"].as_i64().unwrap_or(0), s["system_us"].as_i64().unwrap_or(0));
            let want = s["out"].as_array().map(|a| (a[0].as_f64().unwrap_or(f64::NAN), a[1].as_f64().unwrap_or(f64::NAN)));
            match (got, want) {
                (Some((r, sd)), Some((wr, ws))) => assert!(
                    (r - wr).abs() < 1e-9 && (sd - ws).abs() < 1e-9,
                    "S-sym-RULE-SIM-TICK-7 — 묶음 간 델타 {name} 걸음 {i}: 받은 {r} · 보낸 {sd} ≠ {wr} · {ws}"
                ),
                (g, w) => assert_eq!(g.is_some(), w.is_some(), "S-sym-RULE-SIM-TICK-7 — 묶음이 끝났나 {name} 걸음 {i}"),
            }
        }
    }
}

#[test]
fn sim_policy_vectors() {
    use oxsfud::sim::core::*;
    let v = load("sim.json");
    let k = &v["policy_constants"];
    let f = |n: &str| k[n].as_f64().unwrap_or(f64::NAN);
    for (name, got, want) in [
        ("demoteLoss", P_DEMOTE_LOSS, f("demoteLoss")),
        ("cleanLoss", P_CLEAN_LOSS, f("cleanLoss")),
        ("nackRatio", P_NACK_RATIO, f("nackRatio")),
        ("nackWinTicks", P_NACK_WIN_TICKS as f64, f("nackWinTicks")),
        ("demoteTicks", P_DEMOTE_TICKS as f64, f("demoteTicks")),
        ("backoffMs", P_BACKOFF_MS as f64, f("backoffMs")),
        ("backoffMaxMs", P_BACKOFF_MAX_MS as f64, f("backoffMaxMs")),
        ("collapseMs", P_COLLAPSE_MS as f64, f("collapseMs")),
    ] {
        assert_eq!(got, want, "정:RULE-SIM-TICK 층 판단 수치 {name} — 원천 SIM.model.SIM_POLICY 와 같아야 한다");
    }
    for c in cases(&v, "policy") {
        let name = c["name"].as_str().unwrap_or("");
        let mut p = Policy::default();
        for (i, st) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let s = &st["in"];
            let sig = Signals {
                now: s["now"].as_u64().unwrap_or(0),
                loss: s["loss"].as_f64().unwrap_or(0.0),
                sent: s["sent"].as_u64().unwrap_or(0),
                repeats: s["repeats"].as_u64().unwrap_or(0),
                drops: s["drops"].as_u64().unwrap_or(0),
            };
            let d = p.tick(&sig);
            let o = &st["out"];
            let reasons: Vec<&str> = d.reasons.iter().map(|r| r.name()).collect();
            let want: Vec<&str> = o["reasons"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            assert_eq!(reasons, want, "S-sym-RULE-SIM-TICK-4 · S-sym-RULE-SIM-TICK-5 — 사유 {name} 걸음 {i}");
            assert_eq!(d.to.map(|x| x as u64), o["to"].as_u64(), "S-sym-RULE-SIM-TICK-1 — 층 바꿈 {name} 걸음 {i}");
            assert!((d.nack_ratio - o["nackRatio"].as_f64().unwrap_or(f64::NAN)).abs() < 1e-12, "S-sym-RULE-SIM-TICK-4 — 반복 NACK 비율 {name} 걸음 {i}");
            assert_eq!(p.level as u64, st["level"].as_u64().unwrap_or(9), "정:RULE-SIM-TICK — 층 {name} 걸음 {i}");
            assert_eq!(p.backoff, st["backoff_ms"].as_u64().unwrap_or(0), "S-sym-RULE-SIM-TICK-1 — backoff {name} 걸음 {i}");
        }
    }
}

#[test]
fn rtcp_nack_ledger_vectors() {
    use oxsfud::rtcp::nack::*;
    let v = load("rtcp.json");
    let k = &v["nack_ledger_constants"];
    assert_eq!(MAX_PER_SEQ as u64, k["maxPerSeq"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 maxPerSeq — 원천 RTCP.model.NACK_LEDGER");
    assert_eq!(REPEAT_GAP_MS, k["repeatGapMs"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 repeatGapMs");
    assert_eq!(SLOTS as u64, k["slots"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 slots");
    assert_eq!(FRESH_MS, k["freshMs"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 freshMs");
    assert_eq!(MAX_AGE_MS, k["maxAgeMs"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 maxAgeMs");
    for c in cases(&v, "nack_ledger") {
        let name = c["name"].as_str().unwrap_or("");
        let mut l = Ledger::default();
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            match s["event"].as_str() {
                Some("keyRequest") => {
                    l.on_key_request();
                    continue;
                }
                Some("keySent") => {
                    l.on_key_sent();
                    continue;
                }
                _ => {}
            }
            let st = l.step(s["seq"].as_u64().unwrap_or(0) as u16, s["at_ms"].as_u64().unwrap_or(0), s["age_ms"].as_u64().unwrap_or(0));
            assert_eq!(st.why.name(), s["why"].as_str().unwrap_or(""), "S-sym-RULE-RTCP-NACK-DOWN-2 · S-sym-RULE-RTCP-NACK-DOWN-3 · S-sym-RULE-SIM-TICK-5 — 장부 판정 {name} 걸음 {i}");
            assert_eq!(st.send, s["send"].as_bool().unwrap_or(false), "S-sym-RULE-RTCP-NACK-DOWN-2 — 재전송 {name} 걸음 {i}");
            assert_eq!(st.repeated, s["repeated"].as_bool().unwrap_or(false), "S-sym-RULE-SIM-TICK-4 — 반복 셈 {name} 걸음 {i}");
        }
        assert_eq!(l.repeats, c["repeats"].as_u64().unwrap_or(0), "S-sym-RULE-SIM-TICK-4 — 반복 수 {name}");
    }
}

#[test]
fn rtcp_pli_decide_vectors() {
    use oxsfud::r#pub::PliState;
    use oxsfud::rtcp::{ENGINE_FLOOR_MS, PLI_READY, PLI_USER, pli_decide};
    let v = load("rtcp.json");
    let k = &v["pli_constants"];
    assert_eq!(ENGINE_FLOOR_MS, k["engineFloorMs"].as_u64().unwrap_or(0), "정:IMPL-RTCP-TERMINATE 엔진 바닥 — 원천 FWD.model.EngineFloorMs");
    let cs = cases(&v, "pli_decide");
    assert_eq!(common::policy::Policy::default().media.pli_reopen_margin_ms as u64, k["marginMs"].as_u64().unwrap_or(0), "PL-KEY-MEDIA-PLI-REOPEN-MARGIN-MS 원천 FWD.model.ReopenMarginMs = 정책 기본");
    assert!(cs.len() >= 14, "S-sym-IMPL-RTCP-TERMINATE-11 PLI 판단 판 수");
    for c in cs {
        let name = c["name"].as_str().unwrap_or("");
        let gap = c["gap_ms"].as_u64().unwrap_or(0);
        let margin = c["margin_ms"].as_u64();
        assert!(margin.is_some(), "S-sym-IMPL-RTCP-TERMINATE-11 판 {name} 에 margin_ms 가 없다");
        let margin = margin.unwrap_or(0);
        let mut p = PliState::default();
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let w = s["want"].as_str().unwrap_or("-");
            let want = if w.contains('u') { PLI_USER } else { 0 } | if w.contains('i') { PLI_READY } else { 0 };
            let go = pli_decide(&mut p, want, s["bound"].as_bool().unwrap_or(true), s["at_ms"].as_u64().unwrap_or(0), gap, s["last_key_ms"].as_u64().unwrap_or(0), margin);
            assert_eq!(go.name(), s["go"].as_str().unwrap_or(""), "S-sym-IMPL-RTCP-TERMINATE-11 판단 {name} 걸음 {i}");
            assert_eq!(p.last_ms, s["last_ms"].as_u64().unwrap_or(0), "S-sym-IMPL-RTCP-TERMINATE-12 마지막 PLI {name} 걸음 {i}");
            assert_eq!(p.held_since, s["held_since"].as_u64(), "S-sym-IMPL-RTCP-TERMINATE-10 미룬 때 {name} 걸음 {i}");
            assert_eq!(p.sent, s["sent"].as_u64().unwrap_or(0), "S-sym-IMPL-RTCP-TERMINATE-11 넘긴 수 {name} 걸음 {i}");
        }
    }
}

#[test]
fn rtcp_nack_drain_vectors() {
    use oxsfud::rtcp::nack::*;
    let v = load("rtcp.json");
    let k = &v["nack_ledger_constants"];
    assert_eq!(PER_TURN as u64, k["perTurn"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 perTurn — 원천 RTCP.model.NACK_LEDGER");
    assert_eq!(TURN_MS, k["turnMs"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-DOWN 수치 turnMs");
    let cs = cases(&v, "nack_drain");
    assert!(cs.len() >= 6, "S-sym-RULE-RTCP-NACK-DOWN-4 흘림 판 수");
    for c in cs {
        let name = c["name"].as_str().unwrap_or("");
        let mut l = Ledger::default();
        let steps = c["steps"].as_array().cloned().unwrap_or_default();
        let ages: std::collections::HashMap<u16, u64> = c["ages"].as_object().map(|o| o.iter().filter_map(|(k, a)| Some((k.parse().ok()?, a.as_u64()?))).collect()).unwrap_or_default();
        for (i, s) in steps.iter().enumerate() {
            let at = s["at_ms"].as_u64().unwrap_or(0);
            let d = match s["event"].as_str() {
                Some("keyRequest") => {
                    l.on_key_request();
                    continue;
                }
                Some("keySent") => {
                    l.on_key_sent();
                    continue;
                }
                Some("nack") => {
                    let seqs: Vec<u16> = s["seqs"].as_array().unwrap_or(&vec![]).iter().map(|x| x.as_u64().unwrap_or(0) as u16).collect();
                    let idle = l.enqueue(&seqs);
                    assert_eq!(idle, s["idle"].as_bool().unwrap_or(false), "S-sym-RULE-RTCP-NACK-DOWN-4 대기줄이 비어 있었나 {name} 걸음 {i}");
                    if idle { l.drain(at, |q| ages.get(&q).map(|a| (*a, ()))) } else { Drained { sent: Vec::new(), refused: Vec::new() } }
                }
                _ => l.drain(at, |q| ages.get(&q).map(|a| (*a, ()))),
            };
            let sent: Vec<u64> = d.sent.iter().map(|(q, _)| *q as u64).collect();
            let want: Vec<u64> = s["sent"].as_array().unwrap_or(&vec![]).iter().map(|x| x.as_u64().unwrap_or(0)).collect();
            assert_eq!(sent, want, "S-sym-RULE-RTCP-NACK-DOWN-4 보낸 seq {name} 걸음 {i}");
            let refused: Vec<&str> = d.refused.iter().map(|w| w.name()).collect();
            let want_r: Vec<String> = s["refused"].as_array().unwrap_or(&vec![]).iter().map(|x| x.as_str().unwrap_or("").to_string()).collect();
            assert_eq!(refused, want_r, "S-sym-RULE-RTCP-NACK-DOWN-4 거른 까닭 {name} 걸음 {i}");
            assert_eq!(l.queued() as u64, s["queued"].as_u64().unwrap_or(0), "S-sym-RULE-RTCP-NACK-DOWN-4 남은 대기 {name} 걸음 {i}");
        }
    }
}

#[test]
fn rtcp_nack_up_vectors() {
    use oxsfud::r#pub::{NACK_JUMP, NACK_MAX, NackUp};
    let v = load("rtcp.json");
    let k = &v["nack_up_constants"];
    assert_eq!(NACK_JUMP, k["jump"].as_i64().unwrap_or(0), "정:RULE-RTCP-NACK-UP 수치 jump — 원천 RTCP.model.NACK_UP");
    assert_eq!(NACK_MAX as u64, k["max"].as_u64().unwrap_or(0), "정:RULE-RTCP-NACK-UP 수치 max");
    let mut n_cases = 0;
    for c in cases(&v, "nack_up") {
        n_cases += 1;
        let name = c["name"].as_str().unwrap_or("");
        let mut n = NackUp::default();
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let a = n.on_packet(s["seq"].as_u64().unwrap_or(0) as u16, s["at_ms"].as_u64().unwrap_or(0));
            assert_eq!(a.name(), s["arrival"].as_str().unwrap_or(""), "S-sym-RULE-RTCP-NACK-UP-2 — 도착 판정 {name} 걸음 {i}");
            assert_eq!(a.repair_passes(), s["passes"].as_bool().unwrap_or(false), "S-sym-RULE-RTCP-NACK-UP-2 — 복원본을 흘리나 {name} 걸음 {i}");
            let holes: Vec<u64> = s["holes"].as_array().unwrap_or(&vec![]).iter().filter_map(|x| x.as_u64()).collect();
            assert_eq!(n.holes().iter().map(|x| *x as u64).collect::<Vec<_>>(), holes, "S-sym-RULE-RTCP-NACK-UP-1 — 결손 장부 {name} 걸음 {i}");
        }
    }
    assert!(n_cases > 0, "정:RULE-RTCP-NACK-UP 벡터가 비었다");
}

#[test]
fn sim_trendline_vectors() {
    let v = load("sim.json");
    for c in cases(&v, "trendline") {
        let name = c["name"].as_str().unwrap_or("");
        let mut t = Trendline::new();
        for (i, s) in c["steps"].as_array().unwrap_or(&vec![]).iter().enumerate() {
            let got = t.update(s["recv_ms"].as_f64().unwrap_or(0.0), s["send_ms"].as_f64().unwrap_or(0.0), s["arrival_ms"].as_f64().unwrap_or(0.0));
            let want = Usage::parse(s["usage"].as_str().unwrap_or("")).unwrap_or(Usage::Normal);
            assert_eq!(got, want, "sim 벡터 trendline {name} 걸음 {i}");
        }
    }
}

fn effect(e: &Value) -> Effect {
    let a = e.as_array().cloned().unwrap_or_default();
    match a.first().and_then(|x| x.as_str()) {
        Some("bind") => Effect::Bind { user: a[1].as_str().unwrap_or("").into(), g: a[2].as_u64().unwrap_or(0) },
        _ => Effect::Release { g: a.get(1).and_then(|x| x.as_u64()).unwrap_or(0) },
    }
}

fn bound(v: &Value) -> Option<(String, u64)> {
    let a = v.as_array()?;
    Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_u64()?))
}

#[test]
fn flr_media_bind_vectors() {
    let v = load("flr.json");
    for c in cases(&v, "media_bind") {
        let name = c["name"].as_str().unwrap_or("");
        let decided: Vec<Effect> = c["decided"].as_array().map(|a| a.iter().map(effect).collect()).unwrap_or_default();
        let mut inorder = Slot::default();
        for e in &decided {
            inorder.apply(e);
        }
        assert_eq!(inorder.bound, bound(&c["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-3 — 판정 차례 결과 {name}");
        let orders = c["orders"].as_array().map(|a| a.to_vec()).unwrap_or_default();
        assert!(!orders.is_empty(), "S-sym-IMPL-FLR-MEDIA-BIND-4 — 집행 차례 판이 비었다 {name}");
        let mut differs = 0u64;
        for o in orders {
            let mut s = Slot::default();
            let idx: Vec<usize> = o["order"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect()).unwrap_or_default();
            let mut blind: Option<(String, u64)> = None;
            for i in &idx {
                blind = match &decided[*i] {
                    Effect::Bind { user, g } => Some((user.clone(), *g)),
                    Effect::Release { .. } => None,
                };
            }
            if blind != bound(&c["bound"]) {
                differs += 1;
            }
            let mut applied = vec![false; decided.len()];
            for i in &idx {
                applied[*i] = s.apply(&decided[*i]);
            }
            let want: Vec<bool> = o["applied"].as_array().map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
            let got: Vec<bool> = idx.iter().map(|i| applied[*i]).collect();
            assert_eq!(got, want, "S-sym-IMPL-FLR-MEDIA-BIND-4 — 적용 여부 {name} {idx:?}");
            assert_eq!(s.bound, bound(&o["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-5 — 끝 슬롯 {name} {idx:?}");
        }
        let want = c["targetless_differs"].as_u64().unwrap_or_else(|| panic!("targetless_differs 없음 {name}"));
        assert_eq!(differs, want, "S-sym-IMPL-FLR-MEDIA-BIND-4 — 번호를 안 보는 집행이 판정 차례와 갈리는 차례 수 {name}");
    }
}
