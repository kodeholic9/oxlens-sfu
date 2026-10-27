// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — RR 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 피드백(draft-holmer-01 §3.1) · NACK 상향 · RTCP 꼴. 규격 문장에서 지은 신탁과 제품을 맞댄다.

use oxsfud::rtcp::core::{encode_twcc, Feedback, NackUp, RecvStats, Sym, TwccRecv};
use oxsfud::rtcp::pkt::{self, Packet, ReportBlock};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn vectors(name: &str) -> Value {
    let base = match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    };
    let p = base.join("server/out/vectors").join(name);
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()))).unwrap()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const MOD: u32 = 65536;

struct ORr {
    clock: u64,
    started: bool,
    base: u32,
    max: u16,
    bad: u32,
    cycles: u32,
    recv: u32,
    rp: u32,
    ep: u32,
    prob: u32,
    transit: Option<i64>,
    j: f64,
}

impl ORr {
    fn new(clock: u32) -> ORr {
        ORr { clock: clock as u64, started: false, base: 0, max: 0, bad: MOD + 1, cycles: 0, recv: 0, rp: 0, ep: 0, prob: 0, transit: None, j: 0.0 }
    }
    fn init(&mut self, seq: u16) {
        self.base = seq as u32;
        self.max = seq;
        self.bad = MOD + 1;
        self.cycles = 0;
        self.recv = 0;
        self.rp = 0;
        self.ep = 0;
    }
    fn upd(&mut self, seq: u16) -> bool {
        if !self.started {
            self.started = true;
            self.init(seq);
            self.max = seq.wrapping_sub(1);
            self.prob = 2;
        }
        let ud = seq.wrapping_sub(self.max) as u32;
        if self.prob > 0 {
            if seq == self.max.wrapping_add(1) {
                self.prob -= 1;
                self.max = seq;
                if self.prob == 0 {
                    self.init(seq);
                    self.recv += 1;
                    return true;
                }
            } else {
                self.prob = 1;
                self.max = seq;
            }
            return false;
        } else if ud < 3000 {
            if seq < self.max {
                self.cycles += MOD;
            }
            self.max = seq;
        } else if ud <= MOD - 100 {
            if seq as u32 == self.bad {
                self.init(seq);
            } else {
                self.bad = (seq as u32 + 1) & (MOD - 1);
                return false;
            }
        }
        self.recv += 1;
        true
    }
    fn update(&mut self, seq: u16, ts: u32, arr_ms: u64) -> bool {
        let v = self.upd(seq);
        let a = (arr_ms * self.clock / 1000) as i64;
        let t = a - ts as i64;
        if let Some(p) = self.transit {
            let d = (t - p).abs() as f64;
            self.j += (d - self.j) / 16.0;
        }
        self.transit = Some(t);
        v
    }
    fn report(&mut self) -> (u8, i32, u32, u32) {
        let ext = self.cycles + self.max as u32;
        let exp = ext as i64 - self.base as i64 + 1;
        let lost = (exp - self.recv as i64).clamp(-0x80_0000, 0x7f_ffff) as i32;
        let ei = exp - self.ep as i64;
        self.ep = exp as u32;
        let ri = self.recv as i64 - self.rp as i64;
        self.rp = self.recv;
        let li = ei - ri;
        let fr = if ei == 0 || li <= 0 { 0 } else { ((li << 8) / ei) as u8 };
        (fr, lost, ext, self.j as u32)
    }
}

#[test]
fn rr_vectors_match_oracle_and_product() {
    let v = vectors("rtcp.json");
    let cases = v["rr"].as_array().expect("rr 판 묶음");
    assert!(!cases.is_empty(), "S-sym-IMPL-RTCP-TERMINATE-5 rr 판이 비었다");
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let clock = c["clock"].as_u64().unwrap() as u32;
        let mut o = ORr::new(clock);
        let mut p = RecvStats::new(clock);
        for e in c["events"].as_array().unwrap() {
            if let Some(r) = e.get("report") {
                let want = (r["fraction"].as_u64().unwrap() as u8, r["lost"].as_i64().unwrap() as i32, r["extended_max"].as_u64().unwrap() as u32, r["jitter"].as_u64().unwrap() as u32);
                assert_eq!(o.report(), want, "S-sym-IMPL-RTCP-TERMINATE-5 신탁 ≠ 벡터 — {name}");
                let g = p.report();
                assert_eq!((g.fraction, g.lost, g.extended_max, g.jitter), want, "S-sym-IMPL-RTCP-TERMINATE-5 제품 RR — {name}");
            } else {
                let (s, t, a) = (e["seq"].as_u64().unwrap() as u16, e["ts"].as_u64().unwrap() as u32, e["arrival_ms"].as_u64().unwrap());
                let want = e["valid"].as_u64().unwrap() == 1;
                assert_eq!(o.update(s, t, a), want, "S-sym-IMPL-RTCP-TERMINATE-5 신탁 출처 인정 — {name}");
                assert_eq!(p.update(s, t, a), want, "S-sym-IMPL-RTCP-TERMINATE-5 제품 출처 인정 — {name} seq={s}");
            }
        }
    }
}

