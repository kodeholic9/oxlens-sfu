// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§7-3 · §8-1 · 연§4-1 · MASTER §핫패스 H1·H5 · model: claude-opus-5

//! egress 재기록 — ★★**전송로마다 단일 스칼라 offset.**
//!
//! ★**입력 seq 별 구간 지도(range map)를 두지 않는다**(20260622 폐기) — 발행자가 바뀔 때
//! offset 이 충돌해 ★**egress seq 가 역행**하고, 수신 NetEQ 가 그것을 버려 ★**무전 음성이 끊긴다.**
//!
//! ★**제자리에서 고치고 길이를 바꾸지 않는다**(H1) — SRTP 태그 계산의 전제다.

/// RTP 고정 헤더 길이. 확장·CSRC 는 그 뒤다.
const RTP_HEADER_LEN: usize = 12;

/// 한 전송로(구독자 하나의 한 스트림)의 재기록 상태.
///
/// ★**스칼라 둘이 전부다** — 그래서 화자가 바뀌어도 충돌할 자리가 없다.
#[derive(Debug, Clone, Default)]
pub struct Rewriter {
    /// 지금까지 내보낸 마지막 egress seq.
    last_out_seq: Option<u16>,
    /// 지금 출처의 입력 seq 에서 egress seq 로 가는 차이.
    seq_offset: u16,
    ts_offset: u32,
    /// ★**출처가 갈린 자리** — 여기가 egress seq 공간의 경계다.
    source: Option<u32>,
    last_out_ts: u32,
}

/// 한 패킷의 재기록 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rewritten {
    pub seq: u16,
    pub ts: u32,
}

