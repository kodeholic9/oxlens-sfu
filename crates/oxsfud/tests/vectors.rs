// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-CONC-SANS-IO 정:IMPL-FWD-SOURCE-SWITCH 정:IMPL-SUB-SUBSCRIBER-TABLE 정:IMPL-RTCP-TERMINATE 정:RULE-SIM-TICK 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS 정:PROP-FWD-SWITCH-TS-FORWARD 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! 1층 — 벡터 셋(fwd · rtcp · sim) 판 전부를 산수 함수에 그대로 넣는다 · 판 묶음이 없거나 비면 빨강 · 규격 위치는 `OXLENS_SPEC` 이 이긴다 · PROP 두 성질.

use std::path::PathBuf;

use oxsfud::fwd::core::{Rewriter, Sr, pt_byte1, seq_after, translate_sr};
use oxsfud::rtcp::core::{RecvStats, TwccRecv};
use oxsfud::sim::core::{Aimd, Usage};
use oxsfud::sub::ext::{ExtTable, build};
use serde_json::Value;

const UNIT: &str = "S-sym-RULE-CONC-SANS-IO-1";

fn spec_dir() -> PathBuf {
    match std::env::var("OXLENS_SPEC") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    }
}

fn load(name: &str) -> Value {
    let p = spec_dir().join("server/out/vectors").join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{UNIT} 벡터 파일 {} 없음: {e}", p.display()));
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("{UNIT} 벡터 {name} 형: {e}"))
}

fn set<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = v.get(key).and_then(Value::as_array).unwrap_or_else(|| panic!("{UNIT} 판 묶음 {key} 없음"));
    assert!(!a.is_empty(), "{UNIT} 판 묶음 {key} 비었다");
    a
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_else(|| panic!("{UNIT} 칸 {k} 없음 in {v}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn fwd_rewrite_vectors() {
    let d = load("fwd.json");
    for case in set(&d, "rewrite") {
        let name = case["name"].as_str().unwrap_or("?");
        let clock = u(case, "clock") as u32;
        let mut r = Rewriter::default();
        for st in case["steps"].as_array().unwrap_or_else(|| panic!("{UNIT} steps")) {
            let (s, t) = r.map(u(st, "src"), u(st, "seq") as u16, u(st, "ts") as u32, u(st, "at_ms"), clock);
            assert_eq!((u64::from(s), u64::from(t)), (u(st, "out_seq"), u(st, "out_ts")), "{UNIT} rewrite {name} step {st}");
        }
    }
}

#[test]
fn fwd_serial_pt_sr_vectors() {
    let d = load("fwd.json");
    for c in set(&d, "serial") {
        assert_eq!(seq_after(u(c, "a") as u16, u(c, "b") as u16), c["after"].as_bool().unwrap_or(false), "{UNIT} serial {c}");
    }
    for c in set(&d, "pt_byte1") {
        assert_eq!(u64::from(pt_byte1(u(c, "byte1") as u8, u(c, "pt") as u8)), u(c, "out"), "{UNIT} pt_byte1 {c}");
    }
    for c in set(&d, "sr") {
        let i = &c["in"];
        let o = translate_sr(Sr { ssrc: u(i, "ssrc") as u32, ntp: u(i, "ntp"), rtp_ts: u(i, "rtp_ts") as u32 }, u(c, "downlink_ssrc") as u32, u(c, "ts_off") as u32);
        let e = &c["out"];
        assert_eq!((u64::from(o.ssrc), o.ntp, u64::from(o.rtp_ts)), (u(e, "ssrc"), u(e, "ntp"), u(e, "rtp_ts")), "{UNIT} sr {c}");
    }
}

#[test]
fn ext_vectors() {
    let d = load("fwd.json");
    for c in set(&d, "ext") {
        let tb = &c["table"];
        let id = |k: &str| tb.get(k).and_then(Value::as_u64).map(|x| x as u8);
        let t = ExtTable { mid: id("mid"), audio_level: id("audio_level"), abs_send_time: id("abs_send_time"), twcc: id("twcc") };
        let mut out = Vec::new();
        build(&t, c["mid"].as_str(), c["audio_level_in"].as_u64().map(|x| x as u8), u(c, "t_us"), u(c, "twcc_counter"), &mut out);
        assert_eq!(t.one_byte(), c["form"].as_str() == Some("one"), "{UNIT} ext form {}", c["name"]);
        assert_eq!(hex(&out), c["hex"].as_str().unwrap_or(""), "{UNIT} ext {}", c["name"]);
    }
}

#[test]
fn rtcp_rr_vectors() {
    let d = load("rtcp.json");
    for case in set(&d, "rr") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut s = RecvStats::new(u(case, "clock") as u32);
        for ev in case["events"].as_array().unwrap_or_else(|| panic!("{UNIT} events")) {
            if let Some(r) = ev.get("report") {
                let got = s.report();
                let want = (r["fraction"].as_u64(), r["lost"].as_i64(), r["extended_max"].as_u64(), r["jitter"].as_u64());
                assert_eq!((Some(u64::from(got.fraction)), Some(i64::from(got.lost)), Some(u64::from(got.extended_max)), Some(u64::from(got.jitter))), want, "{UNIT} rr {name}");
            } else {
                let valid = s.update(u(ev, "seq") as u16, u(ev, "ts") as u32, u(ev, "arrival_ms"), 100);
                assert_eq!(valid, u(ev, "valid") == 1, "{UNIT} rr {name} valid {ev}");
            }
        }
    }
}

