// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:RULE-CONC-SANS-IO · 정:IMPL-SUB-SUBSCRIBER-TABLE · model: claude-opus-5-5
//! 재기록기 — 다운링크 SSRC 하나의 단일 오프셋(seq · ts · VP8 picture ID · TL0PICIDX) · 전환 하한 = 앞 패킷이 덮는 길이 · NACK 되돌림 · SR 번역 · PT 교체. 시계는 인자.

pub const M15: u16 = 0x8000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8Pic {
    pub pid: Option<u16>,
    pub pid_at: usize,
    pub wide: bool,
    pub tl0: Option<u8>,
    pub tl0_at: usize,
}

pub fn vp8_pic(p: &[u8]) -> Option<Vp8Pic> {
    let b0 = *p.first()?;
    if b0 & 0x80 == 0 {
        return None;
    }
    let x = *p.get(1)?;
    let mut i = 2;
    let mut d = Vp8Pic { pid: None, pid_at: 0, wide: false, tl0: None, tl0_at: 0 };
    if x & 0x80 != 0 {
        let h = *p.get(i)?;
        d.wide = h & 0x80 != 0;
        d.pid_at = i;
        if d.wide {
            let l = *p.get(i + 1)?;
            d.pid = Some((((h & 0x7f) as u16) << 8) | l as u16);
            i += 2;
        } else {
            d.pid = Some((h & 0x7f) as u16);
            i += 1;
        }
    }
    if x & 0x40 != 0 {
        d.tl0 = Some(*p.get(i)?);
        d.tl0_at = i;
    }
    Some(d)
}

pub fn vp8_write(p: &mut [u8], d: &Vp8Pic, pid: Option<u16>, tl0: Option<u8>) {
    if let (Some(_), Some(v)) = (d.pid, pid) {
        if d.wide && d.pid_at + 1 < p.len() {
            p[d.pid_at] = 0x80 | ((v >> 8) as u8 & 0x7f);
            p[d.pid_at + 1] = v as u8;
        } else if !d.wide && d.pid_at < p.len() {
            p[d.pid_at] = (p[d.pid_at] & 0x80) | (v as u8 & 0x7f);
        }
    }
    if let (Some(_), Some(v)) = (d.tl0, tl0)
        && d.tl0_at < p.len()
    {
        p[d.tl0_at] = v;
    }
}

pub fn serial_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

pub fn serial_before(a: u16, b: u16) -> bool {
    a != b && !serial_after(a, b)
}

pub fn episode_src(src: u64, episode: u64) -> u64 {
    src.rotate_left(32) ^ episode
}

pub const BOUNDARY_WINDOW: u16 = 16384;

pub fn opus_toc_samples(payload: &[u8]) -> u32 {
    let Some(&toc) = payload.first() else { return 1 };
    let config = toc >> 3;
    let frame_x10: u32 = match config {
        0..=11 => [100, 200, 400, 600][(config % 4) as usize],
        12..=15 => [100, 200][(config % 2) as usize],
        _ => [25, 50, 100, 200][(config % 4) as usize],
    };
    let frames: u32 = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        _ => match payload.get(1) {
            Some(b) => (b & 0x3f) as u32,
            None => return 1,
        },
    };
    let samples = frame_x10 * 48 * frames / 10;
    samples.max(1)
}

