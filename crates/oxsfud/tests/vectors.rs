// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · 정:RULE-SIM-TICK · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/` 넷(fwd · rtcp · sim · flr)을 제품 함수에 그대로 넣는다. 판 묶음이 없거나 비면 빨강.

use std::path::{Path, PathBuf};

use oxsfud::flr::bind::{Effect, Slot};
use oxsfud::fwd::core::{Rewriter, covers, pt_byte1, serial_after};
use oxsfud::rtcp::core::{RecvStats, SYM_LARGE, SYM_NONE, SYM_SMALL, TwccRecv};
use oxsfud::sim::core::{Aimd, Usage};
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
            assert_eq!(m.seq as u64, s["out_seq"].as_u64().unwrap_or(0), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 — out_seq {name} #{i}");
            assert_eq!(m.ts as u64, s["out_ts"].as_u64().unwrap_or(0), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 — out_ts {name} #{i}");
        }
        for n in c["nack"].as_array().unwrap_or(&vec![]) {
            let got = rw.unmap_nack(n["out_seq"].as_u64().unwrap_or(0) as u16).map(|x| x as u64);
            assert_eq!(got, n["in_seq"].as_u64(), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 — NACK 되짚기 {name}");
        }
    }
    for s in cases(&v, "serial") {
        let (a, b) = (s["a"].as_u64().unwrap_or(0) as u16, s["b"].as_u64().unwrap_or(0) as u16);
        assert_eq!(serial_after(a, b), s["after"].as_bool().unwrap_or(false), "S-sym-IMPL-FWD-SOURCE-SWITCH-4 — serial {a} {b}");
    }
    for p in cases(&v, "pt_byte1") {
        let out = pt_byte1(p["byte1"].as_u64().unwrap_or(0) as u8, p["pt"].as_u64().unwrap_or(0) as u8);
        assert_eq!(out as u64, p["out"].as_u64().unwrap_or(0), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 — PT 교체 marker 보존");
    }
    for s in cases(&v, "sr") {
        let mut rw = Rewriter::new(90000);
        rw.ts_off = s["ts_off"].as_u64().unwrap_or(0) as u32;
        let got = rw.translate_sr_ts(s["in"]["rtp_ts"].as_u64().unwrap_or(0) as u32);
        assert_eq!(got as u64, s["out"]["rtp_ts"].as_u64().unwrap_or(0), "S-sym-IMPL-FWD-SOURCE-SWITCH-5 — SR ts 번역");
        assert_eq!(s["out"]["ntp"], s["in"]["ntp"], "S-sym-IMPL-FWD-SOURCE-SWITCH-5 — NTP 원본");
        assert_eq!(s["out"]["ssrc"], s["downlink_ssrc"], "S-sym-IMPL-FWD-SOURCE-SWITCH-5 — 다운링크 SSRC");
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
        assert_eq!(table.one_byte(), e["form"] == "one", "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-4 — 꼴 {}", e["name"]);
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
                let got = st.report();
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
    let rtt = v["constants"]["rtt_ms"].as_u64().unwrap_or(200);
    for c in cases(&v, "aimd") {
        let name = c["name"].as_str().unwrap_or("");
        let mut a = Aimd::new(c["start_bps"].as_u64().unwrap_or(0), rtt);
        for s in c["steps"].as_array().unwrap_or(&vec![]) {
            let usage = Usage::parse(s["usage"].as_str().unwrap_or("normal")).unwrap_or(Usage::Normal);
            let out = a.update(s["at_ms"].as_u64().unwrap_or(0), usage, s["acked_bps"].as_u64().unwrap_or(0));
            assert_eq!(out, s["out_bps"].as_u64().unwrap_or(0), "S-sym-RULE-SIM-TICK — AIMD {name} @{}", s["at_ms"]);
            assert_eq!(a.near(), s["out_near"].as_bool().unwrap_or(false), "S-sym-RULE-SIM-TICK — near {name} @{}", s["at_ms"]);
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
        for o in orders {
            let mut s = Slot::default();
            let idx: Vec<usize> = o["order"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect()).unwrap_or_default();
            let mut applied = vec![false; decided.len()];
            for i in &idx {
                applied[*i] = s.apply(&decided[*i]);
            }
            let want: Vec<bool> = o["applied"].as_array().map(|a| a.iter().filter_map(|x| x.as_bool()).collect()).unwrap_or_default();
            let got: Vec<bool> = idx.iter().map(|i| applied[*i]).collect();
            assert_eq!(got, want, "S-sym-IMPL-FLR-MEDIA-BIND-4 — 적용 여부 {name} {idx:?}");
            assert_eq!(s.bound, bound(&o["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-5 — 끝 슬롯 {name} {idx:?}");
        }
    }
}