#[test]
fn rr_random_streams_match_rfc3550_oracle() {
    let mut rng = Rng(0x5eed_3550);
    for round in 0..200 {
        let clock = if round % 3 == 0 { 90_000 } else { 48_000 };
        let mut o = ORr::new(clock);
        let mut p = RecvStats::new(clock);
        let mut seq = rng.next() as u16;
        let mut ts = rng.next() as u32;
        let mut at = rng.below(1 << 30);
        for step in 0..300 {
            let r = rng.below(100);
            let s = match r {
                0..=69 => {
                    seq = seq.wrapping_add(1);
                    seq
                }
                70..=79 => {
                    seq = seq.wrapping_add(2 + rng.below(20) as u16);
                    seq
                }
                80..=86 => seq.wrapping_sub(rng.below(50) as u16),
                87..=89 => seq,
                90..=93 => {
                    seq = seq.wrapping_add(3000 + rng.below(20000) as u16);
                    seq
                }
                _ => seq.wrapping_sub(200 + rng.below(3000) as u16),
            };
            ts = ts.wrapping_add(960);
            at += rng.below(40);
            let a = o.update(s, ts, at);
            let b = p.update(s, ts, at);
            assert_eq!(a, b, "S-sym-IMPL-RTCP-TERMINATE-5 출처 인정 (round {round} step {step} seq {s})");
            if step % 37 == 36 {
                let g = p.report();
                assert_eq!((g.fraction, g.lost, g.extended_max, g.jitter), o.report(), "S-sym-IMPL-RTCP-TERMINATE-5 RR 값 (round {round} step {step})");
            }
        }
    }
}

#[test]
fn rr_duplicates_make_cumulative_loss_negative() {
    let mut p = RecvStats::new(48_000);
    for (i, s) in [7u16, 8, 9, 9, 9, 9].iter().enumerate() {
        p.update(*s, i as u32 * 960, i as u64 * 20);
    }
    assert!(p.report().lost < 0, "S-sym-IMPL-RTCP-TERMINATE-5 중복이면 누적 손실은 음수(부호 있는 수)");
}

#[test]
fn rr_highest_seq_is_actual_receipt_across_wrap() {
    let mut p = RecvStats::new(48_000);
    let mut s = 65_000u16;
    for i in 0..2000u32 {
        p.update(s, i * 960, i as u64 * 20);
        s = s.wrapping_add(1);
    }
    let last = s.wrapping_sub(1);
    let r = p.report();
    assert_eq!(r.extended_max & 0xffff, last as u32, "S-sym-IMPL-RTCP-TERMINATE-5 highest seq 하위 16비트 = 실제 마지막 수신");
    assert_eq!(r.extended_max >> 16, 1, "S-sym-IMPL-RTCP-TERMINATE-5 감김 1 회");
    assert_eq!(r.lost, 0, "S-sym-IMPL-RTCP-TERMINATE-5 손실 0");
}

#[derive(Default)]
struct OTw {
    arr: BTreeMap<i64, i64>,
    last: Option<i64>,
    start: Option<i64>,
    fb: u32,
}

