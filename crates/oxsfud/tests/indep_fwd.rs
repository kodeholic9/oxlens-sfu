// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-CONC-SANS-IO · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 맹검 독립 1층 — 재기록기(단일 오프셋 · 전환 하한 = 앞 패킷이 덮는 길이 · serial 비교 · NACK 되돌림 · SR 번역) · PT byte1 · 신탁은 규격 식에서 지어 fwd.json 과 먼저 맞댄다.

use oxsfud::fwd::core::{Rewriter, covers, opus_toc_samples, pt_byte1, serial_after};
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    match std::env::var("OXLENS_SPEC_VECTORS") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec/server/out/vectors"),
    }
}

fn load(name: &str) -> serde_json::Value {
    let p = vectors_dir().join(name);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("vector file {} unreadable: {e}", p.display()));
    serde_json::from_str(&text).expect("vector json")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex")).collect()
}

fn o_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

fn o_toc(p: &[u8]) -> u32 {
    if p.is_empty() {
        return 1;
    }
    let config = p[0] >> 3;
    let code = p[0] & 3;
    let frame: u32 = if config < 12 {
        [480, 960, 1920, 2880][(config % 4) as usize]
    } else if config < 16 {
        [480, 960][(config % 2) as usize]
    } else {
        [120, 240, 480, 960][(config % 4) as usize]
    };
    let n = match code {
        0 => 1,
        1 | 2 => 2,
        _ => {
            if p.len() < 2 {
                return 1;
            }
            (p[1] & 0x3f) as u32
        }
    };
    frame * n
}

#[derive(Debug, Clone, Copy)]
struct OLast {
    seq: u16,
    ts: u32,
    at: u64,
    cov: u32,
}

#[derive(Debug, Clone, Default)]
struct ORw {
    clock: u32,
    src: Option<u64>,
    so: u16,
    to: u32,
    last: Option<OLast>,
    boundary: Option<u16>,
}

