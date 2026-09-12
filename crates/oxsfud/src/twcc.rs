// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§11-2 · §10-3 · model: claude-opus-5

//! TWCC — ★★**시간축은 서버가 보낸 시각이다.**
//!
//! 피드백이 싣고 오는 도착시각은 ★**구독자 시계**라 우리 시계와 견줄 수 없다 —
//! ★**쓰는 것은 차분(도착 간격)뿐**이고, 절대 시각을 섞으면 ★**시계 오프셋이 그대로 추세가 된다.**
//!
//! 그래서 egress twcc seq 는 ★**서버가 교체 스탬핑**한다 — 발행자 값을 통과시키면
//! 여러 발행자의 seq 가 한 구독 전송로에 섞여 ★**구독자 피드백이 무의미**해진다.
//!
//! 이 모듈이 하는 일은 둘 — ★**보낸 것을 적는 장부**와 ★**피드백 읽기**(RTPFB fmt 15).
//! 추정은 [`crate::gcc`] 몫이다(장부는 사실만 쥔다).

use std::collections::BTreeMap;

use crate::gcc::Sample;

/// RTPFB.
const PT_RTPFB: u8 = 205;
/// transport-wide cc feedback.
const FMT_TWCC: u8 = 15;

/// 한 전송로의 송신 장부. ★**우리 시각과 크기만 담는다.**
#[derive(Debug, Default)]
pub struct SendLedger {
    /// egress twcc seq → (우리가 보낸 시각 ms, 크기 byte).
    sent: BTreeMap<u16, (u64, u16)>,
    next_seq: u16,
}

/// 피드백 한 건 — 구독자가 말한 것 그대로.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    pub media_ssrc: u32,
    pub base_seq: u16,
    /// 24비트 부호 있는 값 — ★**×64ms 가 기준 시각**이다(구독자 시계).
    pub reference_time_raw: i32,
    /// 순서대로 `Some(delta ×250µs)` 또는 ★`None`(못 받음).
    pub packets: Vec<Option<i64>>,
}

impl SendLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★**서버가 번호를 매긴다** — 발행자 값을 통과시키지 않는다.
    pub fn stamp(&mut self, now: u64, size: u16) -> u16 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.sent.insert(seq, (now, size));
        // 장부가 무한히 자라지 않게 걷는다 — 핫패스 밖(피드백 주기)에서 해도 된다.
        while self.sent.len() > 4_096 {
            let Some((&k, _)) = self.sent.iter().next() else { break };
            self.sent.remove(&k);
        }
        seq
    }

    /// 피드백을 장부와 대조해 표본을 만든다. 반환 = `(표본, 못 받았다는 수)`.
    ///
    /// ★**두 시계를 한 값에 섞지 않는다** — `send_ms` 는 우리 것, `arrival_ms` 는 구독자
    /// 것으로 따로 담아 넘기고, 오프셋 상쇄는 추정기가 차분으로 한다.
    /// ★**장부에 없는 seq 는 표본에서 뺀다**(덮여 사라진 것은 *"안 보냈다"* 가 아니다).
    pub fn samples(&mut self, fb: &Feedback) -> (Vec<Sample>, usize) {
        let mut out = Vec::with_capacity(fb.packets.len());
        let mut lost = 0usize;
        let mut arrival_ms = fb.reference_time_raw as f64 * 64.0;
        for (i, p) in fb.packets.iter().enumerate() {
            let seq = fb.base_seq.wrapping_add(i as u16);
            match p {
                Some(d) => {
                    arrival_ms += *d as f64 * 0.25;
                    if let Some(&(sent_ms, size)) = self.sent.get(&seq) {
                        out.push(Sample { send_ms: sent_ms as f64, arrival_ms, size });
                    }
                }
                None => lost += 1,
            }
            self.sent.remove(&seq);
        }
        (out, lost)
    }
}

