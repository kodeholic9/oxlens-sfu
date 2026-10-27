// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CONC-SANS-IO · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · 정:RULE-SIM-TICK · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-LOCK · model: claude-opus-5-5
//! 1층 — 서버 벡터 넷(`server/out/vectors/fwd · rtcp · sim · flr .json`)을 산수 함수에 그대로 넣는다. 판 묶음이 없거나 비면 빨강이다(0회 초록 금지).

use std::path::PathBuf;

use oxsfud::flr::bind::{Effect, Slot};
use oxsfud::fwd::core::{Rewriter, SrIn, covers, pt_byte1, serial_after, translate_sr};
use oxsfud::rtcp::core::{RecvStats, TwccRecv};
use oxsfud::sim::core::{Aimd, Usage};
use oxsfud::sub::ext::{ExtTable, build};
use serde_json::Value;

fn spec_root() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn load(name: &str) -> Value {
    let p = spec_root().join("server/out/vectors").join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터를 못 읽었다 {}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("벡터 JSON {}: {e}", p.display()))
}

fn set<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = v.get(key).and_then(Value::as_array).unwrap_or_else(|| panic!("판 묶음 `{key}` 이 없다"));
    assert!(!a.is_empty(), "판 묶음 `{key}` 이 비었다 — 0회 초록 금지");
    a
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_else(|| panic!("칸 `{k}` 이 없다: {v}"))
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex")).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn fwd_rewrite_vectors() {
    let v = load("fwd.json");
    let cases = set(&v, "rewrite");
    for c in cases {
        let name = c["name"].as_str().unwrap_or("?");
        let clock = u(c, "clock") as u32;
        let steps = c["steps"].as_array().expect("steps");
        assert!(!steps.is_empty(), "{name}: 걸음이 비었다");
        let mut r = Rewriter::new();
        for (i, s) in steps.iter().enumerate() {
            let payload = hex(s["payload"].as_str().expect("payload"));
            let cov = covers(clock, &payload);
            assert_eq!(u64::from(cov), u(s, "covers"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 {name} #{i} 덮는 길이(Opus TOC)");
            let m = r.map(u(s, "src"), u(s, "seq") as u16, u(s, "ts") as u32, u(s, "at_ms"), clock, cov);
            assert_eq!(u64::from(m.seq), u(s, "out_seq"), "S-sym-RULE-CONC-SANS-IO-1 {name} #{i} out_seq");
            assert_eq!(u64::from(m.ts), u(s, "out_ts"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 {name} #{i} out_ts");
        }
        let nacks = c["nack"].as_array().unwrap_or_else(|| panic!("{name}: nack 판이 없다"));
        assert!(!nacks.is_empty(), "{name}: nack 판이 비었다");
        for n in nacks {
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(r.nack_to_source(u(n, "out_seq") as u16), want, "S-sym-RULE-CONC-SANS-IO-1 {name} nack {n}");
        }
    }
}

#[test]
fn fwd_serial_pt_sr_vectors() {
    let v = load("fwd.json");
    for c in set(&v, "serial") {
        assert_eq!(serial_after(u(c, "a") as u16, u(c, "b") as u16), c["after"].as_bool().expect("after"), "S-sym-RULE-CONC-SANS-IO-1 serial {c}");
    }
    for c in set(&v, "pt_byte1") {
        assert_eq!(u64::from(pt_byte1(u(c, "byte1") as u8, u(c, "pt") as u8)), u(c, "out"), "S-sym-RULE-CONC-SANS-IO-1 pt_byte1 {c}");
    }
    for c in set(&v, "sr") {
        let i = &c["in"];
        let o = &c["out"];
        let got = translate_sr(SrIn { ssrc: u(i, "ssrc") as u32, ntp: u(i, "ntp"), rtp_ts: u(i, "rtp_ts") as u32 }, u(c, "downlink_ssrc") as u32, u(c, "ts_off") as u32);
        assert_eq!((u64::from(got.ssrc), got.ntp, u64::from(got.rtp_ts)), (u(o, "ssrc"), u(o, "ntp"), u(o, "rtp_ts")), "S-sym-RULE-CONC-SANS-IO-1 sr {c}");
    }
}

#[test]
fn sub_ext_vectors() {
    let v = load("fwd.json");
    for c in set(&v, "ext") {
        let name = c["name"].as_str().unwrap_or("?");
        let t = &c["table"];
        let id = |k: &str| t.get(k).and_then(Value::as_u64).map(|x| x as u8);
        let table = ExtTable { mid: id("mid"), audio_level: id("audio_level"), abs_send_time: id("abs_send_time"), twcc: id("twcc") };
        let got = build(&table, c["mid"].as_str(), c["audio_level_in"].as_u64().map(|x| x as u8), u(c, "t_us"), u(c, "twcc_counter"));
        assert_eq!(to_hex(&got), c["hex"].as_str().expect("hex"), "S-sym-RULE-CONC-SANS-IO-1 {name}");
        let form = if table.form() == oxsfud::sub::ext::Form::One { "one" } else { "two" };
        assert_eq!(form, c["form"].as_str().expect("form"), "{name} 꼴");
        assert_eq!((got.len() as u64 - 4) / 4, u(c, "words"), "{name} 길이(32비트)");
        let els = oxsfud::sub::ext::elements(&table, c["mid"].as_str(), c["audio_level_in"].as_u64().map(|x| x as u8), u(c, "t_us"), u(c, "twcc_counter"));
        let want: Vec<(u64, String)> = c["elements"].as_array().expect("elements").iter().map(|e| (u(e, "id"), e["hex"].as_str().expect("hex").to_string())).collect();
        let have: Vec<(u64, String)> = els.iter().map(|(i, b)| (u64::from(*i), to_hex(b))).collect();
        assert_eq!(have, want, "S-sym-RULE-CONC-SANS-IO-1 {name} 원소");
    }
}

#[test]
fn rtcp_rr_vectors() {
    let v = load("rtcp.json");
    for c in set(&v, "rr") {
        let name = c["name"].as_str().unwrap_or("?");
        let mut s = RecvStats::new(u(c, "clock") as u32);
        let mut reports = 0;
        for e in c["events"].as_array().expect("events") {
            if let Some(r) = e.get("report") {
                let got = s.report();
                reports += 1;
                assert_eq!(
                    (u64::from(got.fraction), i64::from(got.lost), u64::from(got.extended_max), u64::from(got.jitter)),
                    (u(r, "fraction"), r["lost"].as_i64().expect("lost"), u(r, "extended_max"), u(r, "jitter")),
                    "S-sym-RULE-CONC-SANS-IO-1 {name} RR"
                );
            } else {
                let valid = s.update(u(e, "seq") as u16, u(e, "ts") as u32, u(e, "arrival_ms"));
                assert_eq!(u64::from(valid), u(e, "valid"), "{name} 출처 인정(A.1) {e}");
            }
        }
        assert!(reports > 0, "{name}: report 걸음이 없다");
    }
}

#[test]
fn rtcp_twcc_vectors() {
    let v = load("rtcp.json");
    for c in set(&v, "twcc") {
        let name = c["name"].as_str().unwrap_or("?");
        let mut t = TwccRecv::new();
        let mut pending: Vec<oxsfud::rtcp::core::Feedback> = Vec::new();
        let mut builds = 0;
        for e in c["events"].as_array().expect("events") {
            if let Some(b) = e.get("build") {
                if pending.is_empty() {
                    pending = t.build();
                    pending.reverse();
                }
                let got = pending.pop().unwrap_or_else(|| panic!("{name}: 피드백이 모자란다"));
                builds += 1;
                let syms: String = got.symbols.iter().map(|s| s.letter()).collect();
                let deltas: Vec<i64> = b["deltas_250us"].as_array().expect("deltas").iter().map(|x| x.as_i64().expect("d")).collect();
                assert_eq!(
                    (u64::from(got.base_seq), u64::from(got.status_count), u64::from(got.reference_time), u64::from(got.fb_count), syms, got.deltas_250us.iter().map(|x| i64::from(*x)).collect::<Vec<_>>()),
                    (u(b, "base_seq"), u(b, "status_count"), u(b, "reference_time"), u(b, "fb_count"), b["symbols"].as_str().expect("sym").to_string(), deltas),
                    "S-sym-RULE-CONC-SANS-IO-1 {name} TWCC"
                );
            } else {
                assert!(pending.is_empty(), "{name}: 앞 피드백이 남았다");
                t.record(u(e, "seq") as u16, u(e, "arrival_us") as i64);
            }
        }
        assert!(pending.is_empty(), "{name}: 피드백이 남는다");
        assert!(builds > 0, "{name}: build 걸음이 없다");
    }
}

#[test]
fn sim_aimd_vectors() {
    let v = load("sim.json");
    let k = &v["constants"];
    assert_eq!(u(k, "floor_bps") as f64, oxsfud::sim::core::FLOOR_BPS);
    assert_eq!(u(k, "rtt_ms") as f64, oxsfud::sim::core::RTT_MS);
    assert_eq!(u(k, "packet_bytes") as f64, oxsfud::sim::core::PACKET_BYTES);
    assert_eq!(u(k, "fps") as f64, oxsfud::sim::core::FPS);
    for c in set(&v, "aimd") {
        let name = c["name"].as_str().unwrap_or("?");
        let mut a = Aimd::new(u(c, "start_bps") as f64);
        let steps = c["steps"].as_array().expect("steps");
        assert!(!steps.is_empty(), "{name}: 걸음이 비었다");
        for (i, s) in steps.iter().enumerate() {
            let usage = Usage::parse(s["usage"].as_str().expect("usage")).expect("usage");
            let out = a.update(usage, u(s, "acked_bps") as f64, u(s, "at_ms"));
            assert_eq!((out, a.near_max()), (u(s, "out_bps"), s["out_near"].as_bool().expect("near")), "S-sym-RULE-CONC-SANS-IO-1 {name} #{i}");
        }
    }
}

fn effects(decided: &[Value]) -> Vec<Effect> {
    decided
        .iter()
        .map(|d| match d[0].as_str().expect("kind") {
            "bind" => Effect::Bind(d[1].as_str().expect("u").to_string(), d[2].as_u64().expect("g")),
            "release" => Effect::Release(d[1].as_u64().expect("g")),
            k => panic!("모르는 효과 {k}"),
        })
        .collect()
}

fn bound_of(v: &Value) -> Option<(String, u64)> {
    v.as_array().map(|a| (a[0].as_str().expect("u").to_string(), a[1].as_u64().expect("g")))
}

#[test]
fn flr_media_bind_vectors() {
    let v = load("flr.json");
    for c in set(&v, "media_bind") {
        let name = c["name"].as_str().unwrap_or("?");
        let fx = effects(c["decided"].as_array().expect("decided"));
        let mut s = Slot::new();
        for e in &fx {
            s.apply(e);
        }
        assert_eq!(s.bound, bound_of(&c["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-3 {name} 판정 차례");
        let orders = c["orders"].as_array().expect("orders");
        assert!(!orders.is_empty(), "{name}: 집행 차례가 비었다");
        let mut naive_differs = 0;
        for o in orders {
            let order: Vec<usize> = o["order"].as_array().expect("order").iter().map(|x| x.as_u64().expect("i") as usize).collect();
            let mut s = Slot::new();
            let applied: Vec<bool> = order.iter().map(|i| s.apply(&fx[*i])).collect();
            let want: Vec<bool> = o["applied"].as_array().expect("applied").iter().map(|x| x.as_bool().expect("b")).collect();
            assert_eq!(applied, want, "S-sym-IMPL-FLR-MEDIA-BIND-4 {name} {order:?} 효과마다 적용");
            assert_eq!(s.bound, bound_of(&o["bound"]), "S-sym-IMPL-FLR-MEDIA-BIND-3 {name} {order:?} 끝 슬롯");
            let mut naive: Option<(String, u64)> = None;
            for i in &order {
                match &fx[*i] {
                    Effect::Bind(u, g) => naive = Some((u.clone(), *g)),
                    Effect::Release(_) => naive = None,
                }
            }
            if naive != s.bound {
                naive_differs += 1;
            }
        }
        assert_eq!(naive_differs, u(c, "targetless_differs"), "S-sym-IMPL-FLR-MEDIA-BIND-3 {name} 번호를 안 보는 집행이 갈리는 차례 수");
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

#[test]
fn prop_switch_seq_contiguous_and_ts_forward() {
    let mut rng = Lcg(20_261_027);
    let mut switches = 0;
    for round in 0..200 {
        let mut r = Rewriter::new();
        let mut now: u64 = rng.next() % 10_000;
        let mut last_out: Option<(u16, u32, u32, u64)> = None;
        let mut src: u64 = 1;
        let mut seq = rng.next() as u16;
        let mut ts = rng.next() as u32;
        for _ in 0..60 {
            if rng.next().is_multiple_of(5) {
                src += 1;
                seq = rng.next() as u16;
                ts = rng.next() as u32;
                now += rng.next() % 50;
            } else {
                seq = seq.wrapping_add(1);
                ts = ts.wrapping_add(960);
                now += 20;
            }
            let payload = [[0xf8u8].as_slice(), &[0x10, 0x00], &[0x18], &[0xe0]][(rng.next() % 4) as usize];
            let cov = covers(48_000, payload);
            let m = r.map(src, seq, ts, now, 48_000, cov);
            if let Some((ls, lts, lcov, lat)) = last_out
                && m.switched
            {
                switches += 1;
                assert_eq!(m.seq, ls.wrapping_add(1), "PROP-FWD-SWITCH-SEQ-CONTIGUOUS round {round}");
                let gap = m.ts.wrapping_sub(lts);
                let elapsed = (now - lat).max(1) * 48;
                assert_eq!(u64::from(gap), elapsed.max(u64::from(lcov)), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 PROP-FWD-SWITCH-TS-FORWARD round {round}");
                assert!(gap >= lcov, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 앞 패킷이 덮는 길이 안으로 겹치지 않는다");
            }
            last_out = Some((m.seq, m.ts, cov, now));
        }
    }
    assert!(switches > 100, "성질 시험이 전환을 충분히 만들지 못했다({switches})");
}
