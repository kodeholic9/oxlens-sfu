// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-FWD-SOURCE-SWITCH 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS 정:PROP-FWD-SWITCH-TS-FORWARD 정:RULE-CONC-SANS-IO 정:IMPL-SUB-SUBSCRIBER-TABLE 연:RULE-LINK-SOURCE-SWITCH 연:RULE-LINK-DOWNLINK-SSRC · model: claude-opus-5-5
//! 재기록기 — 다운링크 스트림마다 단일 스칼라 오프셋 · 출처가 바뀌는 순간 다시 셈 · serial 비교(RFC 1982) · PT 교체(byte1) · SR 번역. 시계 · I/O 없음.

pub fn seq_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

pub fn pt_byte1(byte1: u8, pt: u8) -> u8 {
    (byte1 & 0x80) | (pt & 0x7f)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Last {
    pub seq: u16,
    pub ts: u32,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Rewriter {
    pub src: Option<u64>,
    pub seq_off: u16,
    pub ts_off: u32,
    pub last: Option<Last>,
    pub boundary: Option<u16>,
    pub switches: u32,
}

impl Rewriter {
    pub fn map(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, clock: u32) -> (u16, u32) {
        match self.last {
            None => {
                self.seq_off = 0;
                self.ts_off = 0;
                self.src = Some(src);
            }
            Some(last) if self.src != Some(src) => {
                let elapsed = at_ms.saturating_sub(last.at_ms).max(1);
                let tick = ((elapsed as u128 * clock as u128) / 1000).max(1) as u32;
                self.seq_off = last.seq.wrapping_add(1).wrapping_sub(seq);
                self.ts_off = last.ts.wrapping_add(tick).wrapping_sub(ts);
                self.src = Some(src);
                self.boundary = Some(last.seq.wrapping_add(1));
                self.switches += 1;
            }
            Some(_) => {}
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        let advance = match self.last {
            None => true,
            Some(l) => seq_after(out_seq, l.seq),
        };
        if advance {
            self.last = Some(Last { seq: out_seq, ts: out_ts, at_ms });
        }
        (out_seq, out_ts)
    }

    pub fn nack_to_source(&self, out_seq: u16) -> Option<u16> {
        if let Some(b) = self.boundary
            && seq_after(b, out_seq)
        {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sr {
    pub ssrc: u32,
    pub ntp: u64,
    pub rtp_ts: u32,
}

pub fn translate_sr(sr: Sr, downlink_ssrc: u32, ts_off: u32) -> Sr {
    Sr { ssrc: downlink_ssrc, ntp: sr.ntp, rtp_ts: sr.rtp_ts.wrapping_add(ts_off) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_carry_is_identity_and_switch_is_contiguous() {
        let mut r = Rewriter::default();
        assert_eq!(r.map(1, 100, 1000, 0, 48000), (100, 1000));
        assert_eq!(r.map(1, 101, 1960, 20, 48000), (101, 1960));
        let (s, t) = r.map(2, 9000, 5, 60, 48000);
        assert_eq!(s, 102);
        assert_eq!(t, 1960 + 1920);
        assert_eq!(r.nack_to_source(101), None);
        assert_eq!(r.nack_to_source(103), Some(9001));
    }
}