impl Rewriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★**출처가 갈렸나** — 화자 교대·레이어 전환이 그것이다.
    fn on_source_change(&mut self, ssrc: u32, in_seq: u16, in_ts: u32) {
        // ★새 출처의 첫 패킷이 **직전 egress 의 바로 다음**이 되도록 offset 을 다시 잡는다.
        //   구간 지도라면 여기서 옛 구간과 겹쳐 역행이 난다.
        let next = self.last_out_seq.map(|s| s.wrapping_add(1)).unwrap_or(in_seq);
        self.seq_offset = next.wrapping_sub(in_seq);
        // ts 는 단조여야 한다 — 역행하면 수신자가 망 손실로 오인한다.
        let next_ts = self.last_out_ts.wrapping_add(1);
        self.ts_offset = next_ts.wrapping_sub(in_ts);
        self.source = Some(ssrc);
    }

    /// 값만 계산한다 — ★**바이트는 `apply` 가 제자리에서 고친다.**
    pub fn map(&mut self, ssrc: u32, in_seq: u16, in_ts: u32) -> Rewritten {
        if self.source != Some(ssrc) {
            if self.source.is_none() {
                // 첫 패킷 — 그대로 이어 간다.
                self.source = Some(ssrc);
                self.seq_offset = 0;
                self.ts_offset = 0;
            } else {
                self.on_source_change(ssrc, in_seq, in_ts);
            }
        }
        let seq = in_seq.wrapping_add(self.seq_offset);
        let ts = in_ts.wrapping_add(self.ts_offset);
        self.last_out_seq = Some(seq);
        self.last_out_ts = ts;
        Rewritten { seq, ts }
    }

    /// ★**제자리 재기록** — 버퍼를 새로 잡지 않고 길이를 바꾸지 않는다.
    ///
    /// ★`pt` 는 ★**byte1 만 교체**한다(구독자 표의 값 — 발행자 값이 아니다).
    pub fn apply(buf: &mut [u8], out: Rewritten, pt: u8) -> Result<(), &'static str> {
        if buf.len() < RTP_HEADER_LEN {
            return Err("RTP 헤더보다 짧다");
        }
        // marker 비트는 보존하고 PT 7비트만 바꾼다.
        buf[1] = (buf[1] & 0x80) | (pt & 0x7f);
        buf[2..4].copy_from_slice(&out.seq.to_be_bytes());
        buf[4..8].copy_from_slice(&out.ts.to_be_bytes());
        Ok(())
    }

    pub fn last_seq(&self) -> Option<u16> {
        self.last_out_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(pt: u8, seq: u16, ts: u32, marker: bool) -> Vec<u8> {
        let mut v = vec![0x80, pt | if marker { 0x80 } else { 0 }, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4];
        v[2..4].copy_from_slice(&seq.to_be_bytes());
        v[4..8].copy_from_slice(&ts.to_be_bytes());
        v
    }

    #[test]
    fn 화자가_바뀌어도_seq_가_역행하지_않는다() {
        // ★구간 지도면 여기서 옛 구간과 겹쳐 역행이 나고, 그 결과가 무전 음성 드롭아웃이다.
        let mut r = Rewriter::new();
        let mut last = 0u16;
        for s in 100..110u16 {
            last = r.map(0xAAAA, s, s as u32 * 960).seq;
        }
        // 새 화자 — 입력 seq 가 훨씬 작다(제 공간의 시작값).
        let first = r.map(0xBBBB, 7, 1_000).seq;
        assert_eq!(first, last.wrapping_add(1), "★직전 egress 의 바로 다음이어야 한다");
        let mut prev = first;
        for s in 8..20u16 {
            let now = r.map(0xBBBB, s, 1_000 + s as u32 * 960).seq;
            assert_eq!(now, prev.wrapping_add(1), "★연속이어야 한다");
            prev = now;
        }
    }

    #[test]
    fn 같은_출처는_구멍을_안_만든다() {
        let mut r = Rewriter::new();
        let a = r.map(1, 65_534, 10).seq;
        let b = r.map(1, 65_535, 20).seq;
        let c = r.map(1, 0, 30).seq;
        // ★u16 이 감기는 자리에서도 이어진다.
        assert_eq!(b, a.wrapping_add(1));
        assert_eq!(c, b.wrapping_add(1));
    }

    #[test]
    fn ts_는_역행하지_않는다() {
        // ★역행하면 수신자가 망 손실로 오인해 NACK·은닉이 돈다.
        let mut r = Rewriter::new();
        let a = r.map(1, 1, 1_000_000).ts;
        // 새 출처의 ts 가 훨씬 작다.
        let b = r.map(2, 1, 10).ts;
        assert!(b > a, "새 출처의 첫 ts {b} 가 직전 {a} 보다 커야 한다");
    }

    #[test]
    fn 제자리에서_고치고_길이를_안_바꾼다() {
        // ★SRTP 태그 계산의 전제다 — 길이가 바뀌면 그 자리가 통째로 깨진다.
        let mut buf = pkt(96, 7, 700, true);
        let before = buf.len();
        Rewriter::apply(&mut buf, Rewritten { seq: 9, ts: 900 }, 100).expect("apply");
        assert_eq!(buf.len(), before);
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), 9);
        assert_eq!(u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]), 900);
        assert_eq!(buf[1] & 0x7f, 100, "★PT 는 구독자 표의 값이다");
        assert_eq!(buf[1] & 0x80, 0x80, "★marker 는 보존한다");
    }

    #[test]
    fn 짧은_버퍼는_건드리지_않는다() {
        let mut buf = vec![0x80, 96, 0, 0];
        assert!(Rewriter::apply(&mut buf, Rewritten { seq: 1, ts: 1 }, 100).is_err());
    }

    #[test]
    fn 전송로마다_따로다() {
        // ★한 발행자를 두 구독자가 받아도 각자의 공간이다 — 공유하면 한쪽의 교대가 다른 쪽을 흔든다.
        let mut a = Rewriter::new();
        let mut b = Rewriter::new();
        a.map(1, 500, 0);
        b.map(1, 500, 0);
        a.map(2, 1, 0);
        assert_eq!(b.map(1, 501, 960).seq, 501, "★b 는 교대를 안 겪었다");
    }
}
