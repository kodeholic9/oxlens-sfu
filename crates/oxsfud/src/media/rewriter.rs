// author: kodeholic (powered by Claude)
//! 재기록 — 정§8-1·§10-2. ★**출처가 바뀌어도 egress 가 이어져야 하는 자리**가 둘이다:
//! 반이중 방 슬롯(화자 교대)과 simulcast(레이어 전환). 둘 다 SSRC 하나에 여러 입력이 번갈아 든다.
//! 수신 NetEQ·jitter buffer 는 구멍을 망 손실로 읽으므로 이어붙이는 것이 계약이다.
//!
//! ★규칙은 ★**단일 스칼라 offset** 하나다. 입력 `seq` 마다 구간을 기억하는 사상표를 두면
//! 발행자가 바뀔 때 offset 이 충돌해 ★**egress seq 가 역행**하고, 그 결과가 무전 음성 드롭아웃이다.
//!
//! 자료가 다섯 낱말짜리 읽고-고치고-쓰기라 RCU 가 맞지 않는다(매 패킷 통째 교체 = 매 패킷 할당).
//! 슬롯 하나에 화자는 언제나 하나뿐이라 이 락은 경합하지 않는다.

use std::sync::Mutex;

use super::rtp;

/// 화자가 바뀔 때 벌리는 간격 — 수신측이 두 발화를 잇대어 읽지 않도록.
const HANDOVER_SEQ_GAP: u16 = 1;
/// 20ms @ 48kHz. 화자 교대에서 타임스탬프를 이만큼 앞으로 민다.
const HANDOVER_TS_STEP: u32 = 960;

#[derive(Debug, Default)]
struct Scalar {
    source: Option<String>,
    seq_offset: u16,
    ts_offset: u32,
    last_out_seq: u16,
    last_out_ts: u32,
    started: bool,
}

#[derive(Debug, Default)]
pub struct Rewriter {
    inner: Mutex<Scalar>,
}

/// 고쳤나, 그리고 ★**출처가 갈렸나.** 갈린 자리는 egress seq 공간의 경계라 그 앞의
/// 재전송 요구는 전부 stale 이다(정§11-1 ①) — 부르는 쪽이 그 사실을 알아야 캐시를 버린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rewrite {
    /// RTP 로 읽히지 않았다 — 흘리지 않는다.
    Skip,
    /// 같은 출처가 이어진다.
    Kept,
    /// ★출처 교대(화자 교대·레이어 전환) — 여기서 seq 공간이 갈렸다.
    Switched,
}

impl Rewriter {
    /// 그 출처의 패킷 하나를 하나의 egress 값으로 고친다 — SSRC 는 목적지 것, `seq`·`ts` 는 offset 하나로.
    /// `source` 는 반이중이면 화자, simulcast 면 rid 다. ★길이는 변하지 않는다.
    pub fn rewrite(&self, packet: &mut [u8], source: &str, out_ssrc: u32) -> Rewrite {
        let (Some(in_seq), Some(in_ts)) = (rtp::sequence(packet), rtp::timestamp(packet)) else {
            return Rewrite::Skip;
        };
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut switched = false;
        if !s.started {
            s.source = Some(source.to_owned());
            s.seq_offset = 0;
            s.ts_offset = 0;
            s.started = true;
        } else if s.source.as_deref() != Some(source) {
            switched = true;
            // 출처 교대(화자 교대·레이어 전환) — 이어지는 자리에서 다시 시작하도록 offset 만 새로 잡는다.
            s.source = Some(source.to_owned());
            s.seq_offset = s.last_out_seq.wrapping_add(HANDOVER_SEQ_GAP).wrapping_sub(in_seq);
            s.ts_offset = s.last_out_ts.wrapping_add(HANDOVER_TS_STEP).wrapping_sub(in_ts);
        }
        let out_seq = in_seq.wrapping_add(s.seq_offset);
        let out_ts = in_ts.wrapping_add(s.ts_offset);
        s.last_out_seq = out_seq;
        s.last_out_ts = out_ts;
        drop(s);
        rtp::set_sequence(packet, out_seq);
        rtp::set_timestamp(packet, out_ts);
        rtp::set_ssrc(packet, out_ssrc);
        if switched { Rewrite::Switched } else { Rewrite::Kept }
    }

