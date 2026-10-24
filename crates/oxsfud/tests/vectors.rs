// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CONC-SANS-IO · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · 정:RULE-SIM-TICK · 정:CAND-DOC-GUARD-PLACE · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/*.json` 판을 그대로 넣어 같은 출력. 판 파일 · 판 묶음이 없거나 비면 빨강.

use oxsfud::fwd::core::{self as fwd, ExtTable, ExtValues, Rewriter, Sr};
use oxsfud::rtcp::core::{RecvStats, TwccRecv};
use oxsfud::sim::core::{Aimd, Usage};
use serde_json::Value;

fn load(name: &str) -> Value {
    let p = common::spec_root().join("server/out/vectors").join(name);
    let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 {} 를 못 읽는다: {e}", p.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("벡터 {} 형: {e}", p.display()))
}

fn group<'a>(doc: &'a Value, key: &str) -> &'a Vec<Value> {
    let g = doc.get(key).and_then(Value::as_array).unwrap_or_else(|| panic!("판 묶음 {key} 이 없다"));
    assert!(!g.is_empty(), "판 묶음 {key} 이 비었다");
    g
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_else(|| panic!("칸 {k} 가 없다: {v}"))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn fwd_rewrite() {
    let doc = load("fwd.json");
    for case in group(&doc, "rewrite") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut rw = Rewriter::new(u(case, "clock") as u32);
        for st in case["steps"].as_array().expect("steps") {
            let (os, ot) = rw.rewrite(u(st, "src"), u(st, "seq") as u16, u(st, "ts") as u32, u(st, "at_ms"));
            assert_eq!(os as u64, u(st, "out_seq"), "정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS {name}: {st}");
            assert_eq!(ot as u64, u(st, "out_ts"), "정:PROP-FWD-SWITCH-TS-FORWARD {name}: {st}");
        }
        for n in case["nack"].as_array().expect("nack") {
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(rw.nack_in(u(n, "out_seq") as u16), want, "정:IMPL-FWD-SOURCE-SWITCH NACK 되돌림 {name}: {n}");
        }
    }
}

#[test]
fn fwd_serial_pt_sr() {
    let doc = load("fwd.json");
    for c in group(&doc, "serial") {
        assert_eq!(fwd::seq_after(u(c, "a") as u16, u(c, "b") as u16), c["after"].as_bool().expect("after"), "RFC 1982 serial: {c}");
    }
    for c in group(&doc, "pt_byte1") {
        assert_eq!(fwd::pt_byte1(u(c, "byte1") as u8, u(c, "pt") as u8) as u64, u(c, "out"), "byte1 하위 7비트만: {c}");
    }
    for c in group(&doc, "sr") {
        let i = &c["in"];
        let o = &c["out"];
        let got = fwd::translate_sr(Sr { ssrc: u(i, "ssrc") as u32, ntp: u(i, "ntp"), rtp_ts: u(i, "rtp_ts") as u32 }, u(c, "downlink_ssrc") as u32, u(c, "ts_off") as u32);
        assert_eq!(got, Sr { ssrc: u(o, "ssrc") as u32, ntp: u(o, "ntp"), rtp_ts: u(o, "rtp_ts") as u32 }, "SR 번역 — NTP 원본 · RTP 같은 오프셋: {c}");
    }
}

#[test]
fn fwd_ext_rebuild() {
    let doc = load("fwd.json");
    for c in group(&doc, "ext") {
        let name = c["name"].as_str().unwrap_or("?");
        let t = &c["table"];
        let id = |k: &str| t.get(k).and_then(Value::as_u64).map(|x| x as u8);
        let table = ExtTable { mid: id("mid"), audio_level: id("audio_level"), abs_send_time: id("abs_send_time"), twcc: id("twcc") };
        let v = ExtValues {
            mid: c["mid"].as_str().map(str::to_string),
            audio_level: c["audio_level_in"].as_u64().map(|x| x as u8),
            t_us: u(c, "t_us"),
            twcc: Some((u(c, "twcc_counter") & 0xffff) as u16),
        };
        let form = if table.two_byte() { "two" } else { "one" };
        assert_eq!(form, c["form"].as_str().expect("form"), "정:IMPL-SUB-SUBSCRIBER-TABLE 꼴은 표로 정한다 {name}");
        let elems = fwd::ext_elements(&table, &v);
        let want: Vec<(u64, String)> = c["elements"].as_array().expect("elements").iter().map(|e| (u(e, "id"), e["hex"].as_str().unwrap_or("").to_string())).collect();
        let got: Vec<(u64, String)> = elems.iter().map(|(i, d)| (*i as u64, hex(d))).collect();
        assert_eq!(got, want, "원소 {name}");
        let block = fwd::ext_block(&table, &v);
        assert_eq!(((block.len() - 4) / 4) as u64, u(c, "words"), "32비트 단위 {name}");
        assert_eq!(hex(&block), c["hex"].as_str().expect("hex"), "S-sym-RULE-CONC-SANS-IO-1 같은 판 같은 출력 {name}");
    }
}

