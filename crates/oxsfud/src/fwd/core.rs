// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:RULE-CONC-SANS-IO · 정:IMPL-SUB-SUBSCRIBER-TABLE · 연:RULE-LINK-SOURCE-SWITCH · model: claude-opus-5-5
//! 전달 산수(sans-IO) — 다운링크 SSRC 하나의 재기록기(스칼라 오프셋 하나 · 전환점 재계산) · serial 비교 · PT 바이트 · SR 번역.

pub fn seq_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

pub fn pt_byte1(byte1: u8, pt: u8) -> u8 {
    (byte1 & 0x80) | (pt & 0x7f)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Last {
    pub seq: u16,
    pub ts: u32,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Rewriter {
    pub src: Option<u64>,
    pub seq_off: u16,
    pub ts_off: u32,
    pub last: Option<Last>,
    pub boundary: Option<u16>,
    pub switches: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapped {
    pub seq: u16,
    pub ts: u32,
    pub switched: bool,
}

impl Rewriter {
    pub fn map(&mut self, src: u64, seq: u16, ts: u32, clock: u32, at_ms: u64) -> Mapped {
        let mut switched = false;
        if self.src != Some(src) {
            match self.last {
                None => {
                    self.seq_off = 0;
                    self.ts_off = 0;
                }
                Some(l) => {
                    let elapsed = at_ms.saturating_sub(l.at_ms).max(1);
                    let tick = ((elapsed as u128 * clock as u128) / 1000).max(1) as u32;
                    self.seq_off = l.seq.wrapping_add(1).wrapping_sub(seq);
                    self.ts_off = l.ts.wrapping_add(tick).wrapping_sub(ts);
                    switched = true;
                    self.switches += 1;
                }
            }
            self.src = Some(src);
            self.boundary = Some(seq.wrapping_add(self.seq_off));
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        let advance = match self.last {
            None => true,
            Some(l) => switched || seq_after(out_seq, l.seq),
        };
        if advance {
            self.last = Some(Last { seq: out_seq, ts: out_ts, at_ms });
        }
        Mapped { seq: out_seq, ts: out_ts, switched }
    }

    pub fn nack(&self, out_seq: u16) -> Option<u16> {
        self.src?;
        if let Some(b) = self.boundary
            && b != out_seq && seq_after(b, out_seq) {
                return None;
            }
        Some(out_seq.wrapping_sub(self.seq_off))
    }

    pub fn sr_ts(&self, rtp_ts: u32) -> u32 {
        rtp_ts.wrapping_add(self.ts_off)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sr {
    pub ssrc: u32,
    pub ntp: u64,
    pub rtp_ts: u32,
}

pub fn translate_sr(sr: Sr, downlink_ssrc: u32, ts_off: u32) -> Sr {
    Sr { ssrc: downlink_ssrc, ntp: sr.ntp, rtp_ts: sr.rtp_ts.wrapping_add(ts_off) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rtp<'a> {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc: &'a [u8],
    pub profile: u16,
    pub ext: &'a [u8],
    pub payload: &'a [u8],
    pub padding: &'a [u8],
}

pub fn parse(b: &[u8]) -> Option<Rtp<'_>> {
    if b.len() < 12 || b[0] >> 6 != 2 {
        return None;
    }
    let cc = (b[0] & 0x0f) as usize;
    let has_ext = b[0] & 0x10 != 0;
    let has_pad = b[0] & 0x20 != 0;
    let mut i = 12 + cc * 4;
    if b.len() < i {
        return None;
    }
    let csrc = &b[12..i];
    let (mut profile, mut ext): (u16, &[u8]) = (0, &[]);
    if has_ext {
        if b.len() < i + 4 {
            return None;
        }
        profile = u16::from_be_bytes([b[i], b[i + 1]]);
        let words = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let end = i + 4 + words * 4;
        if b.len() < end {
            return None;
        }
        ext = &b[i + 4..end];
        i = end;
    }
    let pad = if has_pad { *b.last()? as usize } else { 0 };
    if pad > b.len() - i {
        return None;
    }
    let end = b.len() - pad;
    Some(Rtp {
        marker: b[1] & 0x80 != 0,
        pt: b[1] & 0x7f,
        seq: u16::from_be_bytes([b[2], b[3]]),
        ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        csrc,
        profile,
        ext,
        payload: &b[i..end],
        padding: &b[end..],
    })
}

pub struct Egress<'a> {
    pub byte1: u8,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc: &'a [u8],
    pub ext: &'a [u8],
    pub payload: &'a [u8],
    pub padding: &'a [u8],
}

pub fn build(e: &Egress<'_>, out: &mut Vec<u8>) {
    out.clear();
    let cc = (e.csrc.len() / 4) as u8;
    let mut b0 = 0x80 | (cc & 0x0f);
    if !e.ext.is_empty() {
        b0 |= 0x10;
    }
    if !e.padding.is_empty() {
        b0 |= 0x20;
    }
    out.push(b0);
    out.push(pt_byte1(e.byte1, e.pt));
    out.extend_from_slice(&e.seq.to_be_bytes());
    out.extend_from_slice(&e.ts.to_be_bytes());
    out.extend_from_slice(&e.ssrc.to_be_bytes());
    out.extend_from_slice(e.csrc);
    out.extend_from_slice(e.ext);
    out.extend_from_slice(e.payload);
    out.extend_from_slice(e.padding);
}

pub fn keyframe(codec: &str, p: &[u8]) -> bool {
    if codec.eq_ignore_ascii_case("vp8") {
        let Some(&d0) = p.first() else { return false };
        let s = d0 & 0x10 != 0;
        let pid = d0 & 0x07;
        let mut i = 1;
        if d0 & 0x80 != 0 {
            let Some(&x) = p.get(1) else { return false };
            i = 2;
            if x & 0x80 != 0 {
                let Some(&m) = p.get(i) else { return false };
                i += if m & 0x80 != 0 { 2 } else { 1 };
            }
            if x & 0x40 != 0 {
                i += 1;
            }
            if x & 0x30 != 0 {
                i += 1;
            }
        }
        return s && pid == 0 && p.get(i).map(|h| h & 0x01 == 0).unwrap_or(false);
    }
    if codec.eq_ignore_ascii_case("h264") {
        let Some(&h) = p.first() else { return false };
        return match h & 0x1f {
            5 | 7 => true,
            24 => {
                let mut i = 1;
                while i + 2 < p.len() {
                    let n = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                    if matches!(p[i + 2] & 0x1f, 5 | 7) {
                        return true;
                    }
                    i += 2 + n;
                }
                false
            }
            28 => p.get(1).map(|f| f & 0x80 != 0 && matches!(f & 0x1f, 5 | 7)).unwrap_or(false),
            _ => false,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_build_round_trip_keeps_marker_and_padding() {
        let mut pkt = vec![0xa0, 0x80 | 96, 0, 7, 0, 0, 0, 9, 0, 0, 0, 1];
        pkt.extend_from_slice(&[1, 2, 3, 0, 0, 2]);
        let r = parse(&pkt).unwrap_or_else(|| panic!("parse"));
        assert!(r.marker);
        assert_eq!((r.payload, r.padding.len()), (&[1u8, 2, 3, 0][..], 2));
        let mut out = Vec::new();
        build(&Egress { byte1: pkt[1], pt: 100, seq: 7, ts: 9, ssrc: 1, csrc: r.csrc, ext: &[], payload: r.payload, padding: r.padding }, &mut out);
        assert_eq!(out[1], 0x80 | 100, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 PT 교체 marker 보존");
        assert_eq!(out.len(), pkt.len());
    }

    #[test]
    fn keyframes() {
        assert!(keyframe("H264", &[0x65]));
        assert!(keyframe("H264", &[0x7c, 0x85]));
        assert!(!keyframe("H264", &[0x41]));
        assert!(keyframe("VP8", &[0x10, 0x00]));
        assert!(!keyframe("VP8", &[0x10, 0x01]));
    }

    #[test]
    fn first_carry_is_identity_and_switch_is_contiguous() {
        let mut r = Rewriter::default();
        let a = r.map(1, 100, 5000, 48_000, 1000);
        assert_eq!((a.seq, a.ts), (100, 5000));
        r.map(1, 101, 5960, 48_000, 1020);
        let s = r.map(2, 9000, 777_000, 48_000, 1060);
        assert_eq!((s.seq, s.ts), (102, 7880), "S-sym-IMPL-FWD-SOURCE-SWITCH-2 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS");
        assert_eq!(r.nack(101), None);
        assert_eq!(r.nack(102), Some(9000));
    }
}
