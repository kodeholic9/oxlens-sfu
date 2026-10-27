// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 재기록기 · Opus TOC 덮는 길이 · serial · PT byte1 · SR 번역 · RTP 다시 짓기. 규격 문장에서 지은 신탁과 제품을 맞댄다.

use oxsfud::fwd::core::{covers, opus_covers, pt_byte1, serial_after, translate_sr, Rewriter, SrIn, OPUS_CLOCK, VIDEO_CLOCK};
use oxsfud::fwd::rtp;
use serde_json::Value;
use std::path::PathBuf;

fn spec_dir() -> PathBuf {
    match std::env::var("OXLENS_SPEC") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    }
}

fn vectors(name: &str) -> Value {
    let p = spec_dir().join("server/out/vectors").join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&s).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
}

fn o_serial_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

fn o_opus_covers(p: &[u8]) -> u32 {
    if p.is_empty() {
        return 1;
    }
    let config = p[0] >> 3;
    let code = p[0] & 3;
    let frame: u32 = match config {
        0..=11 => [480, 960, 1920, 2880][(config % 4) as usize],
        12..=15 => [480, 960][(config % 2) as usize],
        _ => [120, 240, 480, 960][(config % 4) as usize],
    };
    let frames: u32 = match code {
        0 => 1,
        1 | 2 => 2,
        _ => {
            if p.len() < 2 {
                return 1;
            }
            (p[1] & 0x3f) as u32
        }
    };
    frame * frames
}

#[derive(Default)]
struct Oracle {
    src: Option<u64>,
    seq_off: u16,
    ts_off: u32,
    last: Option<(u16, u32, u64, u32)>,
    boundary: Option<u16>,
}

impl Oracle {
    fn map(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, clock: u32, cov: u32) -> (u16, u32) {
        if self.src != Some(src) {
            if let (Some(_), Some((ls, lt, la, lc))) = (self.src, self.last) {
                let el = at_ms.saturating_sub(la).max(1);
                let tick = (lc as u64).max(el * clock as u64 / 1000);
                self.seq_off = ls.wrapping_add(1).wrapping_sub(seq);
                self.ts_off = (lt as u64).wrapping_add(tick).wrapping_sub(ts as u64) as u32;
                self.boundary = Some(ls.wrapping_add(1));
            }
            self.src = Some(src);
        }
        let os = seq.wrapping_add(self.seq_off);
        let ot = ts.wrapping_add(self.ts_off);
        match self.last {
            Some((ls, ..)) if !o_serial_after(os, ls) => {}
            _ => self.last = Some((os, ot, at_ms, cov)),
        }
        (os, ot)
    }