#[test]
fn rtcp_twcc_vectors() {
    let d = load("rtcp.json");
    for case in set(&d, "twcc") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut t = TwccRecv::default();
        let mut built: Vec<oxsfud::rtcp::core::Feedback> = Vec::new();
        for ev in case["events"].as_array().unwrap_or_else(|| panic!("{UNIT} events")) {
            if let Some(b) = ev.get("build") {
                if built.is_empty() {
                    built = t.build();
                    built.reverse();
                }
                let fb = built.pop().unwrap_or_else(|| panic!("{UNIT} twcc {name} 피드백 모자람"));
                let syms: String = fb.symbols.iter().map(|s| s.letter()).collect();
                let deltas: Vec<i64> = fb.deltas_250us.iter().map(|x| i64::from(*x)).collect();
                let want_d: Vec<i64> = b["deltas_250us"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                assert_eq!(
                    (u64::from(fb.base_seq), u64::from(fb.status_count), u64::from(fb.reference_time), u64::from(fb.fb_count), syms.as_str(), deltas),
                    (u(b, "base_seq"), u(b, "status_count"), u(b, "reference_time"), u(b, "fb_count"), b["symbols"].as_str().unwrap_or(""), want_d),
                    "{UNIT} twcc {name}"
                );
            } else {
                t.record(u(ev, "seq") as u16, u(ev, "arrival_us"));
            }
        }
        assert!(built.is_empty(), "{UNIT} twcc {name} 남은 피드백");
    }
}

#[test]
fn sim_aimd_vectors() {
    let d = load("sim.json");
    for case in set(&d, "aimd") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut a = Aimd::new(u(case, "start_bps") as f64);
        for st in case["steps"].as_array().unwrap_or_else(|| panic!("{UNIT} steps")) {
            let (bps, near) = a.update(u(st, "at_ms"), Usage::parse(st["usage"].as_str().unwrap_or("normal")), u(st, "acked_bps") as f64);
            assert_eq!((bps, near), (u(st, "out_bps"), st["out_near"].as_bool().unwrap_or(false)), "{UNIT} aimd {name} step {st}");
        }
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn prop_switch_seq_contiguous_and_ts_forward() {
    let mut x = Lcg(0x5eed);
    for _ in 0..2000 {
        let mut r = Rewriter::default();
        let mut now = 0u64;
        let mut src = 0u64;
        let mut seq = (x.next() & 0xffff) as u16;
        let mut ts = x.next() as u32;
        let mut prev: Option<(u16, u32)> = None;
        for _ in 0..(5 + x.next() % 20) {
            let switching = prev.is_some() && x.next().is_multiple_of(4);
            if switching {
                src += 1;
                seq = (x.next() & 0xffff) as u16;
                ts = x.next() as u32;
            }
            now += x.next() % 60;
            let (os, ot) = r.map(src, seq, ts, now, 48000);
            if let Some((ps, pt)) = prev
                && switching
            {
                assert_eq!(os, ps.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 PROP-FWD-SWITCH-SEQ-CONTIGUOUS");
                let d = ot.wrapping_sub(pt);
                assert!((1..(1u32 << 31)).contains(&d), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 PROP-FWD-SWITCH-TS-FORWARD {d}");
            }
            prev = Some((os, ot));
            seq = seq.wrapping_add(1);
            ts = ts.wrapping_add(960);
        }
    }
}