#[test]
fn rtcp_rr() {
    let doc = load("rtcp.json");
    for case in group(&doc, "rr") {
        let name = case["name"].as_str().unwrap_or("?");
        let clock = u(case, "clock") as u32;
        let mut st = RecvStats::default();
        for ev in case["events"].as_array().expect("events") {
            if let Some(r) = ev.get("report") {
                let got = st.report();
                assert_eq!(got.fraction as u64, u(r, "fraction"), "fraction {name}");
                assert_eq!(got.lost as i64, r["lost"].as_i64().expect("lost"), "누적 손실(부호 있음) {name}");
                assert_eq!(got.extended_max as u64, u(r, "extended_max"), "extended_max {name}");
                assert_eq!(got.jitter as u64, u(r, "jitter"), "정:IMPL-RTCP-TERMINATE 지터 A.8 {name}");
                continue;
            }
            let valid = st.on_packet(u(ev, "seq") as u16, u(ev, "ts") as u32, u(ev, "arrival_ms"), clock);
            assert_eq!(valid as u64, u(ev, "valid"), "A.1 출처 인정 {name}: {ev}");
        }
    }
}

#[test]
fn rtcp_twcc() {
    let doc = load("rtcp.json");
    for case in group(&doc, "twcc") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut rx = TwccRecv::default();
        let mut queue = std::collections::VecDeque::new();
        for ev in case["events"].as_array().expect("events") {
            if let Some(b) = ev.get("build") {
                if queue.is_empty() {
                    queue.extend(rx.build());
                }
                let fb = queue.pop_front().unwrap_or_else(|| panic!("피드백이 없다 {name}"));
                let symbols: String = fb.symbols.iter().map(|s| s.letter()).collect();
                let deltas: Vec<i64> = b["deltas_250us"].as_array().expect("deltas").iter().map(|d| d.as_i64().expect("delta")).collect();
                assert_eq!(fb.base_seq as u64, u(b, "base_seq"), "base {name}");
                assert_eq!(fb.status_count as u64, u(b, "status_count"), "상태 수 {name}");
                assert_eq!(fb.reference_time as u64, u(b, "reference_time"), "기준 시각 {name}");
                assert_eq!(fb.fb_count as u64, u(b, "fb_count"), "피드백 수 {name}");
                assert_eq!(symbols, b["symbols"].as_str().expect("symbols"), "상태 {name}");
                assert_eq!(fb.deltas.iter().map(|d| *d as i64).collect::<Vec<_>>(), deltas, "정:IMPL-RTCP-TERMINATE 수신 델타 {name}");
                continue;
            }
            rx.on_packet(u(ev, "seq") as u16, u(ev, "arrival_us") as i64);
        }
        assert!(queue.is_empty(), "남은 피드백 {name}");
    }
}

#[test]
fn sim_aimd() {
    let doc = load("sim.json");
    let k = &doc["constants"];
    assert_eq!(u(k, "floor_bps"), oxsfud::sim::core::FLOOR_BPS);
    assert_eq!(u(k, "rtt_ms") as f64, oxsfud::sim::core::RTT_MS);
    for case in group(&doc, "aimd") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut a = Aimd::new(u(case, "start_bps"));
        for st in case["steps"].as_array().expect("steps") {
            let usage = Usage::parse(st["usage"].as_str().expect("usage")).expect("usage 값");
            let got = a.update(usage, u(st, "acked_bps"), u(st, "at_ms"));
            assert_eq!(got, u(st, "out_bps"), "정:RULE-SIM-TICK AIMD {name}: {st}");
            assert_eq!(a.near(), st["out_near"].as_bool().expect("out_near"), "수렴 근처 {name}: {st}");
        }
    }
}