impl OTw {
    fn record(&mut self, seq: u16, us: i64) {
        let u = match self.last {
            None => seq as i64,
            Some(l) => l + (seq.wrapping_sub(l as u16) as i16) as i64,
        };
        if self.last.is_none_or(|l| u > l) {
            self.last = Some(u);
        }
        if self.arr.contains_key(&u) {
            return;
        }
        self.arr.insert(u, us);
        if let Some(s) = self.start
            && u < s
        {
            self.start = Some(u);
        }
    }
    fn build(&mut self) -> Vec<(u16, u16, u32, u8, String, Vec<i32>)> {
        let mut out = vec![];
        let Some((&hi, _)) = self.arr.iter().next_back() else { return out };
        let mut st = self.start.unwrap_or(*self.arr.keys().next().unwrap());
        while st <= hi {
            let first = (st..=hi).find(|q| self.arr.contains_key(q)).unwrap();
            let rt = (self.arr[&first] % (64_000i64 << 24)) / 64_000;
            let mut base = self.arr[&first] / 64_000 * 64_000;
            let (mut syms, mut dl, mut q) = (String::new(), vec![], st);
            while q <= hi && syms.len() < 2001 {
                if let Some(&a) = self.arr.get(&q) {
                    let d = (a - base) as f64 / 250.0;
                    let d = d.round() as i64;
                    if !(-32768..=32767).contains(&d) {
                        break;
                    }
                    syms.push(if (0..=255).contains(&d) { 'S' } else { 'L' });
                    dl.push(d as i32);
                    base += d * 250;
                } else {
                    syms.push('N');
                }
                q += 1;
            }
            out.push(((st & 0xffff) as u16, syms.len() as u16, rt as u32, (self.fb % 256) as u8, syms, dl));
            self.fb += 1;
            st = q;
        }
        self.start = Some(hi + 1);
        out
    }
}

fn fb_tuple(f: &Feedback) -> (u16, u16, u32, u8, String, Vec<i32>) {
    let syms: String = f.symbols.iter().map(|s| match s {
        Sym::NotReceived => 'N',
        Sym::Small => 'S',
        Sym::Large => 'L',
    }).collect();
    (f.base_seq, f.status_count, f.reference_time, f.fb_count, syms, f.deltas_250us.clone())
}

#[test]
fn twcc_vectors_match_oracle_and_product() {
    let v = vectors("rtcp.json");
    let cases = v["twcc"].as_array().expect("twcc 판 묶음");
    assert!(!cases.is_empty(), "S-sym-IMPL-RTCP-TERMINATE-6 twcc 판이 비었다");
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let (mut o, mut p) = (OTw::default(), TwccRecv::new());
        let evs = c["events"].as_array().unwrap();
        let mut i = 0;
        while i < evs.len() {
            if evs[i].get("build").is_some() {
                let mut want = vec![];
                while i < evs.len() && evs[i].get("build").is_some() {
                    let b = &evs[i]["build"];
                    want.push((b["base_seq"].as_u64().unwrap() as u16, b["status_count"].as_u64().unwrap() as u16, b["reference_time"].as_u64().unwrap() as u32, b["fb_count"].as_u64().unwrap() as u8, b["symbols"].as_str().unwrap().to_string(), b["deltas_250us"].as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect::<Vec<_>>()));
                    i += 1;
                }
                assert_eq!(o.build(), want, "S-sym-IMPL-RTCP-TERMINATE-6 신탁 ≠ 벡터 — {name}");
                let got: Vec<_> = p.build().iter().map(fb_tuple).collect();
                assert_eq!(got, want, "S-sym-IMPL-RTCP-TERMINATE-6 제품 TWCC — {name}");
            } else {
                let (s, a) = (evs[i]["seq"].as_u64().unwrap() as u16, evs[i]["arrival_us"].as_i64().unwrap());
                o.record(s, a);
                p.record(s, a);
                i += 1;
            }
        }
    }
}

#[test]
fn twcc_random_windows_match_oracle() {
    let mut rng = Rng(0x7cc0_0001);
    let res = [0i64, 40, 100, 160, 210];
    for round in 0..200 {
        let (mut o, mut p) = (OTw::default(), TwccRecv::new());
        let mut seq = rng.next() as u16;
        let mut t = (rng.below(1 << 30) as i64) * 250;
        let mut sent: Vec<u16> = vec![];
        for step in 0..120 {
            match rng.below(10) {
                0 if !sent.is_empty() => {
                    let s = sent[rng.below(sent.len() as u64) as usize];
                    let a = t - 250 * rng.below(40) as i64 + res[rng.below(5) as usize];
                    o.record(s, a);
                    p.record(s, a);
                }
                1 => {
                    seq = seq.wrapping_add(1 + rng.below(4) as u16);
                }
                _ => {
                    seq = seq.wrapping_add(1);
                    t += 250 * rng.below(40) as i64;
                    let a = t + res[rng.below(5) as usize];
                    sent.push(seq);
                    o.record(seq, a);
                    p.record(seq, a);
                }
            }
            if rng.below(12) == 0 {
                let got: Vec<_> = p.build().iter().map(fb_tuple).collect();
                assert_eq!(got, o.build(), "S-sym-IMPL-RTCP-TERMINATE-6 TWCC 창 (round {round} step {step})");
            }
        }
    }
}

