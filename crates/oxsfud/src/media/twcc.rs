// author: kodeholic (powered by Claude)
//! 정§11-2 — Ingress TWCC feedback. 서버가 ★자체 생성한다.
//!
//! 발행자가 `transport-cc` 를 협상했으면 이것이 없을 때 ★송신 추정이 갱신되지 않는다 —
//! 화질이 안 올라가고 원인은 어느 계수에도 안 남는다.
//!
//! draft-holmer-rmcat-transport-wide-cc-extensions-01 §3.1.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use super::rtcp::{FMT_TWCC, PT_RTPFB};

/// 참조 시각의 눈금(64ms) — 초안이 정한 값이다.
pub const REF_TICK_MS: u64 = 64;
/// 도착 간격의 눈금(0.25ms).
pub const DELTA_TICK_US: u64 = 250;
/// 한 번에 보고할 수 있는 패킷 수 상한 — 넘으면 다음 판으로 민다.
pub const MAX_REPORTED: usize = 256;

/// 한 전송로(5-tuple)의 도착 장부. ★SSRC 가 아니라 전송로 단위다 — 그래서 transport-wide 다.
#[derive(Debug, Default)]
pub struct Arrivals {
    seen: Mutex<BTreeMap<u16, u64>>,
    fb_count: AtomicU8,
}

impl Arrivals {
    /// 핫패스 — 판정하지 않고 담기만 한다.
    pub fn observe(&self, seq: u16, at_ms: u64) {
        let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= MAX_REPORTED * 2 {
            return;
        }
        g.insert(seq, at_ms);
    }

    /// 담긴 것을 비우고 피드백 한 장을 짓는다. 담긴 것이 없으면 아무것도 안 짓는다.
    pub fn drain(&self, sender: u32, media: u32) -> Option<Vec<u8>> {
        let taken: Vec<(u16, u64)> = {
            let mut g = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_empty() {
                return None;
            }
            let all: Vec<(u16, u64)> = g.iter().map(|(s, t)| (*s, *t)).collect();
            g.clear();
            all
        };
        let count = self.fb_count.fetch_add(1, Ordering::Relaxed);
        Some(build(sender, media, &taken, count))
    }

