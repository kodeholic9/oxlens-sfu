// author: kodeholic (powered by Claude)
//! 정§9-7 — 허가 직후의 priming CN. 무전 슬롯은 조용할 때 아무것도 안 흐르므로
//! 화자가 말을 시작해도 수신 jitter buffer 가 차갑다 — ★첫 음절이 잘린다.
//!
//! ★슬롯 seq 공간을 쓰므로 실제 RTP 와 ★같은 rewriter 를 지나야 한다.
//! 따로 번호를 매기면 화자 첫 패킷과 겹치거나 역행해 그 자리가 곧 드롭아웃이다.

/// libwebrtc 가 DTX 에 쓰는 opus 무음 페이로드. 코덱이 아는 "여기는 조용하다" 다.
pub const OPUS_SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];
/// 20ms @ 48kHz.
pub const FRAME_TS_STEP: u32 = 960;
pub const FRAME_INTERVAL_MS: u64 = 20;
/// 상한 — 5초. 화자가 그때까지 말을 안 하면 데울 이유가 없다.
pub const MAX_FRAMES: u32 = 250;

/// 이 자리에서 쓰는 출처 이름. ★화자 이름과 달라야 rewriter 가 교대로 읽고 offset 을 잇는다.
pub const SOURCE: &str = "\u{0}priming";

/// 무음 RTP 한 장. `seq`·`ts` 는 입력 공간의 값이고 egress 값은 rewriter 가 정한다.
pub fn silence(pt: u8, seq: u16, ts: u32, ssrc: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + OPUS_SILENCE.len());
    out.push(0x80);
    out.push(pt & 0x7F);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&OPUS_SILENCE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::rewriter::Rewriter;
    use crate::media::rtp;

    #[test]
    fn a_silence_frame_is_rtp_the_codec_understands() {
        let pkt = silence(111, 7, 960, 0xABCD);
        assert_eq!((rtp::payload_type(&pkt), rtp::sequence(&pkt)), (Some(111), Some(7)));
        assert_eq!((rtp::timestamp(&pkt), rtp::ssrc(&pkt)), (Some(960), Some(0xABCD)));
        assert_eq!(rtp::payload(&pkt).unwrap(), &OPUS_SILENCE);
    }

    /// ★2층이 잡은 결함 — 데우기를 두 번 돌릴 때 입력 번호를 0 으로 되돌리면
    /// rewriter 가 출처 교대를 못 보고(같은 출처다) egress seq 가 역행한다.
    /// 역행한 패킷은 수신측이 낡은 것으로 버리므로 ★두 번째 화자는 데워지지 않는다.
    #[test]
    fn warming_twice_must_not_walk_the_sequence_backwards() {
        let r = Rewriter::default();
        let out = |r: &Rewriter, seq: u16, ts: u32| {
            let mut p = silence(111, seq, ts, 1);
            assert_ne!(r.rewrite(&mut p, SOURCE, 0x5150), crate::media::rewriter::Rewrite::Skip);
            rtp::sequence(&p).unwrap()
        };
        // 1차 데우기 — 이어지는 입력 번호.
        let first: Vec<u16> = (0..3u16).map(|i| out(&r, i, u32::from(i) * FRAME_TS_STEP)).collect();
        // 2차 데우기 — ★슬롯이 번호를 이어 준다(0 으로 되돌리지 않는다).
        let second: Vec<u16> = (3..6u16).map(|i| out(&r, i, u32::from(i) * FRAME_TS_STEP)).collect();
        assert!(
            second[0] > first[2],
            "★번호를 되돌리면 여기서 역행한다 — 수신측이 버려 두 번째 화자가 안 데워진다"
        );
        assert_eq!(second, vec![3, 4, 5]);
    }

    /// ★이 시험이 이 모듈의 존재 이유다 — 데우기가 화자 첫 패킷을 밀어내면 안 된다.
    #[test]
    fn warming_and_the_speaker_share_one_sequence_space() {
        let r = Rewriter::default();
        let mut out_seqs = Vec::new();
        for i in 0..3u16 {
            let mut pkt = silence(111, i, u32::from(i) * FRAME_TS_STEP, 1);
            assert_ne!(r.rewrite(&mut pkt, SOURCE, 0x5150), crate::media::rewriter::Rewrite::Skip);
            out_seqs.push(rtp::sequence(&pkt).unwrap());
        }
        // 화자의 진짜 RTP — 입력 seq 공간이 전혀 다르다.
        let mut real = silence(111, 40_000, 7_000_000, 2);
        assert_ne!(r.rewrite(&mut real, "u1", 0x5150), crate::media::rewriter::Rewrite::Skip);
        let first_real = rtp::sequence(&real).unwrap();

        assert_eq!(out_seqs, vec![0, 1, 2], "데우기는 이어진 번호를 쓴다");
        assert!(
            first_real > out_seqs[2],
            "★화자 첫 패킷이 데우기 뒤에 온다 — 역행하면 그 자리가 곧 드롭아웃이다"
        );
    }
}