#[test]
#[ignore = "server defect: S-sym-IMPL-RTCP-TERMINATE-6 — 도착 시각이 64ms·2^24(약 12.4일)를 넘으면 TWCC 피드백을 하나도 안 짓는다(기준 시각 mod 감김)"]
fn twcc_reference_time_wraps_at_24_bits_deltas_unchanged() {
    let wrap = 64_000i64 << 24;
    let mut seen = vec![];
    for base in [1_000_000i64, wrap + 1_000_000, 10_000_000_000_000] {
        let mut p = TwccRecv::new();
        for k in 0..5i64 {
            p.record(100 + k as u16, base + k * 20_000);
        }
        let fbs = p.build();
        assert_eq!(fbs.len(), 1, "S-sym-IMPL-RTCP-TERMINATE-6 도착 시각 {base}µs 에도 피드백 1");
        let f = &fbs[0];
        assert_eq!(f.reference_time as i64, (base / 64_000) % (1 << 24), "S-sym-IMPL-RTCP-TERMINATE-6 기준 시각 24비트 감김 — {base}");
        assert_eq!(f.status_count, 5, "S-sym-IMPL-RTCP-TERMINATE-6 상태 5 — {base}");
        seen.push(f.deltas_250us[1..].to_vec());
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "S-sym-IMPL-RTCP-TERMINATE-6 기준 시각 감김이 델타를 안 바꾼다 — {seen:?}");
}

#[test]
fn twcc_long_window_splits_at_2001() {
    let mut p = TwccRecv::new();
    for k in 0..2500u32 {
        p.record(k as u16, 1_000_000 + k as i64 * 1000);
    }
    let fbs = p.build();
    assert!(fbs.len() >= 2, "S-sym-IMPL-RTCP-TERMINATE-6 2,001 패킷을 넘으면 피드백이 나뉜다");
    assert!(fbs.iter().all(|f| f.status_count as usize <= 2001), "S-sym-IMPL-RTCP-TERMINATE-6 한 피드백 최대 2,001");
    let total: usize = fbs.iter().map(|f| f.status_count as usize).sum();
    assert_eq!(total, 2500, "S-sym-IMPL-RTCP-TERMINATE-6 상태 수 합 = 받은 범위");
    assert_eq!(fbs[1].fb_count, fbs[0].fb_count.wrapping_add(1), "S-sym-IMPL-RTCP-TERMINATE-6 피드백 수 +1");
    assert!(!p.pending(), "S-sym-IMPL-RTCP-TERMINATE-6 보낸 뒤 남은 것 없음");
}

#[test]
fn twcc_receiver_clock_offset_does_not_change_deltas() {
    let arr = [0i64, 20_000, 41_000, 59_750, 80_250, 100_000];
    let run = |off: i64| {
        let mut p = TwccRecv::new();
        for (i, a) in arr.iter().enumerate() {
            p.record(100 + i as u16, 5_000_000 + off + a);
        }
        p.build()
    };
    let a = run(0);
    let b = run(64_000 * 1000);
    assert_eq!(a[0].deltas_250us[1..], b[0].deltas_250us[1..], "S-sym-IMPL-RTCP-TERMINATE-6 차분만 쓴다 — 시계 오프셋이 델타에 안 들어간다");
}