    fn nack(&self, out_seq: u16) -> Option<u16> {
        if let Some(b) = self.boundary
            && o_serial_after(b, out_seq)
        {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }
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

#[test]
fn serial_rfc1982_exhaustive_against_oracle() {
    for a in (0u32..=65535).step_by(97) {
        for b in (0u32..=65535).step_by(89) {
            let (a, b) = (a as u16, b as u16);
            assert_eq!(serial_after(a, b), o_serial_after(a, b), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 serial a={a} b={b}");
        }
    }
    assert!(serial_after(0, 65535), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 0 은 65535 뒤");
    assert!(!serial_after(65535, 0), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 65535 는 0 뒤가 아니다");
    assert!(!serial_after(32768, 0) && !serial_after(0, 32768), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 차 2^15 는 어느 쪽도 뒤가 아니다");
    assert!(!serial_after(7, 7), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 같으면 뒤가 아니다");
}

#[test]
fn opus_toc_covers_every_config_and_code() {
    for config in 0u8..32 {
        for code in 0u8..4 {
            let toc = (config << 3) | code;
            for second in [1u8, 3, 0x45, 0x3f] {
                let p = [toc, second, 0xaa];
                assert_eq!(opus_covers(&p), o_opus_covers(&p), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 config={config} code={code} 둘째={second:#x}");
            }
        }
        let only = [(config << 3) | 3];
        assert_eq!(opus_covers(&only), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 code 3 둘째 바이트 없음 → 1 (config {config})");
    }
    assert_eq!(opus_covers(&[]), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 빈 페이로드 → 1");
    assert_eq!(opus_covers(&[0xf8, 0xff, 0xfe]), 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 예열 CN f8 ff fe = 960");
    assert_eq!(opus_covers(&[0xf8]), 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 OPUS_SILENCE f8 = 960");
    assert_eq!(covers(OPUS_CLOCK, &[0xf8]), 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 audio = TOC");
    assert_eq!(covers(VIDEO_CLOCK, &[0xf8, 1, 2]), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 video = 1");
    assert_eq!(covers(VIDEO_CLOCK, &[]), 1, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 video 빈 = 1");
}

#[test]
fn oracle_agrees_with_spec_vectors() {
    let v = vectors("fwd.json");
    let cases = v["rewrite"].as_array().expect("rewrite 판 묶음");
    assert!(cases.len() >= 40, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 rewrite 판 수 {}", cases.len());
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let clock = c["clock"].as_u64().unwrap() as u32;
        let mut o = Oracle::default();
        let mut r = Rewriter::new();
        for st in c["steps"].as_array().unwrap() {
            let payload = hex(st["payload"].as_str().unwrap_or(""));
            let cov_v = st["covers"].as_u64().unwrap() as u32;
            let cov = if clock == OPUS_CLOCK { o_opus_covers(&payload) } else { 1 };
            assert_eq!(cov, cov_v, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 신탁 덮는 길이 ≠ 벡터 — {name}");
            assert_eq!(covers(clock, &payload), cov_v, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 제품 덮는 길이 — {name}");
            let (src, seq, ts, at) = (st["src"].as_u64().unwrap(), st["seq"].as_u64().unwrap() as u16, st["ts"].as_u64().unwrap() as u32, st["at_ms"].as_u64().unwrap());
            let (os, ot) = o.map(src, seq, ts, at, clock, cov);
            assert_eq!((os as u64, ot as u64), (st["out_seq"].as_u64().unwrap(), st["out_ts"].as_u64().unwrap()), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 신탁 ≠ 벡터 — {name}");
            let m = r.map(src, seq, ts, at, clock, cov);
            assert_eq!((m.seq, m.ts), (os, ot), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 제품 ≠ 신탁 — {name} seq={seq}");
        }
        for n in c["nack"].as_array().unwrap() {
            let out = n["out_seq"].as_u64().unwrap() as u16;
            let want = n["in_seq"].as_u64().map(|x| x as u16);
            assert_eq!(o.nack(out), want, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 신탁 NACK ≠ 벡터 — {name}");
            assert_eq!(r.nack_to_source(out), want, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 제품 NACK — {name} out={out}");
        }
    }
}

#[test]
fn switch_tick_floor_is_previous_covers_not_one_tick() {
    let mut r = Rewriter::new();
    r.map(1, 100, 5000, 1000, OPUS_CLOCK, 960);
    let m = r.map(2, 7, 123, 1005, OPUS_CLOCK, 960);
    assert!(m.switched, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 출처가 바뀌면 전환");
    assert_eq!(m.seq, 101, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 seq 이어짐");
    assert_eq!(m.ts, 5960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 경과 5ms(240) < 앞 프레임 960 → ts 차 960");
    let mut r = Rewriter::new();
    r.map(1, 100, 5000, 1000, OPUS_CLOCK, 2880);
    let m = r.map(2, 7, 123, 1030, OPUS_CLOCK, 960);
    assert_eq!(m.ts, 5000 + 2880, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 60ms 프레임 뒤 30ms → 2880");
    let mut r = Rewriter::new();
    r.map(1, 100, 5000, 1000, OPUS_CLOCK, 960);
    let m = r.map(2, 7, 123, 1040, OPUS_CLOCK, 960);
    assert_eq!(m.ts, 5000 + 1920, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 경과 40ms > 덮는 길이 → 경과 틱");
    let mut r = Rewriter::new();
    r.map(1, 100, 5000, 1000, VIDEO_CLOCK, 1);
    let m = r.map(2, 7, 123, 1000, VIDEO_CLOCK, 1);
    assert_eq!(m.ts, 5090, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 video 경과 0 → 하한 1ms 90 틱");
}

#[test]
fn late_packet_out_in_order_but_last_unchanged() {
    let mut r = Rewriter::new();
    r.map(1, 10, 1000, 0, OPUS_CLOCK, 960);
    r.map(1, 12, 2920, 40, OPUS_CLOCK, 960);
    let late = r.map(1, 11, 1960, 45, OPUS_CLOCK, 2880);
    assert_eq!((late.seq, late.ts), (11, 1960), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 늦은 패킷은 같은 오프셋으로 나간다");
    assert_eq!(r.last_seq(), Some(12), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 「마지막」은 가장 앞선 seq");
    let m = r.map(2, 500, 0, 46, OPUS_CLOCK, 960);
    assert_eq!(m.seq, 13, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 전환 seq = 가장 앞선 seq + 1");
    assert_eq!(m.ts, 2920 + 960, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 하한 = 「마지막」(seq 12)의 덮는 길이 — 늦은 패킷의 2880 아님");
}

#[test]
fn back_and_forth_switch_has_no_seq_regression() {
    let mut r = Rewriter::new();
    let mut prev: Option<u16> = None;
    let mut at = 0u64;
    let plan: [(u64, u16, u32); 4] = [(1, 100, 0), (2, 40000, 777), (1, 106, 5760), (2, 40006, 777 + 5760)];
    for (src, s0, t0) in plan {
        for k in 0..6u16 {
            at += 20;
            let m = r.map(src, s0.wrapping_add(k), t0.wrapping_add(k as u32 * 960), at, OPUS_CLOCK, 960);
            if let Some(p) = prev {
                assert_eq!(m.seq, p.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 egress seq 역행 · 구멍 0 (src {src})");
            }
            prev = Some(m.seq);
        }
    }
    assert_eq!(r.switches(), 3, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 전환 수 3");
}

#[test]
fn nack_before_boundary_dropped_after_mapped_back() {
    let mut r = Rewriter::new();
    for k in 0..5u16 {
        r.map(1, 1000 + k, k as u32 * 960, k as u64 * 20, OPUS_CLOCK, 960);
    }
    for k in 0..5u16 {
        r.map(2, 9 + k, 77 + k as u32 * 960, 100 + k as u64 * 20, OPUS_CLOCK, 960);
    }
    assert_eq!(r.nack_to_source(1004), None, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 경계 앞 NACK 은 버린다");
    assert_eq!(r.nack_to_source(1000), None, "S-sym-IMPL-FWD-SOURCE-SWITCH-1 경계 앞 NACK 은 버린다(옛 범위 기억 0)");
    assert_eq!(r.nack_to_source(1005), Some(9), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 경계 이후 NACK 은 지금 출처 seq");
    assert_eq!(r.nack_to_source(1009), Some(13), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 NACK − seq_off");
}

#[test]
fn long_idle_has_no_cap() {
    let mut r = Rewriter::new();
    r.map(1, 1, 100, 0, OPUS_CLOCK, 960);
    let idle_ms: u64 = 13 * 3600 * 1000;
    let m = r.map(2, 50, 9, idle_ms, OPUS_CLOCK, 960);
    let want = (100u64 + idle_ms * 48) as u32;
    assert_eq!(m.ts, want, "S-sym-IMPL-FWD-SOURCE-SWITCH-3 상한 없음 — 경과 × 48 mod 2^32");
}

#[test]
fn prop_random_source_sequences_match_oracle() {
    let payloads: [&[u8]; 8] = [&[0xf8, 0xff, 0xfe], &[0x10], &[0x0b, 1], &[0xe1, 1], &[0xe3, 0x03, 0], &[0xf0], &[0x70], &[0x03]];
    let mut rng = Rng(0x2026_1027_beef);
    for round in 0..300 {
        let clock = if round % 7 == 0 { VIDEO_CLOCK } else { OPUS_CLOCK };
        let mut r = Rewriter::new();
        let mut o = Oracle::default();
        let mut cur: [(u16, u32); 5] = std::array::from_fn(|_| (rng.next() as u16, rng.next() as u32));
        let mut src = 1 + rng.below(4);
        let mut at = rng.below(1 << 40);
        let mut last_out: Option<(u16, u32, u32)> = None;
        for _ in 0..80 {
            if rng.below(6) == 0 {
                src = 1 + rng.below(4);
            }
            at += rng.below(45);
            let p = payloads[rng.below(payloads.len() as u64) as usize];
            let cov = covers(clock, p);
            let late = rng.below(10) == 0;
            let (s, t) = cur[src as usize];
            let (seq, ts) = if late { (s.wrapping_sub(2), t.wrapping_sub(1920)) } else { (s, t) };
            if !late {
                cur[src as usize] = (s.wrapping_add(1), t.wrapping_add(cov.max(960)));
            }
            let before_src = r.source();
            let m = r.map(src, seq, ts, at, clock, cov);
            let (os, ot) = o.map(src, seq, ts, at, clock, cov);
            assert_eq!((m.seq, m.ts), (os, ot), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 PROP 제품 ≠ 신탁 (round {round})");
            if before_src.is_some() && before_src != Some(src) {
                let (ls, lt, lc) = last_out.unwrap();
                assert_eq!(m.seq, ls.wrapping_add(1), "S-sym-IMPL-FWD-SOURCE-SWITCH-1 PROP-FWD-SWITCH-SEQ-CONTIGUOUS (round {round})");
                let d = m.ts.wrapping_sub(lt);
                assert!(d >= lc && d < (1u32 << 31), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 PROP-FWD-SWITCH-TS-FORWARD 차 {d} ≥ 덮는 길이 {lc} (round {round})");
            }
            match last_out {
                Some((ls, ..)) if !o_serial_after(m.seq, ls) => {}
                _ => last_out = Some((m.seq, m.ts, cov)),
            }
        }
    }
}

#[test]
fn sans_io_same_inputs_same_outputs() {
    let run = || {
        let mut r = Rewriter::new();
        let mut out = vec![];
        for k in 0..30u16 {
            let src = 1 + (k / 7) as u64;
            out.push(r.map(src, k * 3, k as u32 * 960, k as u64 * 13, OPUS_CLOCK, 960));
        }
        out.iter().map(|m| (m.seq, m.ts, m.switched)).collect::<Vec<_>>()
    };
    let a = run();
    std::thread::sleep(std::time::Duration::from_millis(30));
    let b = run();
    assert_eq!(a, b, "S-sym-RULE-CONC-SANS-IO-1 재기록기는 시계를 안에서 읽지 않는다 — 같은 입력 같은 출력");
}

#[test]
fn pt_byte1_keeps_marker_replaces_low_seven() {
    for b in 0u16..=255 {
        for pt in 0u8..=127 {
            let b = b as u8;
            assert_eq!(pt_byte1(b, pt), (b & 0x80) | pt, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 byte1 하위 7비트만 교체 b={b} pt={pt}");
        }
    }
}

#[test]
fn sr_translate_ssrc_rtp_offset_ntp_untouched() {
    let out = translate_sr(SrIn { ssrc: 0x1111, ntp: 0xdead_beef_0102_0304, rtp_ts: 4_294_967_000 }, 77, 500);
    assert_eq!(out.ssrc, 77, "S-sym-IMPL-RTCP-TERMINATE-1 SR SSRC = 다운링크 SSRC");
    assert_eq!(out.ntp, 0xdead_beef_0102_0304, "S-sym-IMPL-RTCP-TERMINATE-1 NTP 원본 그대로");
    assert_eq!(out.rtp_ts, 4_294_967_000u32.wrapping_add(500), "S-sym-IMPL-RTCP-TERMINATE-1 RTP ts 는 같은 오프셋(감김)");
    let mut r = Rewriter::new();
    r.map(1, 1, 1000, 0, OPUS_CLOCK, 960);
    r.map(2, 900, 50, 100, OPUS_CLOCK, 960);
    let s = translate_sr(SrIn { ssrc: 2, ntp: 9, rtp_ts: 50 }, 42, r.ts_off());
    let m = r.map(2, 901, 50, 120, OPUS_CLOCK, 960);
    assert_eq!(s.rtp_ts, m.ts, "S-sym-IMPL-RTCP-TERMINATE-1 SR 의 ts 오프셋 = RTP 전달 오프셋");
}

#[allow(clippy::too_many_arguments)]
fn rtp_packet(marker: bool, pt: u8, seq: u16, ts: u32, ssrc: u32, ext: Option<&[u8]>, payload: &[u8], pad: u8) -> Vec<u8> {
    let mut b = vec![0x80u8, (if marker { 0x80 } else { 0 }) | pt];
    if pad > 0 {
        b[0] |= 0x20;
    }
    if ext.is_some() {
        b[0] |= 0x10;
    }
    b.extend_from_slice(&seq.to_be_bytes());
    b.extend_from_slice(&ts.to_be_bytes());
    b.extend_from_slice(&ssrc.to_be_bytes());
    if let Some(e) = ext {
        b.extend_from_slice(e);
    }
    b.extend_from_slice(payload);
    for i in 0..pad {
        b.push(if i + 1 == pad { pad } else { 0 });
    }
    b
}

#[test]
fn rebuild_rewrites_header_keeps_payload_padding_marker() {
    let ext_in = [0xbe, 0xde, 0x00, 0x01, 0x10, 0x55, 0x00, 0x00];
    let payload = [0xf8, 0x01, 0x02, 0x03, 0x04];
    let raw = rtp_packet(true, 111, 9, 1234, 0xabcdef, Some(&ext_in), &payload, 4);
    let p = rtp::parse(&raw).expect("parse");
    assert!(p.marker && p.pt == 111 && p.seq == 9 && p.ts == 1234 && p.ssrc == 0xabcdef, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 parse 머리");
    let out = rtp::rebuild(&p, 0x42, 77, 999, 96, &[]);
    let q = rtp::parse(&out).expect("다시 짓기 결과 parse");
    assert!(q.marker, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 marker 원본 그대로");
    assert_eq!((q.pt, q.seq, q.ts, q.ssrc), (96, 77, 999, 0x42), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 머리 교체");
    let tail_in = &raw[raw.len() - payload.len() - 4..];
    let tail_out = &out[out.len() - payload.len() - 4..];
    assert_eq!(tail_in, tail_out, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 페이로드 · 패딩 원본 그대로");
    assert_eq!(out[0] & 0x20, 0x20, "S-sym-IMPL-FWD-SOURCE-SWITCH-2 패딩 비트 그대로");
    assert_eq!(out.len(), raw.len() - ext_in.len(), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 길이는 확장 영역만큼만 달라진다");
}