    /// 출처가 통째로 없어지면 다음 것이 0 부터 이어 붙는다.
    pub fn reset(&self) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = Scalar::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(seq: u16, ts: u32, ssrc: u32) -> Vec<u8> {
        let mut p = vec![0x80, 0x6F, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        p.extend_from_slice(&[7; 20]);
        rtp::set_sequence(&mut p, seq);
        rtp::set_timestamp(&mut p, ts);
        rtp::set_ssrc(&mut p, ssrc);
        p
    }
    fn out(r: &Rewriter, seq: u16, ts: u32, ssrc: u32, who: &str) -> (u16, u32, u32) {
        let mut p = pkt(seq, ts, ssrc);
        let before = p.len();
        assert_ne!(r.rewrite(&mut p, who, 0xABCD), Rewrite::Skip);
        assert_eq!(p.len(), before, "길이 불변");
        (rtp::sequence(&p).unwrap(), rtp::timestamp(&p).unwrap(), rtp::ssrc(&p).unwrap())
    }

    #[test]
    fn first_speaker_passes_through_and_takes_the_slot_ssrc() {
        let r = Rewriter::default();
        assert_eq!(out(&r, 100, 5_000, 0x1111, "a"), (100, 5_000, 0xABCD));
        assert_eq!(out(&r, 101, 5_960, 0x1111, "a"), (101, 5_960, 0xABCD));
    }

    /// ★화자가 바뀌어도 egress 는 앞으로만 간다 — 이것이 무전 음성 드롭아웃을 막는 유일한 성질이다.
    #[test]
    fn handover_never_moves_egress_backwards() {
        let r = Rewriter::default();
        let mut last = out(&r, 60_000, 900_000, 0x1111, "a").0;
        for i in 1..5 {
            last = out(&r, 60_000 + i, 900_000 + u32::from(i) * 960, 0x1111, "a").0;
        }
        // 두 번째 화자는 완전히 다른 자리에서 시작한다(브라우저가 정한 값이라 낮을 수도 높을 수도 있다).
        let (seq, ts, ssrc) = out(&r, 7, 42, 0x2222, "b");
        assert_eq!((seq, ssrc), (last.wrapping_add(1), 0xABCD), "이어 붙는다");
        assert_eq!(ts, 900_000 + 4 * 960 + 960);
        let (seq2, ts2, _) = out(&r, 8, 42 + 960, 0x2222, "b");
        assert_eq!((seq2, ts2), (seq + 1, ts + 960), "같은 화자는 offset 하나로 계속 간다");

        // 세 번째 화자가 훨씬 큰 값에서 시작해도 역행이 없다.
        let (seq3, _, _) = out(&r, 65_000, 8_000_000, 0x3333, "c");
        assert_eq!(seq3, seq2.wrapping_add(1));
    }

    #[test]
    fn wrapping_is_the_normal_case_not_an_error() {
        let r = Rewriter::default();
        assert_eq!(out(&r, 65_535, 0xFFFF_FF00, 0x1111, "a").0, 65_535);
        let wrapped = 0xFFFF_FF00u32.wrapping_add(960);
        let (seq, ts, _) = out(&r, 0, wrapped, 0x1111, "a");
        assert_eq!((seq, ts), (0, wrapped), "같은 화자의 감김은 그대로 통과");
        let (seq2, _, _) = out(&r, 500, 10, 0x2222, "b");
        assert_eq!(seq2, 1, "교대도 감긴 자리에서 이어진다");
    }

    /// ★출처 교대를 부르는 쪽에 알린다 — 그 경계 앞의 재전송 요구는 stale 이다(정§11-1 ①).
    #[test]
    fn a_source_change_is_reported_so_the_caller_can_drop_its_cache() {
        let r = Rewriter::default();
        let mut p = pkt(100, 5_000, 0x1111);
        assert_eq!(r.rewrite(&mut p, "a", 0xABCD), Rewrite::Kept, "첫 출처는 교대가 아니다");
        let mut p = pkt(101, 5_960, 0x1111);
        assert_eq!(r.rewrite(&mut p, "a", 0xABCD), Rewrite::Kept);
        let mut p = pkt(7, 700, 0x2222);
        assert_eq!(r.rewrite(&mut p, "b", 0xABCD), Rewrite::Switched);
        let mut p = pkt(8, 1_660, 0x2222);
        assert_eq!(r.rewrite(&mut p, "b", 0xABCD), Rewrite::Kept, "교대는 한 번만 알린다");
    }

    #[test]
    fn reset_starts_over_and_short_packets_are_refused() {
        let r = Rewriter::default();
        out(&r, 100, 5_000, 0x1111, "a");
        r.reset();
        assert_eq!(out(&r, 9, 9, 0x1111, "b"), (9, 9, 0xABCD), "빈 슬롯은 그대로 통과");
        let mut short = vec![0x80, 0x6F, 0, 1];
        assert_eq!(r.rewrite(&mut short, "a", 1), Rewrite::Skip);
    }
}