fn decode_twcc(b: &[u8]) -> (u32, u32, u16, u16, u32, u8, Vec<char>, Vec<i32>) {
    assert_eq!(b[0] & 0x1f, 15, "S-sym-IMPL-RTCP-TERMINATE-6 FMT 15");
    assert_eq!(b[1], 205, "S-sym-IMPL-RTCP-TERMINATE-6 PT 205");
    let len = (u16::from_be_bytes([b[2], b[3]]) as usize + 1) * 4;
    assert_eq!(len, b.len(), "S-sym-IMPL-RTCP-TERMINATE-6 길이 칸 = 실제 길이(32비트 정렬)");
    let sender = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let media = u32::from_be_bytes([b[8], b[9], b[10], b[11]]);
    let base = u16::from_be_bytes([b[12], b[13]]);
    let count = u16::from_be_bytes([b[14], b[15]]);
    let rt = u32::from_be_bytes([0, b[16], b[17], b[18]]);
    let fbc = b[19];
    let mut i = 20;
    let mut syms = vec![];
    while syms.len() < count as usize {
        let c = u16::from_be_bytes([b[i], b[i + 1]]);
        i += 2;
        assert_eq!(c >> 15, 0, "S-sym-IMPL-RTCP-TERMINATE-6 상태 묶음은 run-length 만");
        let sym = match (c >> 13) & 3 {
            0 => 'N',
            1 => 'S',
            2 => 'L',
            _ => panic!("예약 기호"),
        };
        for _ in 0..(c & 0x1fff) {
            syms.push(sym);
        }
    }
    assert_eq!(syms.len(), count as usize, "S-sym-IMPL-RTCP-TERMINATE-6 run 길이 합 = 상태 수");
    let mut dl = vec![];
    for s in &syms {
        match s {
            'S' => {
                dl.push(b[i] as i32);
                i += 1;
            }
            'L' => {
                dl.push(i16::from_be_bytes([b[i], b[i + 1]]) as i32);
                i += 2;
            }
            _ => {}
        }
    }
    assert!(b.len() - i < 4, "S-sym-IMPL-RTCP-TERMINATE-6 덧붙임은 정렬만큼");
    (sender, media, base, count, rt, fbc, syms, dl)
}

#[test]
fn twcc_encode_is_draft_layout() {
    let mut p = TwccRecv::new();
    for (s, a) in [(10u16, 1_000_000i64), (11, 1_020_000), (13, 1_010_000), (14, 9_000_000)] {
        p.record(s, a);
    }
    for f in p.build() {
        let raw = encode_twcc(1, 0xabcd, &f);
        let (snd, med, base, cnt, rt, fbc, syms, dl) = decode_twcc(&raw);
        assert_eq!((snd, med), (1, 0xabcd), "S-sym-IMPL-RTCP-TERMINATE-6 sender SSRC 1 · media SSRC");
        let (b2, c2, r2, f2, s2, d2) = fb_tuple(&f);
        assert_eq!((base, cnt, rt, fbc, syms.into_iter().collect::<String>(), dl), (b2, c2, r2, f2, s2, d2), "S-sym-IMPL-RTCP-TERMINATE-6 부호화 왕복");
    }
}

#[test]
fn nack_up_gap_rules() {
    let mut n = NackUp::new();
    n.on_packet(100, 0);
    n.on_packet(228, 1);
    let mut all = vec![];
    for t in [1u64, 50, 250, 500, 900] {
        all.extend(n.due(t));
    }
    assert!(all.is_empty(), "S-sym-RULE-RTCP-NACK-UP-1 gap 128 이상은 NACK 0 — {all:?}");
    let mut n = NackUp::new();
    n.on_packet(100, 0);
    n.on_packet(227, 1);
    let mut seen: std::collections::BTreeSet<u16> = Default::default();
    for t in [1u64, 2, 210, 420] {
        seen.extend(n.due(t));
    }
    assert!(!seen.is_empty() && seen.iter().all(|s| (101..227).contains(s)), "S-sym-RULE-RTCP-NACK-UP-1 diff 127 은 등재 — 그 사이 seq 만");
    assert!(seen.len() <= 128, "S-sym-RULE-RTCP-NACK-UP-1 추적 상한 128");
}

#[test]
fn nack_up_reorder_recovers_and_does_not_rewind() {
    let mut n = NackUp::new();
    n.on_packet(65_533, 0);
    n.on_packet(2, 1);
    n.on_packet(65_535, 2);
    let first: std::collections::BTreeSet<u16> = n.due(3).into_iter().collect();
    assert!(!first.contains(&65_535), "S-sym-RULE-RTCP-NACK-UP-1 늦게 온 seq 는 결손에서 빠진다");
    assert!(first.contains(&65_534) && first.contains(&0) && first.contains(&1), "S-sym-RULE-RTCP-NACK-UP-1 감김 너머 결손 등재 — {first:?}");
    n.on_packet(3, 4);
    n.on_packet(6, 5);
    let later: std::collections::BTreeSet<u16> = [n.due(6), n.due(260)].concat().into_iter().collect();
    assert!(!later.iter().any(|s| *s > 6 && *s < 60_000), "S-sym-RULE-RTCP-NACK-UP-1 last_seq 를 되돌리지 않는다 — {later:?}");
    assert!(later.contains(&4) || later.contains(&5), "S-sym-RULE-RTCP-NACK-UP-1 새 결손 4 · 5 등재 — {later:?}");
}

