// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FWD-MEDIA-WORKERS · S-sym-RULE-FWD-MEDIA-WORKERS-19 · 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! 기형 입력 1층 — 바깥 바이트를 처음 만지는 해석(RTP · 확장 · Opus TOC · STUN · TCP 틀 · 키프레임 · RTCP · NACK · TWCC · DC · 발언권 TLV · WS 틀)이 어떤 바이트에도 panic 하지 않는다. 정상 표본을 길이마다 자르고 · 바이트를 바꾸고 · 길이 칸을 속이고 · 씨앗 고정 무작위와 과대 입력을 넣는다(context 202611/20261113f A2).

use std::panic::{AssertUnwindSafe, catch_unwind};

use oxsfud::fwd::core::{covers, opus_toc_samples};
use oxsfud::fwd::{ext_get, parse_rtp};
use oxsfud::link::stun;
use oxsfud::link::tcp::{Deframer, frame};
use oxsfud::rtcp::{parse_nack, parse_twcc, split};
use oxsfud::trk::is_keyframe;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn rtp_sample() -> Vec<u8> {
    let mut b = vec![0x90 | 0x02, 0x60, 0x12, 0x34, 0, 0, 0x10, 0, 0xA0, 0, 0, 1];
    b.extend_from_slice(&[0, 0, 0, 7, 0, 0, 0, 8]);
    b.extend_from_slice(&[0xBE, 0xDE, 0, 2, 0x10, 0xAA, 0x21, 0xBB, 0xCC, 0, 0, 0]);
    b.extend_from_slice(&[0x10, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01]);
    b
}

fn rtp_two_byte_ext() -> Vec<u8> {
    let mut b = vec![0x90, 0x66, 0, 1, 0, 0, 0, 1, 0xB0, 0, 0, 1];
    b.extend_from_slice(&[0x10, 0x00, 0, 2, 3, 2, 0x11, 0x22, 5, 1, 0x33, 0]);
    b.extend_from_slice(&[0x18, 0x00, 0x05, 0x67, 0x42, 0x00, 0x1f, 0xe9, 0x00, 0x02, 0x68, 0xce, 0x00, 0x03, 0x65, 0x88, 0x84]);
    b
}

fn rtcp_sample() -> Vec<u8> {
    let mut b = vec![0x80, 200, 0, 6, 0xA0, 0, 0, 1];
    b.extend_from_slice(&[0; 20]);
    b.extend_from_slice(&[0x81, 205, 0, 3, 0xA0, 0, 0, 2, 0xB0, 0, 0, 1, 0x00, 0x10, 0x00, 0x05]);
    b.extend_from_slice(&[0x8f, 205, 0, 5, 0xA0, 0, 0, 2, 0xB0, 0, 0, 1, 0x00, 0x01, 0x00, 0x03, 0x00, 0x00, 0x01, 0x00, 0x20, 0x03, 0x04, 0x08]);
    b
}

fn stun_sample() -> Vec<u8> {
    stun::request("abcdefgh", "ijklmnop", "0123456789abcdefghijkl", [7; 12], true)
}

fn floor_sample() -> Vec<u8> {
    oxsig::floor::encode(&oxsig::floor::request("room-1", Some(3))).expect("floor encode")
}

fn dc_sample() -> Vec<u8> {
    oxsig::dc::encode(1, &floor_sample()).expect("dc encode")
}