pub fn covers(audio_opus: bool, payload: &[u8]) -> u32 {
    if audio_opus { opus_toc_samples(payload) } else { 1 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Last {
    pub seq: u16,
    pub ts: u32,
    pub at_ms: u64,
    pub covers: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Rewriter {
    pub clock: u32,
    pub src: Option<u64>,
    pub seq_off: u16,
    pub ts_off: u32,
    pub last: Option<Last>,
    pub boundary: Option<u16>,
    pub switches: u32,
    pub last_out_at: u64,
    pub pid_off: u16,
    pub tl0_off: u8,
    pub last_pid: Option<u16>,
    pub last_tl0: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapped {
    pub seq: u16,
    pub ts: u32,
    pub switched: bool,
    pub pid: Option<u16>,
    pub tl0: Option<u8>,
}

impl Rewriter {
    pub fn new(clock: u32) -> Rewriter {
        Rewriter { clock, ..Default::default() }
    }

    pub fn map(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, covers: u32) -> Option<Mapped> {
        self.map_pic(src, seq, ts, at_ms, covers, None)
    }

    pub fn pic_out(&self, pic: Option<&Vp8Pic>) -> (Option<u16>, Option<u8>) {
        let pid = pic.and_then(|d| d.pid).map(|v| v.wrapping_add(self.pid_off) % M15);
        let tl0 = pic.and_then(|d| d.tl0).map(|v| v.wrapping_add(self.tl0_off));
        (pid, tl0)
    }

    pub fn map_pic(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64, covers: u32, pic: Option<&Vp8Pic>) -> Option<Mapped> {
        let mut switched = false;
        if self.src != Some(src) {
            self.pid_off = match (self.last_pid, pic.and_then(|d| d.pid)) {
                (Some(lp), Some(p)) => lp.wrapping_add(1).wrapping_sub(p) % M15,
                _ => 0,
            };
            self.tl0_off = match (self.last_tl0, pic.and_then(|d| d.tl0)) {
                (Some(lx), Some(x)) => lx.wrapping_add(1).wrapping_sub(x),
                _ => 0,
            };
            switched = self.src.is_some() || self.last.is_some();
            match self.last {
                None => {
                    self.seq_off = 0;
                    self.ts_off = 0;
                }
                Some(l) => {
                    let elapsed = at_ms.saturating_sub(l.at_ms).max(1);
                    let ticks = (elapsed as u128 * self.clock as u128 / 1000) as u64;
                    let tick = (l.covers.max(1) as u64).max(ticks) as u32;
                    self.seq_off = l.seq.wrapping_add(1).wrapping_sub(seq);
                    self.ts_off = l.ts.wrapping_add(tick).wrapping_sub(ts);
                }
            }
            if self.src.is_some() || self.last.is_some() {
                self.switches += 1;
            }
            self.src = Some(src);
            self.boundary = Some(seq.wrapping_add(self.seq_off));
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        if let Some(b) = self.boundary
            && serial_before(out_seq, b)
        {
            return None;
        }
        let advance = match self.last {
            None => true,
            Some(l) => switched || serial_after(out_seq, l.seq),
        };
        let (pid, tl0) = self.pic_out(pic);
        if advance {
            self.last = Some(Last { seq: out_seq, ts: out_ts, at_ms, covers });
            if pic.is_some() {
                self.last_pid = pid.or(self.last_pid);
                self.last_tl0 = tl0.or(self.last_tl0);
            }
        }
        if let (Some(b), Some(l)) = (self.boundary, self.last)
            && l.seq.wrapping_sub(b) >= BOUNDARY_WINDOW
        {
            self.boundary = None;
        }
        self.last_out_at = at_ms;
        Some(Mapped { seq: out_seq, ts: out_ts, switched, pid, tl0 })
    }

    pub fn unmap_nack(&self, out_seq: u16) -> Option<u16> {
        self.src?;
        if let Some(b) = self.boundary
            && serial_before(out_seq, b)
        {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }

    pub fn translate_sr_ts(&self, rtp_ts: u32) -> u32 {
        rtp_ts.wrapping_add(self.ts_off)
    }
}

pub fn pt_byte1(byte1: u8, pt: u8) -> u8 {
    (byte1 & 0x80) | (pt & 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_lengths() {
        assert_eq!(opus_toc_samples(&[0xf8]), 960);
        assert_eq!(opus_toc_samples(&[]), 1);
        assert_eq!(opus_toc_samples(&[0x03]), 1);
        assert_eq!(opus_toc_samples(&[0x0b, 0x03]), 2880);
    }
}