impl ORw {
    fn new(clock: u32) -> ORw {
        ORw { clock, ..ORw::default() }
    }
    fn map(&mut self, src: u64, seq: u16, ts: u32, at: u64, cov: u32) -> (u16, u32, bool) {
        let mut sw = false;
        match self.src {
            None => {
                self.src = Some(src);
                self.boundary = Some(seq);
            }
            Some(cur) if cur != src => {
                let l = self.last.expect("last after first packet");
                let el = at.saturating_sub(l.at).max(1);
                let tick = (el * self.clock as u64 / 1000).max(l.cov as u64);
                self.so = l.seq.wrapping_add(1).wrapping_sub(seq);
                self.to = (l.ts as u64).wrapping_add(tick).wrapping_sub(ts as u64) as u32;
                self.src = Some(src);
                self.boundary = Some(l.seq.wrapping_add(1));
                sw = true;
            }
            _ => {}
        }
        let os = seq.wrapping_add(self.so);
        let ot = ts.wrapping_add(self.to);
        if sw || self.last.is_none_or(|l| o_after(os, l.seq)) {
            self.last = Some(OLast { seq: os, ts: ot, at, cov });
        }
        (os, ot, sw)
    }
    fn nack(&self, out: u16) -> Option<u16> {
        if let Some(b) = self.boundary
            && o_after(b, out)
        {
            return None;
        }
        Some(out.wrapping_sub(self.so))
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_opus_payload(rng: &mut Rng) -> Vec<u8> {
    match rng.below(10) {
        0 => Vec::new(),
        1 => vec![((rng.below(32) as u8) << 3) | 3],
        2 => vec![((rng.below(32) as u8) << 3) | 3, 1 + rng.below(6) as u8, 0xaa],
        _ => {
            let config = rng.below(32) as u8;
            let code = rng.below(3) as u8;
            vec![(config << 3) | code, 0x11, 0x22]
        }
    }
}

#[test]
fn oracle_matches_vectors() {
    let v = load("fwd.json");
    for s in v["serial"].as_array().expect("serial") {
        let a = s["a"].as_u64().expect("a") as u16;
        let b = s["b"].as_u64().expect("b") as u16;
        assert_eq!(o_after(a, b), s["after"].as_bool().expect("after"), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 oracle serial {a} {b}");
    }
    for case in v["rewrite"].as_array().expect("rewrite") {
        let name = case["name"].as_str().unwrap_or("");
        let clock = case["clock"].as_u64().expect("clock") as u32;
        let mut o = ORw::new(clock);
        for st in case["steps"].as_array().expect("steps") {
            let p = hex(st["payload"].as_str().expect("payload"));
            let cov = if clock == 48_000 { o_toc(&p) } else { 1 };
            assert_eq!(cov as u64, st["covers"].as_u64().expect("covers"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 oracle covers · {name}");
            let (os, ot, _) = o.map(
                st["src"].as_u64().expect("src"),
                st["seq"].as_u64().expect("seq") as u16,
                st["ts"].as_u64().expect("ts") as u32,
                st["at_ms"].as_u64().expect("at"),
                cov,
            );
            assert_eq!(os as u64, st["out_seq"].as_u64().expect("out_seq"), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 oracle out_seq · {name}");
            assert_eq!(ot as u64, st["out_ts"].as_u64().expect("out_ts"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 oracle out_ts · {name}");
        }
        for n in case["nack"].as_array().expect("nack") {
            let out = n["out_seq"].as_u64().expect("out") as u16;
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(o.nack(out), want, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 oracle nack · {name} · {out}");
        }
    }
}

#[test]
fn product_matches_vectors() {
    let v = load("fwd.json");
    for s in v["serial"].as_array().expect("serial") {
        let a = s["a"].as_u64().expect("a") as u16;
        let b = s["b"].as_u64().expect("b") as u16;
        assert_eq!(serial_after(a, b), s["after"].as_bool().expect("after"), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 serial_after {a} {b}");
    }
    for p in v["pt_byte1"].as_array().expect("pt_byte1") {
        let b1 = p["byte1"].as_u64().expect("byte1") as u8;
        let pt = p["pt"].as_u64().expect("pt") as u8;
        assert_eq!(pt_byte1(b1, pt) as u64, p["out"].as_u64().expect("out"), "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 pt_byte1 {b1} {pt}");
    }
    for sr in v["sr"].as_array().expect("sr") {
        let rw = Rewriter { ts_off: sr["ts_off"].as_u64().expect("ts_off") as u32, ..Rewriter::new(90_000) };
        let got = rw.translate_sr_ts(sr["in"]["rtp_ts"].as_u64().expect("rtp_ts") as u32);
        assert_eq!(got as u64, sr["out"]["rtp_ts"].as_u64().expect("out rtp_ts"), "S-sym-IMPL-RTCP-TERMINATE-1 SR rtp_ts translated with the RTP offset");
    }
    for case in v["rewrite"].as_array().expect("rewrite") {
        let name = case["name"].as_str().unwrap_or("");
        let clock = case["clock"].as_u64().expect("clock") as u32;
        let mut rw = Rewriter::new(clock);
        for st in case["steps"].as_array().expect("steps") {
            let p = hex(st["payload"].as_str().expect("payload"));
            let cov = covers(clock == 48_000, &p);
            assert_eq!(cov as u64, st["covers"].as_u64().expect("covers"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 covers · {name} · {}", st["payload"]);
            let m = rw.map(
                st["src"].as_u64().expect("src"),
                st["seq"].as_u64().expect("seq") as u16,
                st["ts"].as_u64().expect("ts") as u32,
                st["at_ms"].as_u64().expect("at"),
                cov,
            );
            assert_eq!(m.seq as u64, st["out_seq"].as_u64().expect("out_seq"), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 out_seq · {name}");
            assert_eq!(m.ts as u64, st["out_ts"].as_u64().expect("out_ts"), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 out_ts · {name}");
        }
        for n in case["nack"].as_array().expect("nack") {
            let out = n["out_seq"].as_u64().expect("out") as u16;
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(rw.unmap_nack(out), want, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 unmap_nack · {name} · {out}");
        }
    }
}

#[test]
fn toc_samples_match_rfc6716_table() {
    let mut rng = Rng(0x0716_0716_0716_0716);
    for config in 0u8..32 {
        for code in 0u8..4 {
            for second in [None, Some(0u8), Some(1), Some(3), Some(0x45)] {
                let mut p = vec![(config << 3) | code];
                if let Some(b) = second {
                    p.push(b);
                }
                if code == 3 && second == Some(0) {
                    continue;
                }
                assert_eq!(covers(true, &p), o_toc(&p), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 covers(opus) config {config} code {code} second {second:?}");
            }
        }
    }
    assert_eq!(covers(true, &[]), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 empty payload covers 1");
    assert_eq!(covers(true, &[0xf8]), 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 OPUS_SILENCE f8 covers 960");
    assert_eq!(covers(true, &[0xf8, 0xff, 0xfe]), 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 priming frame covers 960");
    for _ in 0..200 {
        let p = random_opus_payload(&mut rng);
        assert_eq!(covers(false, &p), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 video covers 1 · {p:?}");
        if !(p.len() >= 2 && p[0] & 3 == 3 && p[1] & 0x3f == 0) {
            assert_eq!(opus_toc_samples(&p).max(1), o_toc(&p).max(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 opus_toc_samples · {p:?}");
        }
    }
}

#[test]
fn switch_after_prime_frame_does_not_overlap() {
    for gap_ms in 0u64..20 {
        let mut rw = Rewriter::new(48_000);
        let prime = rw.map(1, 500, 10_000, 1_000, covers(true, &[0xf8, 0xff, 0xfe]));
        let voice = rw.map(2, 7_000, 3_000_000, 1_000 + gap_ms, covers(true, &[0x78, 0x01]));
        let d = voice.ts.wrapping_sub(prime.ts);
        assert!(voice.switched, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 second source is a switch");
        assert!(d >= 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 first voice ts must start after the 20ms CN frame ends · gap {gap_ms}ms · diff {d}");
        let want = (gap_ms.max(1) * 48).max(960) as u32;
        assert_eq!(d, want, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 tick = max(covers, elapsed) · gap {gap_ms}ms");
        assert_eq!(voice.seq, prime.seq.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 seq contiguous at switch");
    }
    let mut rw = Rewriter::new(48_000);
    let a = rw.map(1, 1, 0, 0, covers(true, &[(2 << 3), 0]));
    let b = rw.map(9, 1, 0, 10, covers(true, &[0xf8]));
    assert_eq!(b.ts.wrapping_sub(a.ts), 1920, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 SILK 40ms frame then switch 10ms later: tick 1920");
}

#[test]
fn many_switches_keep_egress_contiguous_and_forward() {
    let mut rw = Rewriter::new(48_000);
    let mut at = 0u64;
    let mut prev: Option<oxsfud::fwd::core::Mapped> = None;
    let mut bases = [(65_000u16, 4_294_960_000u32), (100, 5), (30_000, 2_000_000_000)];
    for i in 0..3_000u64 {
        let src = ((i / 3) % 3) as usize;
        let (s, t) = bases[src];
        let m = rw.map(src as u64 + 1, s, t, at, 960);
        bases[src] = (s.wrapping_add(1), t.wrapping_add(960));
        if let Some(p) = prev {
            assert_eq!(m.seq, p.seq.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 egress seq never regresses across switches · i {i}");
            assert!(o_after(m.seq, p.seq), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 egress seq is serial-after previous · i {i}");
            let dt = m.ts.wrapping_sub(p.ts);
            assert!((960..1 << 31).contains(&dt),"S-sym-IMPL-FWD-SOURCE-SWITCH-2 ts never goes back and never overlaps · i {i} · dt {dt}");
        }
        prev = Some(m);
        at += 20;
    }
}

#[test]
fn random_streams_match_oracle() {
    let mut rng = Rng(0x2026_1030_f00d_0003);
    for round in 0..300 {
        let clock = if rng.below(4) == 0 { 90_000 } else { 48_000 };
        let opus = clock == 48_000;
        let mut rw = Rewriter::new(clock);
        let mut o = ORw::new(clock);
        let nsrc = 1 + rng.below(4);
        let mut st: Vec<(u16, u32)> = (0..nsrc).map(|_| (rng.next() as u16, rng.next() as u32)).collect();
        let mut at = rng.below(1 << 40);
        let mut cur = 0usize;
        let mut run_emitted: Vec<(u16, u16)> = Vec::new();
        let mut held: Option<(u16, u32, Vec<u8>)> = None;
        for step in 0..120 {
            if rng.below(8) == 0 {
                if let Some((s, t, p)) = held.take() {
                    let cov = covers(opus, &p);
                    let m = rw.map(cur as u64 + 100, s, t, at, cov);
                    let (os, ot, _) = o.map(cur as u64 + 100, s, t, at, cov);
                    assert_eq!((m.seq, m.ts), (os, ot), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 late packet vs oracle · round {round} step {step}");
                    run_emitted.push((m.seq, s));
                }
                cur = rng.below(nsrc) as usize;
                run_emitted.clear();
            }
            at += match rng.below(10) {
                0 => rng.below(5_000),
                1 => 0,
                _ => rng.below(40),
            };
            let (s, t) = st[cur];
            let p = if opus { random_opus_payload(&mut rng) } else { vec![0x90, 0x80] };
            let step_ts = if opus { o_toc(&p).max(1) } else { 3_000 };
            st[cur] = (s.wrapping_add(1), t.wrapping_add(step_ts));
            if held.is_none() && !run_emitted.is_empty() && rng.below(6) == 0 {
                held = Some((s, t, p));
                continue;
            }
            assert_eq!(covers(opus, &p), if opus { o_toc(&p) } else { 1 }, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 covers vs oracle · {p:?}");
            let cov = covers(opus, &p);
            let m = rw.map(cur as u64 + 100, s, t, at, cov);
            let (os, ot, sw) = o.map(cur as u64 + 100, s, t, at, cov);
            assert_eq!(m.seq, os, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 out seq vs oracle · round {round} step {step}");
            assert_eq!(m.ts, ot, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 out ts vs oracle · round {round} step {step} · switched {sw}");
            assert_eq!(m.switched, sw, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 switched flag vs oracle · round {round} step {step}");
            if sw {
                run_emitted.clear();
            }
            run_emitted.push((m.seq, s));
            if let Some((s2, t2, p2)) = held.take() {
                let cov2 = covers(opus, &p2);
                let m2 = rw.map(cur as u64 + 100, s2, t2, at, cov2);
                let (os2, ot2, _) = o.map(cur as u64 + 100, s2, t2, at, cov2);
                assert_eq!((m2.seq, m2.ts), (os2, ot2), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 reordered packet vs oracle · round {round} step {step}");
                run_emitted.push((m2.seq, s2));
            }
            for &(out, inp) in &run_emitted {
                assert_eq!(rw.unmap_nack(out), Some(inp), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 NACK of current source maps back · round {round} step {step}");
            }
            if let Some(b) = o.boundary
                && o.last.is_some()
                && sw
            {
                for k in 1..5u16 {
                    let before = b.wrapping_sub(k);
                    assert_eq!(rw.unmap_nack(before), None, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 NACK before switch boundary dropped · round {round} step {step} k {k}");
                }
            }
            let probe = rng.next() as u32;
            assert_eq!(rw.translate_sr_ts(probe), probe.wrapping_add(o.to), "S-sym-IMPL-RTCP-TERMINATE-1 SR ts uses the RTP offset · round {round}");
        }
    }
}

#[test]
fn rewriter_reads_time_only_from_argument() {
    let run = |shift: u64| {
        let mut rw = Rewriter::new(48_000);
        let a = rw.map(1, 10, 1_000, 5_000, 960);
        let b = rw.map(2, 70, 99_000, 5_100 + shift, 960);
        (a, b)
    };
    let first = run(0);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let second = run(0);
    assert_eq!(first, second, "S-sym-RULE-CONC-SANS-IO-1 same input → same output regardless of wall clock");
    let later = run(250);
    assert_eq!(later.1.ts.wrapping_sub(first.1.ts), 250 * 48, "S-sym-RULE-CONC-SANS-IO-1 switch tick follows the at_ms argument");
}

#[test]
fn pt_byte1_keeps_marker() {
    for byte1 in 0u8..=255 {
        for pt in [0u8, 35, 63, 96, 102, 111, 127] {
            let out = pt_byte1(byte1, pt);
            assert_eq!(out & 0x7f, pt, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 egress PT is the subscriber PT · byte1 {byte1} pt {pt}");
            assert_eq!(out & 0x80, byte1 & 0x80, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 marker bit untouched · byte1 {byte1} pt {pt}");
        }
    }
}

#[test]
fn serial_after_matches_rfc1982() {
    let mut rng = Rng(0x1982_1982_1982_1982);
    for _ in 0..20_000 {
        let a = rng.next() as u16;
        let b = rng.next() as u16;
        assert_eq!(serial_after(a, b), o_after(a, b), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 serial_after {a} {b}");
    }
    assert!(serial_after(0, 0xffff), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 0 is after 65535");
    assert!(!serial_after(0x8000, 0), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 exact half is not after");
}
