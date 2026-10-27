// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-CONC-SANS-IO · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:CAND-FWD-REWRITER-LOCK · 정:IMPL-SUB-SUBSCRIBER-TABLE · 연:RULE-LINK-SOURCE-SWITCH · 연:RULE-LINK-DOWNLINK-SSRC · model: claude-opus-5-5
//! 재기록기(순수) — 스칼라 오프셋 하나 · 전환점에서 다시 셈 · 하한 = 「마지막」이 덮는 길이 · RFC 1982 serial · NACK 되돌림 · SR 번역 · PT byte1. 시계는 인자다.

pub const OPUS_CLOCK: u32 = 48_000;
pub const VIDEO_CLOCK: u32 = 90_000;

pub fn serial_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

pub fn opus_covers(payload: &[u8]) -> u32 {
    let Some(&toc) = payload.first() else { return 1 };
    let config = toc >> 3;
    let frame_us: u32 = match config {
        0..=11 => [10_000, 20_000, 40_000, 60_000][usize::from(config % 4)],
        12..=15 => [10_000, 20_000][usize::from(config % 2)],
        _ => [2_500, 5_000, 10_000, 20_000][usize::from(config % 4)],
    };
    let frames: u32 = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => match payload.get(1) {
            Some(b) => u32::from(b & 0x3f),
            None => return 1,
        },
    };
    (frame_us * frames * 48 / 1_000).max(1)
}

pub fn covers(clock: u32, payload: &[u8]) -> u32 {
    if clock == OPUS_CLOCK { opus_covers(payload) } else { 1 }
}

pub fn pt_byte1(byte1: u8, pt: u8) -> u8 {
    (byte1 & 0x80) | (pt & 0x7f)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapped {
    pub seq: u16,
    pub ts: u32,
    pub switched: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Last {
    seq: u16,
    ts: u32,
    at_ms: u64,
    covers: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Rewriter {
    src: Option<u64>,
    seq_off: u16,
    ts_off: u32,
    last: Option<Last>,
    boundary: Option<u16>,
    switches: u32,
}

impl Rewriter {
    pub fn new() -> Rewriter {
        Rewriter::default()
    }

    pub fn source(&self) -> Option<u64> {
        self.src
    }

    pub fn ts_off(&self) -> u32 {
        self.ts_off
    }

    pub fn seq_off(&self) -> u16 {
        self.seq_off
    }

    pub fn last_seq(&self) -> Option<u16> {
        self.last.map(|l| l.seq)
    }

    pub fn last_ts(&self) -> Option<u32> {
        self.last.map(|l| l.ts)
    }

    pub fn last_at_ms(&self) -> Option<u64> {
        self.last.map(|l| l.at_ms)
    }

    pub fn switches(&self) -> u32 {
        self.switches
    }

    pub fn map(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, clock: u32, covers: u32) -> Mapped {
        let mut switched = false;
        if self.src != Some(src) {
            switched = self.src.is_some();
            if switched {
                self.switches += 1;
            }
            match self.last {
                None => {
                    self.seq_off = 0;
                    self.ts_off = 0;
                }
                Some(l) => {
                    self.seq_off = l.seq.wrapping_add(1).wrapping_sub(seq);
                    let elapsed = at_ms.saturating_sub(l.at_ms).max(1);
                    let by_time = (u128::from(elapsed) * u128::from(clock) / 1_000) as u64;
                    let tick = by_time.max(u64::from(l.covers));
                    self.ts_off = (u64::from(l.ts).wrapping_add(tick).wrapping_sub(u64::from(ts))) as u32;
                }
            }
            self.src = Some(src);
            self.boundary = Some(seq.wrapping_add(self.seq_off));
        }
        let out = Mapped { seq: seq.wrapping_add(self.seq_off), ts: ts.wrapping_add(self.ts_off), switched };
        let advance = match self.last {
            None => true,
            Some(l) => serial_after(out.seq, l.seq),
        };
        if advance || switched {
            self.last = Some(Last { seq: out.seq, ts: out.ts, at_ms, covers });
        }
        out
    }

    pub fn nack_to_source(&self, out_seq: u16) -> Option<u16> {
        let b = self.boundary?;
        if out_seq == b || serial_after(out_seq, b) {
            Some(out_seq.wrapping_sub(self.seq_off))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SrIn {
    pub ssrc: u32,
    pub ntp: u64,
    pub rtp_ts: u32,
}

pub fn translate_sr(sr: SrIn, downlink_ssrc: u32, ts_off: u32) -> SrIn {
    SrIn { ssrc: downlink_ssrc, ntp: sr.ntp, rtp_ts: sr.rtp_ts.wrapping_add(ts_off) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_packet_keeps_original() {
        let mut r = Rewriter::new();
        let m = r.map(7, 100, 5_000, 0, OPUS_CLOCK, 960);
        assert_eq!((m.seq, m.ts, m.switched), (100, 5_000, false));
        assert_eq!(r.nack_to_source(100), Some(100));
        assert_eq!(r.nack_to_source(99), None);
    }

    #[test]
    fn switch_floor_is_previous_cover() {
        let mut r = Rewriter::new();
        r.map(1, 10, 1_000, 2_000, OPUS_CLOCK, 960);
        let m = r.map(2, 50, 90_000, 2_000, OPUS_CLOCK, 960);
        assert_eq!((m.seq, m.ts), (11, 1_960), "S-sym-IMPL-FWD-SOURCE-SWITCH-3 경과 0 이어도 앞 프레임이 덮는 960");
    }

    #[test]
    fn opus_toc_table() {
        assert_eq!(opus_covers(&[0xf8]), 960);
        assert_eq!(opus_covers(&[0x10, 0x00]), 1_920);
        assert_eq!(opus_covers(&[0xf3]), 1);
        assert_eq!(opus_covers(&[]), 1);
        assert_eq!(opus_covers(&[0xe0]), 120);
        assert_eq!(pt_byte1(239, 96), 224);
    }
}
