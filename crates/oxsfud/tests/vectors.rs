// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CONC-SANS-IO · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 1층 — 규격 벡터(`server/out/vectors/fwd.json` · `rtcp.json`)를 sans-IO 함수에 그대로 넣는다 · 전환 성질 둘은 임의 출처열로 잰다. 벡터가 없으면 실패한다(건너뛰지 않는다).

use std::path::PathBuf;

use oxsfud::fwd::core::{pt_byte1, seq_after, translate_sr, Rewriter, Sr};
use oxsfud::rtcp::core::{RecvStats, Sym, TwccRecv};
use oxsfud::sub::ext::{build, ExtTable, ExtValues, Form};
use serde_json::Value;

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn load(name: &str) -> Value {
    let p = spec_dir().join("server/out/vectors").join(name);
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 {}: {e} — OXLENS_SPEC 로 규격 위치를 준다", p.display()));
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("벡터 {}: {e}", p.display()))
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_else(|| panic!("{k} 없음: {v}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn fwd_rewrite_cases() {
    let v = load("fwd.json");
    let cases = v["rewrite"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty(), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 rewrite 판 없음");
    for c in cases {
        let name = c["name"].as_str().unwrap_or("");
        let clock = u(&c, "clock") as u32;
        let mut rw = Rewriter::default();
        for s in c["steps"].as_array().cloned().unwrap_or_default() {
            let m = rw.map(u(&s, "src"), u(&s, "seq") as u16, u(&s, "ts") as u32, clock, u(&s, "at_ms"));
            assert_eq!((m.seq as u64, m.ts as u64), (u(&s, "out_seq"), u(&s, "out_ts")), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 S-sym-IMPL-FWD-SOURCE-SWITCH-2 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS 정:PROP-FWD-SWITCH-TS-FORWARD [{name}] {s}");
        }
        for n in c["nack"].as_array().cloned().unwrap_or_default() {
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(rw.nack(u(&n, "out_seq") as u16), want, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 NACK 되돌림 [{name}] {n}");
        }
    }
}

#[test]
fn fwd_serial_pt_sr() {
    let v = load("fwd.json");
    for s in v["serial"].as_array().cloned().unwrap_or_default() {
        assert_eq!(seq_after(u(&s, "a") as u16, u(&s, "b") as u16), s["after"].as_bool().unwrap_or(false), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 RFC 1982 serial {s}");
    }
    for p in v["pt_byte1"].as_array().cloned().unwrap_or_default() {
        assert_eq!(pt_byte1(u(&p, "byte1") as u8, u(&p, "pt") as u8) as u64, u(&p, "out"), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 PT 교체 {p}");
    }
    for s in v["sr"].as_array().cloned().unwrap_or_default() {
        let i = &s["in"];
        let out = translate_sr(Sr { ssrc: u(i, "ssrc") as u32, ntp: u(i, "ntp"), rtp_ts: u(i, "rtp_ts") as u32 }, u(&s, "downlink_ssrc") as u32, u(&s, "ts_off") as u32);
        let o = &s["out"];
        assert_eq!((out.ssrc as u64, out.ntp, out.rtp_ts as u64), (u(o, "ssrc"), u(o, "ntp"), u(o, "rtp_ts")), "S-sym-IMPL-RTCP-TERMINATE-1 SR 번역 {s}");
    }
}

#[test]
fn fwd_ext_cases() {
    let v = load("fwd.json");
    let cases = v["ext"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty());
    for c in cases {
        let name = c["name"].as_str().unwrap_or("");
        let t = &c["table"];
        let id = |k: &str| t.get(k).and_then(Value::as_u64).map(|x| x as u8);
        let table = ExtTable { mid: id("mid"), audio_level: id("audio_level"), abs_send_time: id("abs_send_time"), twcc: id("twcc") };
        let form = if c["form"] == "two" { Form::Two } else { Form::One };
        assert_eq!(table.form(), form, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 꼴 [{name}]");
        let mid = c["mid"].as_str().map(str::to_string);
        let vals = ExtValues { mid: mid.as_deref(), audio_level: c["audio_level_in"].as_u64().map(|x| x as u8), t_us: u(&c, "t_us"), twcc_seq: (u(&c, "twcc_counter") % 65536) as u16 };
        let mut out = Vec::new();
        build(&table, form, &vals, &mut out);
        assert_eq!(hex(&out), c["hex"].as_str().unwrap_or(""), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 S-sym-IMPL-SUB-SUBSCRIBER-TABLE-2 S-sym-RULE-CONC-SANS-IO-1 확장 다시 짓기 [{name}]");
    }
}

#[test]
fn rtcp_rr_cases() {
    let v = load("rtcp.json");
    let cases = v["rr"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty());
    for c in cases {
        let name = c["name"].as_str().unwrap_or("");
        let clock = u(&c, "clock") as u32;
        let mut st = RecvStats::default();
        for e in c["events"].as_array().cloned().unwrap_or_default() {
            if let Some(r) = e.get("report") {
                let got = st.report();
                assert_eq!(got.fraction as u64, u(r, "fraction"), "S-sym-IMPL-RTCP-TERMINATE-6 RR fraction [{name}]");
                assert_eq!(got.lost as i64, r["lost"].as_i64().unwrap_or(i64::MIN), "S-sym-IMPL-RTCP-TERMINATE-6 RR lost [{name}]");
                assert_eq!(got.extended_max as u64, u(r, "extended_max"), "S-sym-IMPL-RTCP-TERMINATE-6 RR extended_max [{name}]");
                assert_eq!(got.jitter as u64, u(r, "jitter"), "S-sym-IMPL-RTCP-TERMINATE-6 RR jitter [{name}]");
                continue;
            }
            let valid = st.update(u(&e, "seq") as u16, u(&e, "ts") as u32, u(&e, "arrival_ms"), clock);
            assert_eq!(valid as u64, u(&e, "valid"), "S-sym-IMPL-RTCP-TERMINATE-6 A.1 출처 인정 [{name}] {e}");
        }
    }
}

fn sym(s: Sym) -> char {
    match s {
        Sym::N => 'N',
        Sym::S => 'S',
        Sym::L => 'L',
    }
}

#[test]
fn rtcp_twcc_cases() {
    let v = load("rtcp.json");
    let cases = v["twcc"].as_array().cloned().unwrap_or_default();
    assert!(!cases.is_empty());
    for c in cases {
        let name = c["name"].as_str().unwrap_or("");
        let mut tw = TwccRecv::default();
        let mut pending: Vec<oxsfud::rtcp::core::Feedback> = Vec::new();
        for e in c["events"].as_array().cloned().unwrap_or_default() {
            if let Some(b) = e.get("build") {
                if pending.is_empty() {
                    pending = tw.build();
                    pending.reverse();
                }
                let got = pending.pop().unwrap_or_else(|| panic!("S-sym-IMPL-RTCP-TERMINATE-3 피드백 없음 [{name}]"));
                let syms: String = got.symbols.iter().map(|s| sym(*s)).collect();
                assert_eq!(got.base_seq as u64, u(b, "base_seq"), "S-sym-IMPL-RTCP-TERMINATE-3 base [{name}]");
                assert_eq!(got.status_count as u64, u(b, "status_count"), "S-sym-IMPL-RTCP-TERMINATE-3 count [{name}]");
                assert_eq!(got.reference_time as u64, u(b, "reference_time"), "S-sym-IMPL-RTCP-TERMINATE-3 ref [{name}]");
                assert_eq!(got.fb_count as u64, u(b, "fb_count"), "S-sym-IMPL-RTCP-TERMINATE-3 fb_count [{name}]");
                assert_eq!(syms, b["symbols"].as_str().unwrap_or(""), "S-sym-IMPL-RTCP-TERMINATE-3 symbols [{name}]");
                let deltas: Vec<i64> = b["deltas_250us"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_i64).collect();
                assert_eq!(got.deltas.iter().map(|d| *d as i64).collect::<Vec<_>>(), deltas, "S-sym-IMPL-RTCP-TERMINATE-3 deltas [{name}]");
                continue;
            }
            tw.record(u(&e, "seq") as u16, u(&e, "arrival_us"));
        }
        assert!(pending.is_empty(), "S-sym-IMPL-RTCP-TERMINATE-3 남은 피드백 [{name}]");
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
fn prop_switch_contiguous_and_forward() {
    let mut r = Lcg(0x5eed);
    for round in 0..500 {
        let clock = if round % 2 == 0 { 48_000 } else { 90_000 };
        let mut rw = Rewriter::default();
        let mut at = r.next() % 10_000;
        let mut src = 1u64;
        let mut seq = (r.next() % 65536) as u16;
        let mut ts = r.next() as u32;
        let mut best: Option<(u16, u32)> = None;
        for _ in 0..200 {
            if r.next().is_multiple_of(13) {
                src += 1;
                seq = (r.next() % 65536) as u16;
                ts = r.next() as u32;
                at += r.next() % 50;
                let m = rw.map(src, seq, ts, clock, at);
                if let Some((ls, lt)) = best {
                    assert_eq!(m.seq, ls.wrapping_add(1), "정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS S-sym-IMPL-FWD-SOURCE-SWITCH-2 round {round}");
                    let d = m.ts.wrapping_sub(lt) as i32;
                    assert!(d >= 1, "정:PROP-FWD-SWITCH-TS-FORWARD S-sym-IMPL-FWD-SOURCE-SWITCH-2 round {round} d={d}");
                }
                best = Some((m.seq, m.ts));
                continue;
            }
            let back = r.next().is_multiple_of(7);
            let s = if back { seq.wrapping_sub(2) } else { seq.wrapping_add(1) };
            if !back {
                seq = s;
                ts = ts.wrapping_add(960);
            }
            at += 20;
            let m = rw.map(src, s, if back { ts.wrapping_sub(1920) } else { ts }, clock, at);
            if !back || best.is_none() {
                best = Some((m.seq, m.ts));
            }
        }
    }
}