    pub fn pending(&self) -> usize {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// 받은 것들로 한 장을 짓는다. ★입력은 seq 오름차순이어야 한다(BTreeMap 이 그것을 준다).
///
/// 상태 묶음은 ★run-length 만 쓴다 — 같은 상태가 이어지는 구간을 한 묶음으로 접는다.
/// vector 묶음을 섞으면 인코더가 두 갈래가 되고, 그 이득은 상태가 잦게 바뀔 때뿐이다.
pub fn build(sender: u32, media: u32, arrivals: &[(u16, u64)], fb_count: u8) -> Vec<u8> {
    let base = arrivals[0].0;
    let last = arrivals[arrivals.len() - 1].0;
    let span = usize::from(last.wrapping_sub(base)) + 1;
    let span = span.min(MAX_REPORTED);
    let known: BTreeMap<u16, u64> = arrivals.iter().copied().collect();

    // 기준 시각은 64ms 눈금으로 내림한다 — 첫 도착이 음수 델타가 되지 않게.
    let ref_ms = (arrivals[0].1 / REF_TICK_MS) * REF_TICK_MS;
    let mut statuses: Vec<u8> = Vec::with_capacity(span);
    let mut deltas: Vec<u8> = Vec::new();
    let mut prev_us = ref_ms * 1_000;

    for step in 0..span {
        let seq = base.wrapping_add(step as u16);
        let Some(&at_ms) = known.get(&seq) else {
            statuses.push(0);
            continue;
        };
        let at_us = at_ms * 1_000;
        let ticks = (at_us as i64 - prev_us as i64) / DELTA_TICK_US as i64;
        prev_us = (prev_us as i64 + ticks * DELTA_TICK_US as i64) as u64;
        if (0..=255).contains(&ticks) {
            statuses.push(1);
            deltas.push(ticks as u8);
        } else {
            statuses.push(2);
            deltas.extend_from_slice(&(ticks.clamp(-32768, 32767) as i16).to_be_bytes());
        }
    }

    let mut out = Vec::with_capacity(20 + span);
    out.push(0x80 | FMT_TWCC);
    out.push(PT_RTPFB);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    out.extend_from_slice(&base.to_be_bytes());
    out.extend_from_slice(&(statuses.len() as u16).to_be_bytes());
    let ref_ticks = ((ref_ms / REF_TICK_MS) & 0x00FF_FFFF) as u32;
    out.extend_from_slice(&ref_ticks.to_be_bytes()[1..]);
    out.push(fb_count);

    for (symbol, run) in runs(&statuses) {
        // run-length 묶음: bit15=0 · 상태 2비트 · 길이 13비트.
        let chunk = (u16::from(symbol) << 13) | (run & 0x1FFF);
        out.extend_from_slice(&chunk.to_be_bytes());
    }
    out.extend_from_slice(&deltas);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    let words = (out.len() / 4 - 1) as u16;
    out[2..4].copy_from_slice(&words.to_be_bytes());
    out
}

/// 같은 상태가 이어지는 구간으로 접는다. 길이는 13비트라 8191 에서 끊는다.
fn runs(statuses: &[u8]) -> Vec<(u8, u16)> {
    let mut out: Vec<(u8, u16)> = Vec::new();
    for &s in statuses {
        match out.last_mut() {
            Some((sym, run)) if *sym == s && *run < 0x1FFF => *run += 1,
            _ => out.push((s, 1)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::rtcp;

    #[test]
    fn a_feedback_names_its_base_and_counts_what_it_covers() {
        let pkt = build(1, 42, &[(100, 0), (101, 5), (102, 10)], 7);
        assert_eq!(rtcp::payload_type(&pkt), Some(PT_RTPFB));
        assert_eq!(rtcp::fmt(&pkt), Some(FMT_TWCC));
        assert_eq!((rtcp::sender_ssrc(&pkt), rtcp::media_ssrc(&pkt)), (Some(1), Some(42)));
        assert_eq!(u16::from_be_bytes([pkt[12], pkt[13]]), 100, "base seq");
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 3, "덮은 개수");
        assert_eq!(pkt[19], 7, "피드백 일련번호");
        assert_eq!(pkt.len() % 4, 0, "RTCP 는 4바이트 정렬이다");
    }

    #[test]
    fn a_hole_is_reported_as_not_received() {
        // 100·102 만 받았다 — 101 은 상태 0 이고 델타가 없다.
        let pkt = build(1, 42, &[(100, 0), (102, 4)], 0);
        assert_eq!(u16::from_be_bytes([pkt[14], pkt[15]]), 3, "빠진 것도 덮는 범위다");
        let chunks = &pkt[20..];
        // 상태 1(받음) 1개 · 0(못 받음) 1개 · 1(받음) 1개 = 묶음 셋.
        assert_eq!(u16::from_be_bytes([chunks[0], chunks[1]]) >> 13, 1);
        assert_eq!(u16::from_be_bytes([chunks[2], chunks[3]]) >> 13, 0);
        assert_eq!(u16::from_be_bytes([chunks[4], chunks[5]]) >> 13, 1);
    }

    #[test]
    fn a_run_of_the_same_status_folds_into_one_chunk() {
        assert_eq!(runs(&[1, 1, 1]), vec![(1, 3)]);
        assert_eq!(runs(&[1, 0, 0, 1]), vec![(1, 1), (0, 2), (1, 1)]);
        assert_eq!(runs(&[]), vec![]);
    }

    #[test]
    fn the_ledger_empties_when_drained() {
        let a = Arrivals::default();
        assert!(a.drain(1, 2).is_none(), "담긴 것이 없으면 안 짓는다");
        a.observe(5, 0);
        a.observe(6, 5);
        assert_eq!(a.pending(), 2);
        assert!(a.drain(1, 2).is_some());
        assert_eq!(a.pending(), 0, "★비우지 않으면 같은 도착을 매 판 다시 보고한다");
    }

    #[test]
    fn the_feedback_counter_moves_each_time() {
        let a = Arrivals::default();
        a.observe(1, 0);
        let first = a.drain(1, 2).unwrap();
        a.observe(2, 1);
        let second = a.drain(1, 2).unwrap();
        assert_eq!((first[19], second[19]), (0, 1), "발행자가 유실을 알아채는 값이다");
    }

    #[test]
    fn the_ledger_does_not_grow_without_bound() {
        let a = Arrivals::default();
        for seq in 0..(MAX_REPORTED as u16 * 3) {
            a.observe(seq, u64::from(seq));
        }
        assert!(a.pending() <= MAX_REPORTED * 2);
    }
}
