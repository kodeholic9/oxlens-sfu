// author: kodeholic (powered by Claude)
//! 수신 통계 — 정§11-2 Ingress RR 의 재료. 발행자가 보는 "우리 수신 품질"이다.
//! 셈은 RFC 3550 부록 A.1(seq 감김·손실)과 A.8(지터)을 그대로 따른다.
//!
//! ★매 패킷 도는 자리라 자료가 전부 원자값이다(§2-3 계약 1) — 락도 할당도 없다.
//! ★RTX 는 여기 넣지 않는다(정§11-2) — 넣으면 손실률이 0 근처로 오염돼 자동 레이어가 강등을 못 한다.

use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};

use super::rtcp::ReportBlock;

/// RFC 3550 A.1 — 이만큼 벌어지면 감긴 것이 아니라 새로 시작한 것으로 본다.
const MAX_DROPOUT: u32 = 3_000;
const RTP_SEQ_MOD: u32 = 1 << 16;

#[derive(Debug, Default)]
pub struct Reception {
    started: AtomicU32,
    base_seq: AtomicU32,
    max_seq: AtomicU32,
    cycles: AtomicU32,
    received: AtomicU64,
    expected_prior: AtomicU64,
    received_prior: AtomicU64,
    /// 1/16 단위 고정소수 — RFC 3550 A.8 과 같은 표현.
    jitter: AtomicU32,
    transit: AtomicI64,
    last_sr_middle: AtomicU32,
    last_sr_at_ms: AtomicU64,
}

impl Reception {
    /// ★핫패스 — 패킷 하나를 센다. `clock_rate` 는 kind 가 정한다(opus 48k · video 90k).
    pub fn observe(&self, seq: u16, rtp_ts: u32, arrival_ms: u64, clock_rate: u32) {
        let seq32 = u32::from(seq);
        if self.started.swap(1, Ordering::AcqRel) == 0 {
            self.base_seq.store(seq32, Ordering::Relaxed);
            self.max_seq.store(seq32, Ordering::Relaxed);
        } else {
            let max = self.max_seq.load(Ordering::Relaxed);
            let delta = seq32.wrapping_sub(max) & 0xFFFF;
            if delta < MAX_DROPOUT {
                if seq32 < max {
                    self.cycles.fetch_add(RTP_SEQ_MOD, Ordering::Relaxed);
                }
                self.max_seq.store(seq32, Ordering::Relaxed);
            }
            // 되돌아온(늦은) 패킷은 창을 뒤로 끌지 않는다 — 정§11-1 의 순서 역전 규율과 같은 정신.
        }
        self.received.fetch_add(1, Ordering::Relaxed);
        self.update_jitter(rtp_ts, arrival_ms, clock_rate);
    }

    /// RFC 3550 A.8 — 도착 간격과 RTP 간격의 차이를 1/16 로 감쇠 누적한다.
    fn update_jitter(&self, rtp_ts: u32, arrival_ms: u64, clock_rate: u32) {
        if clock_rate == 0 {
            return;
        }
        let arrival_rtp = (arrival_ms.wrapping_mul(u64::from(clock_rate)) / 1_000) as i64;
        let transit = arrival_rtp - i64::from(rtp_ts as i32);
        let prev = self.transit.swap(transit, Ordering::AcqRel);
        if prev == 0 {
            return;
        }
        let d = (transit - prev).unsigned_abs() as u32;
        let j = self.jitter.load(Ordering::Relaxed);
        self.jitter.store(j + (d.saturating_sub(j / 16 * 16)) / 16, Ordering::Relaxed);
    }

    /// 발행자 SR 이 왔다 — RR 의 `last_sr`·`delay_since_last_sr` 의 기준점.
    pub fn on_sender_report(&self, ntp: u64, now_ms: u64) {
        self.last_sr_middle.store(super::rtcp::ntp_middle(ntp), Ordering::Relaxed);
        self.last_sr_at_ms.store(now_ms, Ordering::Relaxed);
    }