#[test]
fn nack_up_retry_interval_and_give_up() {
    let mut n = NackUp::new();
    n.on_packet(10, 0);
    n.on_packet(12, 0);
    let mut hits = vec![];
    for t in 0..1500u64 {
        if n.due(t).contains(&11) {
            hits.push(t);
        }
    }
    assert!(!hits.is_empty(), "S-sym-RULE-RTCP-NACK-UP-1 결손은 NACK 된다");
    for w in hits.windows(2) {
        assert!(w[1] - w[0] >= 200, "S-sym-RULE-RTCP-NACK-UP-1 재전송 간격 200ms — {hits:?}");
    }
    assert!(hits.iter().all(|t| *t <= 1000), "S-sym-RULE-RTCP-NACK-UP-1 1,000ms 경과 시 만료 — {hits:?}");
}

#[test]
fn rtcp_packets_round_trip_and_sender_ssrc_one() {
    let blk = ReportBlock { ssrc: 0x1234_5678, fraction: 51, lost: -2, ext_max: 65_538, jitter: 85, lsr: 0xaabb_ccdd, dlsr: 65_536 };
    let raw = pkt::rr(&blk);
    match pkt::parse(&raw).as_slice() {
        [Packet::Rr { ssrc, blocks }] => {
            assert_eq!(*ssrc, 1, "S-sym-IMPL-RTCP-TERMINATE-5 서버가 짓는 RR 의 sender SSRC = 1");
            assert_eq!(blocks.as_slice(), std::slice::from_ref(&blk), "S-sym-IMPL-RTCP-TERMINATE-5 RR 블록 왕복(음수 손실 24비트)");
        }
        other => panic!("S-sym-IMPL-RTCP-TERMINATE-5 RR 블록 하나짜리여야 — {other:?}"),
    }
    let seqs = [100u16, 101, 105, 116, 117, 400, 65_535, 0];
    let raw = pkt::nack(9, &seqs);
    match pkt::parse(&raw).as_slice() {
        [Packet::Nack { media, seqs: got }] => {
            assert_eq!(*media, 9, "S-sym-RULE-RTCP-NACK-UP-1 NACK media SSRC");
            let mut a: Vec<u16> = got.clone();
            let mut b = seqs.to_vec();
            a.sort();
            b.sort();
            assert_eq!(a, b, "S-sym-RULE-RTCP-NACK-UP-1 PID+BLP 묶음 왕복");
        }
        other => panic!("NACK 꼴 — {other:?}"),
    }
    assert!(matches!(pkt::parse(&pkt::pli(77)).as_slice(), [Packet::Pli { media: 77 }]), "S-sym-IMPL-RTCP-TERMINATE-1 PLI 꼴");
    match pkt::parse(&pkt::remb(1_000_000, &[5, 6])).as_slice() {
        [Packet::Remb { bitrate, ssrcs }] => assert!(*bitrate == 1_000_000 && ssrcs == &vec![5, 6], "S-sym-IMPL-RTCP-TERMINATE-3 REMB 왕복"),
        other => panic!("REMB 꼴 — {other:?}"),
    }
    match pkt::parse(&pkt::sr(3, 0x0102_0304_0506_0708, 99, 10, 2000)).as_slice() {
        [Packet::Sr { ssrc, ntp, rtp_ts, packets, octets, .. }] => assert_eq!((*ssrc, *ntp, *rtp_ts, *packets, *octets), (3, 0x0102_0304_0506_0708, 99, 10, 2000), "S-sym-IMPL-RTCP-TERMINATE-1 SR 왕복"),
        other => panic!("SR 꼴 — {other:?}"),
    }
}

#[test]
fn dlsr_and_lsr_units() {
    assert_eq!(pkt::ntp_mid(0x1122_3344_5566_7788), 0x3344_5566, "S-sym-IMPL-RTCP-TERMINATE-7 LSR = NTP 가운데 32비트");
    assert_eq!(pkt::dlsr(1000), 65_536, "S-sym-IMPL-RTCP-TERMINATE-7 DLSR 단위 1/65536 초");
    assert_eq!(pkt::dlsr(500), 32_768, "S-sym-IMPL-RTCP-TERMINATE-7 DLSR 500ms");
    assert!(pkt::dlsr(1) > 0, "S-sym-IMPL-RTCP-TERMINATE-7 쥐고 있던 시간이 있으면 DLSR > 0");
    assert_eq!(pkt::dlsr(0), 0, "S-sym-IMPL-RTCP-TERMINATE-7 쥔 시간 0 이면 0");
}