/// 피드백 읽기 — ★**우리가 짓지 않는 청크 형도 읽는다**(브라우저가 보내는 것이 정본이다).
pub fn parse(pkt: &[u8]) -> Option<Feedback> {
    if pkt.len() < 20 || pkt[1] != PT_RTPFB || (pkt[0] & 0x1F) != FMT_TWCC {
        return None;
    }
    let media_ssrc = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
    let base_seq = u16::from_be_bytes([pkt[12], pkt[13]]);
    let count = u16::from_be_bytes([pkt[14], pkt[15]]) as usize;
    let reference_time_raw = {
        let raw = ((pkt[16] as i32) << 16) | ((pkt[17] as i32) << 8) | (pkt[18] as i32);
        // 24비트 부호 확장.
        (raw << 8) >> 8
    };
    // ★길이 필드가 경계다 — 버퍼 끝까지 읽으면 패딩을 자료로 읽는다.
    let words = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
    let end = ((words + 1) * 4).min(pkt.len());

    // 1차 — 상태 심볼(0=못 받음, 1=작은 델타, 2=큰 델타).
    let mut status: Vec<u8> = Vec::with_capacity(count);
    let mut at = 20;
    while status.len() < count {
        if at + 2 > end {
            return None;
        }
        let chunk = u16::from_be_bytes([pkt[at], pkt[at + 1]]);
        at += 2;
        if chunk & 0x8000 == 0 {
            // 같은 심볼이 이어지는 형.
            let sym = ((chunk >> 13) & 0x03) as u8;
            let run = (chunk & 0x1FFF) as usize;
            for _ in 0..run.min(count - status.len()) {
                status.push(sym);
            }
        } else if chunk & 0x4000 == 0 {
            // 1비트 열 14칸.
            for i in 0..14 {
                if status.len() >= count {
                    break;
                }
                status.push(((chunk >> (13 - i)) & 1) as u8);
            }
        } else {
            // 2비트 열 7칸.
            for i in 0..7 {
                if status.len() >= count {
                    break;
                }
                status.push(((chunk >> (12 - i * 2)) & 0x03) as u8);
            }
        }
    }

    // 2차 — 받았다는 것만 델타를 먹는다.
    let mut packets: Vec<Option<i64>> = Vec::with_capacity(count);
    for &s in &status {
        match s {
            1 => {
                if at + 1 > end {
                    return None;
                }
                packets.push(Some(pkt[at] as i64));
                at += 1;
            }
            2 => {
                if at + 2 > end {
                    return None;
                }
                packets.push(Some(i16::from_be_bytes([pkt[at], pkt[at + 1]]) as i64));
                at += 2;
            }
            // 0 = 못 받음, 3 = 예약(받은 것으로 치지 않는다).
            _ => packets.push(None),
        }
    }
    Some(Feedback { media_ssrc, base_seq, reference_time_raw, packets })
}