fn ws_sample() -> Vec<u8> {
    oxsig::frame::encode(oxsig::frame::FrameKind::Request, 513, 7, br#"{"room_id":"r"}"#)
}

type Probe = (&'static str, fn(&[u8]), Vec<u8>);

fn parsers() -> Vec<Probe> {
    vec![
        ("rtp", |b| {
            if let Some(r) = parse_rtp(b) {
                for id in 0..=15 {
                    let _ = ext_get(r.ext, id);
                }
                let p = &r.raw[r.hlen.min(r.raw.len())..];
                let _ = opus_toc_samples(p);
                let _ = covers(true, p);
                let _ = covers(false, p);
                let _ = is_keyframe("VP8", p);
                let _ = is_keyframe("H264", p);
            }
        }, rtp_sample()),
        ("rtp-two-byte-ext", |b| {
            if let Some(r) = parse_rtp(b) {
                for id in 0..=15 {
                    let _ = ext_get(r.ext, id);
                }
                let p = &r.raw[r.hlen.min(r.raw.len())..];
                let _ = is_keyframe("H264", p);
            }
        }, rtp_two_byte_ext()),
        ("payload", |b| {
            let _ = opus_toc_samples(b);
            let _ = covers(true, b);
            let _ = is_keyframe("VP8", b);
            let _ = is_keyframe("H264", b);
        }, rtp_two_byte_ext()[12 + 12..].to_vec()),
        ("rtcp", |b| {
            for p in split(b) {
                let _ = parse_nack(p.body);
                let _ = parse_twcc(p.body);
            }
            let _ = parse_nack(b);
            let _ = parse_twcc(b);
        }, rtcp_sample()),
        ("stun", |b| {
            let _ = stun::is_stun(b);
            let _ = stun::parse(b);
            let _ = stun::check(b, |_| Some("0123456789abcdefghijkl"));
        }, stun_sample()),
        ("tcp", |b| {
            let mut d = Deframer::default();
            d.push(b);
            while d.next_frame().is_some() {}
            d.push(b);
            while d.next_frame().is_some() {}
            let _ = frame(b);
        }, frame(&rtp_sample()).expect("frame")),
        ("dc", |b| {
            if let Some(f) = oxsig::dc::decode(b) {
                let _ = oxsig::floor::decode(f.payload);
            }
        }, dc_sample()),
        ("floor", |b| {
            let _ = oxsig::floor::decode(b);
        }, floor_sample()),
        ("ws", |b| {
            let _ = oxsig::frame::decode(b);
        }, ws_sample()),
    ]
}

fn variants(seed: &[u8], rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for n in 0..=seed.len() {
        out.push(seed[..n].to_vec());
    }
    for i in 0..seed.len() {
        for v in [0x00, 0xFF, 0x7F, 0x80, seed[i] ^ 0x01] {
            let mut s = seed.to_vec();
            s[i] = v;
            out.push(s.clone());
            s.truncate(i + 1);
            out.push(s);
        }
    }
    for i in 0..seed.len().saturating_sub(1) {
        for v in [[0xFF, 0xFF], [0x00, 0x00], [0x7F, 0xFF], [0x80, 0x00]] {
            let mut s = seed.to_vec();
            s[i] = v[0];
            s[i + 1] = v[1];
            out.push(s);
        }
    }
    for _ in 0..2_000 {
        let n = (rng.next() % 1_500) as usize;
        let mut s = seed.to_vec();
        for _ in 0..1 + rng.next() % 4 {
            if s.is_empty() {
                break;
            }
            let i = (rng.next() as usize) % s.len();
            s[i] = rng.next() as u8;
        }
        s.truncate(n.max(1).min(s.len()));
        out.push(s);
        out.push(rng.bytes(n));
    }
    let mut big = seed.to_vec();
    big.resize(65_536, 0xFF);
    out.push(big);
    out.push(rng.bytes(65_536));
    out
}

#[test]
fn packet_path_parsers_never_panic_on_outside_bytes() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut rng = Rng(0x5eed_0b5e_55ed_1113);
    let mut fails = Vec::new();
    let mut tried = 0usize;
    for (name, f, seed) in parsers() {
        for input in variants(&seed, &mut rng) {
            tried += 1;
            if catch_unwind(AssertUnwindSafe(|| f(&input))).is_err() && fails.len() < 40 {
                let head: String = input.iter().take(24).map(|b| format!("{b:02x}")).collect();
                fails.push(format!("{name} len {} — {head}", input.len()));
            }
        }
    }
    std::panic::set_hook(prev);
    assert!(tried > 30_000, "변형이 너무 적다({tried}) — 시험이 공허하다");
    assert!(fails.is_empty(), "바깥 바이트로 panic {}건(앞 40):\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn seeds_are_real_packets() {
    assert!(parse_rtp(&rtp_sample()).is_some_and(|r| r.ext.is_some()), "RTP 표본이 확장을 든 RTP 로 읽혀야 변형이 그 길을 지난다");
    assert!(parse_rtp(&rtp_two_byte_ext()).is_some_and(|r| r.ext.is_some()));
    assert_eq!(split(&rtcp_sample()).len(), 3, "RTCP 표본은 SR · NACK · TWCC 세 덩이");
    assert!(parse_nack(split(&rtcp_sample())[1].body).is_some());
    assert!(stun::parse(&stun_sample()).is_ok());
    assert!(oxsig::dc::decode(&dc_sample()).is_some());
    assert!(oxsig::floor::decode(&floor_sample()).is_some());
    assert!(oxsig::frame::decode(&ws_sample()).is_ok());
}