    pub fn packets(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    pub fn seen_any(&self) -> bool {
        self.started.load(Ordering::Acquire) == 1
    }

    /// 정§11-2 — ★구간 델타로 낸다(1,000ms). 조립 함수를 두 곳에서 부르면 구간이 갈려 값이 틀린다.
    pub fn report(&self, ssrc: u32, now_ms: u64) -> Option<ReportBlock> {
        if !self.seen_any() {
            return None;
        }
        let extended = u64::from(self.cycles.load(Ordering::Relaxed)) + u64::from(self.max_seq.load(Ordering::Relaxed));
        let expected = extended.saturating_sub(u64::from(self.base_seq.load(Ordering::Relaxed))) + 1;
        let received = self.received.load(Ordering::Relaxed);
        let expected_delta = expected.saturating_sub(self.expected_prior.swap(expected, Ordering::AcqRel));
        let received_delta = received.saturating_sub(self.received_prior.swap(received, Ordering::AcqRel));
        let fraction = if expected_delta == 0 || received_delta >= expected_delta {
            0
        } else {
            u8::try_from((expected_delta - received_delta) * 256 / expected_delta).unwrap_or(255)
        };
        let last_sr = self.last_sr_middle.load(Ordering::Relaxed);
        let since = self.last_sr_at_ms.load(Ordering::Relaxed);
        Some(ReportBlock {
            ssrc,
            fraction_lost: fraction,
            // 24비트 부호값 — 음수(중복 수신)는 0 으로 눕힌다.
            cumulative_lost: u32::try_from(expected.saturating_sub(received)).unwrap_or(0) & 0x00FF_FFFF,
            highest_seq: u32::try_from(extended).unwrap_or(u32::MAX),
            jitter: self.jitter.load(Ordering::Relaxed) / 16,
            last_sr,
            delay_since_last_sr: if last_sr == 0 { 0 } else { super::rtcp::delay_units(now_ms.saturating_sub(since)) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPUS: u32 = 48_000;

    #[test]
    fn a_clean_run_reports_no_loss() {
        let r = Reception::default();
        assert!(r.report(1, 0).is_none(), "관찰 전엔 보고할 것이 없다 — 0 이 아니라 없음이다");
        for i in 0..100u16 {
            r.observe(1_000 + i, u32::from(i) * 960, u64::from(i) * 20, OPUS);
        }
        let b = r.report(0xABCD, 2_000).unwrap();
        assert_eq!((b.ssrc, b.fraction_lost, b.cumulative_lost), (0xABCD, 0, 0));
        assert_eq!((b.highest_seq, r.packets()), (1_099, 100));
        assert_eq!((b.last_sr, b.delay_since_last_sr), (0, 0), "SR 을 못 받았으면 기준점이 없다");
    }

    #[test]
    fn loss_shows_up_in_the_window_not_forever() {
        let r = Reception::default();
        for i in 0..100u16 {
            if i % 10 == 0 {
                continue;
            }
            r.observe(i, u32::from(i) * 960, u64::from(i) * 20, OPUS);
        }
        // 첫 수신이 seq 1 이라 그것이 base 다 — 그 앞의 결번(seq 0)은 손실이 아니다(RFC 3550 A.3).
        let first = r.report(1, 2_000).unwrap();
        assert!(first.fraction_lost > 20 && first.cumulative_lost == 9, "{first:?}");
        // 다음 구간은 깨끗하다 — 누적은 남되 분율은 0 이어야 한다(구간 델타).
        for i in 100..200u16 {
            r.observe(i, u32::from(i) * 960, u64::from(i) * 20, OPUS);
        }
        let second = r.report(1, 4_000).unwrap();
        assert_eq!((second.fraction_lost, second.cumulative_lost), (0, 9));
    }

    #[test]
    fn sequence_wrap_is_a_cycle_not_a_restart() {
        let r = Reception::default();
        for seq in [65_530u16, 65_531, 65_532, 65_533, 65_534, 65_535, 0, 1, 2] {
            r.observe(seq, u32::from(seq) * 960, 0, OPUS);
        }
        let b = r.report(1, 0).unwrap();
        assert_eq!(b.highest_seq, 65_536 + 2, "감김은 cycles 로 센다");
        assert_eq!((b.cumulative_lost, r.packets()), (0, 9));
    }

    #[test]
    fn jitter_grows_with_irregular_arrival_and_the_sr_anchors_the_delay() {
        let steady = Reception::default();
        let jumpy = Reception::default();
        for i in 0..50u16 {
            let ts = u32::from(i) * 960;
            steady.observe(i, ts, u64::from(i) * 20, OPUS);
            jumpy.observe(i, ts, u64::from(i) * 20 + u64::from(i % 2) * 30, OPUS);
        }
        let (a, b) = (steady.report(1, 0).unwrap(), jumpy.report(1, 0).unwrap());
        assert!(b.jitter > a.jitter, "고르지 않게 오면 지터가 는다 {a:?} {b:?}");

        jumpy.on_sender_report(0x1234_5678_9ABC_DEF0, 5_000);
        let c = jumpy.report(1, 6_000).unwrap();
        assert_eq!((c.last_sr, c.delay_since_last_sr), (0x5678_9ABC, 65_536));
    }
}