/// 프로브 패딩 한 개 — ★**RTX 로 보낸다**(약속된 pt·ssrc 라 구독자가 조용히 버린다).
///
/// ★본문은 뜻이 없다 — 마지막 바이트가 패딩 길이다(RFC 3550 P 비트).
/// ★타임스탬프를 `0` 으로 두는 까닭 — ★**미디어 타임라인에 손대지 않겠다**는 뜻이다.
pub fn probe_padding(rtx_ssrc: u32, rtx_seq: u16, rtx_pt: u8, pad: usize) -> Vec<u8> {
    let pad = pad.clamp(1, 255);
    let mut p = Vec::with_capacity(12 + pad);
    // V=2, P=1, X=0, CC=0.
    p.push(0xA0);
    p.push(rtx_pt & 0x7F);
    p.extend_from_slice(&rtx_seq.to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&rtx_ssrc.to_be_bytes());
    p.resize(12 + pad, 0);
    *p.last_mut().expect("길이 1 이상") = pad as u8;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 서버가_번호를_매긴다() {
        // ★발행자 값을 통과시키면 여러 발행자의 seq 가 한 전송로에 섞인다.
        let mut a = SendLedger::new();
        let mut b = SendLedger::new();
        assert_eq!(a.stamp(0, 1200), 0);
        assert_eq!(a.stamp(1, 1200), 1);
        assert_eq!(b.stamp(0, 1200), 0, "★전송로마다 제 공간이다");
    }

    #[test]
    fn 두_시계를_한_값에_안_섞는다() {
        let mut l = SendLedger::new();
        let seqs: Vec<u16> = (0..4).map(|i| l.stamp(1_000 + i * 20, 1_000)).collect();
        // 구독자 시계가 아주 앞서 있다 — 간격은 같다.
        let fb = Feedback {
            media_ssrc: 1,
            base_seq: seqs[0],
            // 64ms 단위 — 100,000 × 64 = 6,400,000ms 앞.
            reference_time_raw: 100_000,
            packets: vec![Some(0), Some(80), Some(80), Some(80)],
        };
        let (s, lost) = l.samples(&fb);
        assert_eq!((s.len(), lost), (4, 0));
        // ★우리 값은 우리 것대로, 그쪽 값은 그쪽 것대로 담긴다.
        assert_eq!(s[0].send_ms, 1_000.0);
        assert!(s[0].arrival_ms > 6_000_000.0, "{}", s[0].arrival_ms);
        // 간격은 양쪽 모두 20ms — 차분끼리 빼면 0 이다(추정기가 그렇게 쓴다).
        assert_eq!(s[1].send_ms - s[0].send_ms, 20.0);
        assert_eq!(s[1].arrival_ms - s[0].arrival_ms, 20.0);
    }

    #[test]
    fn 못_받은_것은_표본이_아니라_계수다() {
        let mut l = SendLedger::new();
        let s0 = l.stamp(0, 1200);
        let _s1 = l.stamp(20, 1200);
        let fb = Feedback {
            media_ssrc: 1,
            base_seq: s0,
            reference_time_raw: 0,
            packets: vec![Some(400), None],
        };
        let (s, lost) = l.samples(&fb);
        assert_eq!((s.len(), lost), (1, 1));
    }

    #[test]
    fn 장부에_없는_것은_표본에서_뺀다() {
        // ★덮여 사라진 자리를 "안 보냈다" 로 읽으면 추정이 흔들린다.
        let mut l = SendLedger::new();
        let fb = Feedback {
            media_ssrc: 1,
            base_seq: 7,
            reference_time_raw: 0,
            packets: vec![Some(4)],
        };
        let (s, lost) = l.samples(&fb);
        assert!(s.is_empty());
        assert_eq!(lost, 0, "★받았다는 말을 유실로 뒤집지 않는다");
    }

    /// 2비트 열 한 칸 — `[받음(1), 못 받음(0), 받음(1)]`.
    fn fb_bytes() -> Vec<u8> {
        let mut p = vec![0x80 | FMT_TWCC, PT_RTPFB, 0, 0];
        p.extend_from_slice(&1u32.to_be_bytes()); // sender ssrc
        p.extend_from_slice(&0xABCDu32.to_be_bytes()); // media ssrc
        p.extend_from_slice(&5u16.to_be_bytes()); // base seq
        p.extend_from_slice(&3u16.to_be_bytes()); // count
        p.extend_from_slice(&[0, 0, 2]); // ref time = 2 (×64ms)
        p.push(0); // fb count
        // 2비트 열: 받음(1) · 못 받음(0) · 받음(1).
        let chunk: u16 = 0xC000 | (1 << 12) | (1 << 8);
        p.extend_from_slice(&chunk.to_be_bytes());
        p.extend_from_slice(&[8, 12]); // 작은 델타 둘
        let words = (p.len() / 4 - 1) as u16;
        p[2..4].copy_from_slice(&words.to_be_bytes());
        p
    }

    #[test]
    fn 피드백을_읽는다() {
        let f = parse(&fb_bytes()).expect("읽힌다");
        assert_eq!(f.media_ssrc, 0xABCD);
        assert_eq!(f.base_seq, 5);
        assert_eq!(f.reference_time_raw, 2);
        assert_eq!(f.packets, vec![Some(8), None, Some(12)]);
    }

    #[test]
    fn 남의_rtcp_는_안_읽는다() {
        let mut p = fb_bytes();
        p[1] = 200; // SR
        assert!(parse(&p).is_none());
        let mut q = fb_bytes();
        q[0] = 0x80 | 1; // NACK
        assert!(parse(&q).is_none(), "★fmt 1 은 NACK 이다");
        assert!(parse(&[0u8; 8]).is_none());
    }

    #[test]
    fn 길이_필드가_경계다() {
        // ★버퍼 끝까지 읽으면 패딩을 자료로 읽는다.
        let mut p = fb_bytes();
        p.extend_from_slice(&[0xFF; 16]);
        let f = parse(&p).expect("읽힌다");
        assert_eq!(f.packets.len(), 3);
    }

    #[test]
    fn 기준시각은_음수도_된다() {
        let mut p = fb_bytes();
        p[16..19].copy_from_slice(&[0xFF, 0xFF, 0xFE]);
        assert_eq!(parse(&p).expect("읽힌다").reference_time_raw, -2);
    }

    #[test]
    fn 프로브_패딩은_길이를_꼬리에_적는다() {
        let p = probe_padding(0x1234, 9, 97, 255);
        assert_eq!(p[0] & 0x20, 0x20, "★P 비트가 선다");
        assert_eq!(p[1] & 0x7F, 97);
        assert_eq!(u16::from_be_bytes([p[2], p[3]]), 9);
        assert_eq!(u32::from_be_bytes([p[8], p[9], p[10], p[11]]), 0x1234);
        assert_eq!(*p.last().expect("있다") as usize, p.len() - 12);
    }
}
