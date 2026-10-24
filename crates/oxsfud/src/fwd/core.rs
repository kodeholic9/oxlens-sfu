// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-FWD-SOURCE-SWITCH · 정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS · 정:PROP-FWD-SWITCH-TS-FORWARD · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:REJ-SUB-EXT-SWAP · 정:CAND-FWD-REWRITER-LOCK · 정:RULE-CONC-SANS-IO · 연:RULE-LINK-SOURCE-SWITCH · 연:RULE-LINK-DOWNLINK-SSRC · 연:RULE-LINK-EGRESS-REWRITE · 연:COND-LINK-REBUILD · model: claude-opus-5-5
//! 전달 산수(순수) — 단일 오프셋 재기록기 · serial 비교 · PT byte1 교체 · SR 번역 · egress 헤더 확장 다시 짓기.

pub const SEQ_MOD: u32 = 1 << 16;

pub fn seq_after(a: u16, b: u16) -> bool {
    (a.wrapping_sub(b) as i16) > 0
}

pub fn pt_byte1(byte1: u8, pt: u8) -> u8 {
    (byte1 & 0x80) | (pt & 0x7f)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Last {
    seq: u16,
    ts: u32,
    at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewriter {
    clock: u32,
    src: Option<u64>,
    seq_off: u16,
    ts_off: u32,
    last: Option<Last>,
    boundary: u16,
}

impl Rewriter {
    pub fn new(clock: u32) -> Rewriter {
        Rewriter { clock: clock.max(1), src: None, seq_off: 0, ts_off: 0, last: None, boundary: 0 }
    }

    pub fn source(&self) -> Option<u64> {
        self.src
    }

    pub fn seq_off(&self) -> u16 {
        self.seq_off
    }

    pub fn ts_off(&self) -> u32 {
        self.ts_off
    }

    pub fn last_out(&self) -> Option<(u16, u32)> {
        self.last.map(|l| (l.seq, l.ts))
    }

    pub fn will_switch(&self, src: u64) -> bool {
        self.src.is_some() && self.src != Some(src)
    }

    pub fn rewrite(&mut self, src: u64, seq: u16, ts: u32, at_ms: u64) -> (u16, u32) {
        match (self.src, self.last) {
            (None, _) => {
                self.src = Some(src);
                self.seq_off = 0;
                self.ts_off = 0;
                self.boundary = seq;
            }
            (Some(cur), Some(last)) if cur != src => {
                let elapsed = at_ms.saturating_sub(last.at_ms).max(1);
                let tick = ((elapsed as u128 * self.clock as u128) / 1000).max(1) as u32;
                self.seq_off = last.seq.wrapping_add(1).wrapping_sub(seq);
                self.ts_off = last.ts.wrapping_add(tick).wrapping_sub(ts);
                self.boundary = last.seq.wrapping_add(1);
                self.src = Some(src);
            }
            (Some(cur), None) if cur != src => {
                self.src = Some(src);
                self.seq_off = 0;
                self.ts_off = 0;
                self.boundary = seq;
            }
            _ => {}
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        let newer = match self.last {
            None => true,
            Some(l) => seq_after(out_seq, l.seq),
        };
        if newer {
            self.last = Some(Last { seq: out_seq, ts: out_ts, at_ms });
        }
        (out_seq, out_ts)
    }

    pub fn nack_in(&self, out_seq: u16) -> Option<u16> {
        self.src?;
        if out_seq != self.boundary && !seq_after(out_seq, self.boundary) {
            return None;
        }
        if let Some(l) = self.last
            && seq_after(out_seq, l.seq)
        {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }

    pub fn reset_source(&mut self) {
        if self.src.is_some() {
            self.src = Some(u64::MAX);
        }
    }

    pub fn sr_ts(&self, rtp_ts: u32) -> u32 {
        rtp_ts.wrapping_add(self.ts_off)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sr {
    pub ssrc: u32,
    pub ntp: u64,
    pub rtp_ts: u32,
}

pub fn translate_sr(input: Sr, downlink_ssrc: u32, ts_off: u32) -> Sr {
    Sr { ssrc: downlink_ssrc, ntp: input.ntp, rtp_ts: input.rtp_ts.wrapping_add(ts_off) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExtTable {
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
    pub abs_send_time: Option<u8>,
    pub twcc: Option<u8>,
}

impl ExtTable {
    fn ids(&self) -> impl Iterator<Item = u8> {
        [self.mid, self.audio_level, self.abs_send_time, self.twcc].into_iter().flatten()
    }

    pub fn two_byte(&self) -> bool {
        self.ids().any(|id| !(1..=14).contains(&id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtValues {
    pub mid: Option<String>,
    pub audio_level: Option<u8>,
    pub t_us: u64,
    pub twcc: Option<u16>,
}

pub fn abs_send_time(t_us: u64) -> u32 {
    let within = t_us % 64_000_000;
    ((within as u128 * (1u128 << 18)) / 1_000_000) as u32 & 0x00ff_ffff
}

pub fn ext_elements(table: &ExtTable, v: &ExtValues) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::with_capacity(4);
    if let (Some(id), Some(mid)) = (table.mid, v.mid.as_ref()) {
        out.push((id, mid.as_bytes().to_vec()));
    }
    if let (Some(id), Some(level)) = (table.audio_level, v.audio_level) {
        out.push((id, vec![level]));
    }
    if let Some(id) = table.abs_send_time {
        let a = abs_send_time(v.t_us);
        out.push((id, vec![(a >> 16) as u8, (a >> 8) as u8, a as u8]));
    }
    if let (Some(id), Some(n)) = (table.twcc, v.twcc) {
        out.push((id, n.to_be_bytes().to_vec()));
    }
    out
}

pub fn ext_block(table: &ExtTable, v: &ExtValues) -> Vec<u8> {
    let elems = ext_elements(table, v);
    if elems.is_empty() {
        return Vec::new();
    }
    let two = table.two_byte();
    let mut body = Vec::with_capacity(16);
    for (id, data) in &elems {
        if two {
            body.push(id.to_owned());
            body.push(data.len() as u8);
        } else {
            body.push((id << 4) | ((data.len() as u8).saturating_sub(1) & 0x0f));
        }
        body.extend_from_slice(data);
    }
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    let words = (body.len() / 4) as u16;
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(if two { &[0x10, 0x00] } else { &[0xbe, 0xde] });
    out.extend_from_slice(&words.to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpView {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub head_len: usize,
    pub ext_off: usize,
    pub ext_len: usize,
    pub ext_profile: u16,
    pub payload_off: usize,
    pub padding: usize,
}

pub fn parse_rtp(b: &[u8]) -> Option<RtpView> {
    if b.len() < 12 || b[0] >> 6 != 2 {
        return None;
    }
    let cc = (b[0] & 0x0f) as usize;
    let has_ext = b[0] & 0x10 != 0;
    let has_pad = b[0] & 0x20 != 0;
    let head_len = 12 + cc * 4;
    if b.len() < head_len {
        return None;
    }
    let (mut payload_off, mut ext_off, mut ext_len, mut ext_profile) = (head_len, 0, 0, 0);
    if has_ext {
        if b.len() < head_len + 4 {
            return None;
        }
        ext_profile = u16::from_be_bytes([b[head_len], b[head_len + 1]]);
        let words = u16::from_be_bytes([b[head_len + 2], b[head_len + 3]]) as usize;
        ext_off = head_len + 4;
        ext_len = words * 4;
        payload_off = ext_off + ext_len;
        if b.len() < payload_off {
            return None;
        }
    }
    let padding = if has_pad { *b.last()? as usize } else { 0 };
    if payload_off + padding > b.len() {
        return None;
    }
    Some(RtpView {
        marker: b[1] & 0x80 != 0,
        pt: b[1] & 0x7f,
        seq: u16::from_be_bytes([b[2], b[3]]),
        ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        head_len,
        ext_off,
        ext_len,
        ext_profile,
        payload_off,
        padding,
    })
}

pub fn read_ext<'a>(b: &'a [u8], v: &RtpView, id: u8) -> Option<&'a [u8]> {
    if id == 0 || v.ext_len == 0 {
        return None;
    }
    let area = &b[v.ext_off..v.ext_off + v.ext_len];
    let mut i = 0;
    if v.ext_profile == 0xbede {
        while i < area.len() {
            let h = area[i];
            if h == 0 {
                i += 1;
                continue;
            }
            let eid = h >> 4;
            let len = (h & 0x0f) as usize + 1;
            if eid == 15 {
                return None;
            }
            if i + 1 + len > area.len() {
                return None;
            }
            if eid == id {
                return Some(&area[i + 1..i + 1 + len]);
            }
            i += 1 + len;
        }
    } else if v.ext_profile & 0xfff0 == 0x1000 {
        while i + 1 < area.len() {
            let eid = area[i];
            if eid == 0 {
                i += 1;
                continue;
            }
            let len = area[i + 1] as usize;
            if i + 2 + len > area.len() {
                return None;
            }
            if eid == id {
                return Some(&area[i + 2..i + 2 + len]);
            }
            i += 2 + len;
        }
    }
    None
}

pub fn rebuild(b: &[u8], v: &RtpView, pt: u8, ssrc: u32, seq: u16, ts: u32, ext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + ext.len());
    let mut first = b[0] & !0x10;
    if !ext.is_empty() {
        first |= 0x10;
    }
    out.push(first);
    out.push(pt_byte1(b[1], pt));
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&b[12..v.head_len]);
    out.extend_from_slice(ext);
    out.extend_from_slice(&b[v.payload_off..]);
    out
}

pub fn payload<'a>(b: &'a [u8], v: &RtpView) -> &'a [u8] {
    &b[v.payload_off..b.len() - v.padding]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(state: &mut u64) -> u64 {
        let x = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let _ = std::mem::replace(state, x);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    #[test]
    fn prop_switch_seq_contiguous_and_ts_forward() {
        let mut rng = 0x5eed_u64;
        for round in 0..2000 {
            let clock = if round % 2 == 0 { 48_000 } else { 90_000 };
            let mut rw = Rewriter::new(clock);
            let mut at = next(&mut rng) % 1_000_000;
            let mut src = 1u64;
            let mut seq = (next(&mut rng) % 65_536) as u16;
            let mut ts = next(&mut rng) as u32;
            let steps = 2 + (next(&mut rng) % 30) as usize;
            let mut prev_out: Option<(u16, u32)> = None;
            for _ in 0..steps {
                let switch = next(&mut rng).is_multiple_of(4);
                if switch {
                    src += 1;
                    seq = (next(&mut rng) % 65_536) as u16;
                    ts = next(&mut rng) as u32;
                }
                at += next(&mut rng) % 200;
                let before_last = rw.last_out();
                let (os, ot) = rw.rewrite(src, seq, ts, at);
                if switch && let Some((ls, lt)) = before_last {
                    assert_eq!(os, ls.wrapping_add(1), "정:PROP-FWD-SWITCH-SEQ-CONTIGUOUS 새 출처 첫 seq 는 앞선 가장 앞선 seq 의 다음");
                    let d = ot.wrapping_sub(lt);
                    assert!((1..(1 << 31)).contains(&d), "정:PROP-FWD-SWITCH-TS-FORWARD 새 출처 첫 ts 는 앞으로 간다 (d={d})");
                }
                if !switch && let Some((ps, pt)) = prev_out {
                    assert_eq!(os, ps.wrapping_add(1), "한 출처 안에서는 들어온 순서 그대로");
                    assert_eq!(ot.wrapping_sub(pt), 960);
                }
                prev_out = Some((os, ot));
                seq = seq.wrapping_add(1);
                ts = ts.wrapping_add(960);
            }
        }
    }

    #[test]
    fn rtp_parse_and_rebuild_keep_payload() {
        let mut p = vec![0x90, 0xef, 0x12, 0x34, 0, 0, 0, 1, 0, 0, 0, 9, 0xbe, 0xde, 0, 1, 0x10, b'7', 0, 0];
        p.extend_from_slice(&[1, 2, 3]);
        let v = parse_rtp(&p).expect("rtp");
        assert_eq!(v.pt, 0x6f);
        assert!(v.marker);
        assert_eq!(read_ext(&p, &v, 1), Some(&b"7"[..]));
        let out = rebuild(&p, &v, 96, 42, 7, 9, &[]);
        assert_eq!(out[0] & 0x10, 0);
        assert_eq!(out[1], 0x80 | 96);
        assert_eq!(&out[12..], &[1, 2, 3]);
    }
}
