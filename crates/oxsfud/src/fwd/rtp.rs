// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-FWD-SOURCE-SWITCH 정:IMPL-SUB-SUBSCRIBER-TABLE 정:RULE-CONC-SANS-IO 연:RULE-LINK-EGRESS-REWRITE · model: claude-opus-5-5
//! RTP 머리 읽기 · 확장 값 읽기(one-byte · two-byte) · egress 다시 짓기(헤더 확장 영역만 구독자 것으로 · 페이로드 · 패딩 원본) · RTX 꾸러미(OSN). 시계 · I/O 없음.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc_end: usize,
    pub ext: Option<(usize, usize, u16)>,
    pub payload: usize,
    pub pad: usize,
}

pub fn is_rtcp(b: &[u8]) -> bool {
    b.len() >= 2 && (192..=223).contains(&b[1])
}

pub fn parse(b: &[u8]) -> Option<Head> {
    if b.len() < 12 || b[0] >> 6 != 2 {
        return None;
    }
    let cc = (b[0] & 0x0f) as usize;
    let x = b[0] & 0x10 != 0;
    let p = b[0] & 0x20 != 0;
    let mut off = 12 + 4 * cc;
    if b.len() < off {
        return None;
    }
    let csrc_end = off;
    let mut ext = None;
    if x {
        if b.len() < off + 4 {
            return None;
        }
        let profile = u16::from_be_bytes([b[off], b[off + 1]]);
        let words = u16::from_be_bytes([b[off + 2], b[off + 3]]) as usize;
        let start = off + 4;
        let end = start + words * 4;
        if b.len() < end {
            return None;
        }
        ext = Some((start, end, profile));
        off = end;
    }
    let pad = if p { b.last().copied().unwrap_or(0) as usize } else { 0 };
    if off + pad > b.len() {
        return None;
    }
    Some(Head {
        marker: b[1] & 0x80 != 0,
        pt: b[1] & 0x7f,
        seq: u16::from_be_bytes([b[2], b[3]]),
        ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        csrc_end,
        ext,
        payload: off,
        pad,
    })
}

pub fn ext_value<'a>(b: &'a [u8], h: &Head, id: u8) -> Option<&'a [u8]> {
    let (start, end, profile) = h.ext?;
    let mut i = start;
    if profile == 0xBEDE {
        while i < end {
            let x = b[i];
            if x == 0 {
                i += 1;
                continue;
            }
            let eid = x >> 4;
            let len = (x & 0x0f) as usize + 1;
            if eid == 15 {
                return None;
            }
            let v = i + 1;
            if v + len > end {
                return None;
            }
            if eid == id {
                return Some(&b[v..v + len]);
            }
            i = v + len;
        }
        None
    } else if profile & 0xFFF0 == 0x1000 {
        while i + 1 < end {
            let eid = b[i];
            if eid == 0 {
                i += 1;
                continue;
            }
            let len = b[i + 1] as usize;
            let v = i + 2;
            if v + len > end {
                return None;
            }
            if eid == id {
                return Some(&b[v..v + len]);
            }
            i = v + len;
        }
        None
    } else {
        None
    }
}

pub fn payload<'a>(b: &'a [u8], h: &Head) -> &'a [u8] {
    &b[h.payload..b.len() - h.pad]
}

pub struct Rewrite<'a> {
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub ext: &'a [u8],
}

pub fn rebuild(src: &[u8], h: &Head, w: &Rewrite<'_>, out: &mut Vec<u8>) {
    out.clear();
    let has_ext = !w.ext.is_empty();
    let b0 = (src[0] & 0xef) | if has_ext { 0x10 } else { 0 };
    out.push(b0);
    out.push(crate::fwd::core::pt_byte1(src[1], w.pt));
    out.extend_from_slice(&w.seq.to_be_bytes());
    out.extend_from_slice(&w.ts.to_be_bytes());
    out.extend_from_slice(&w.ssrc.to_be_bytes());
    out.extend_from_slice(&src[12..h.csrc_end]);
    out.extend_from_slice(w.ext);
    out.extend_from_slice(&src[h.payload..]);
}

pub fn rtx(src: &[u8], h: &Head, osn: u16, w: &Rewrite<'_>, out: &mut Vec<u8>) {
    out.clear();
    let has_ext = !w.ext.is_empty();
    let b0 = (src[0] & 0xcf) | if has_ext { 0x10 } else { 0 };
    out.push(b0);
    out.push((src[1] & 0x80) | (w.pt & 0x7f));
    out.extend_from_slice(&w.seq.to_be_bytes());
    out.extend_from_slice(&w.ts.to_be_bytes());
    out.extend_from_slice(&w.ssrc.to_be_bytes());
    out.extend_from_slice(&src[12..h.csrc_end]);
    out.extend_from_slice(w.ext);
    out.extend_from_slice(&osn.to_be_bytes());
    out.extend_from_slice(payload(src, h));
}

pub fn padding_probe(w: &Rewrite<'_>, osn: u16, pad: usize, out: &mut Vec<u8>) {
    out.clear();
    out.push(0x80 | 0x20 | if w.ext.is_empty() { 0 } else { 0x10 });
    out.push(w.pt & 0x7f);
    out.extend_from_slice(&w.seq.to_be_bytes());
    out.extend_from_slice(&w.ts.to_be_bytes());
    out.extend_from_slice(&w.ssrc.to_be_bytes());
    out.extend_from_slice(w.ext);
    out.extend_from_slice(&osn.to_be_bytes());
    let n = pad.clamp(1, 255);
    out.extend(std::iter::repeat_n(0u8, n - 1));
    out.push(n as u8);
}

pub fn synth(pt: u8, seq: u16, ts: u32, ssrc: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + payload.len());
    out.push(0x80);
    out.push((pt & 0x7f) | if marker { 0x80 } else { 0 });
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt() -> Vec<u8> {
        let mut b = vec![0x90, 0xe0, 0x00, 0x05, 0, 0, 0, 9, 0, 0, 0, 7];
        b.extend_from_slice(&[0xBE, 0xDE, 0, 1, 0x10, b'3', 0x30, 0x88]);
        b.extend_from_slice(&[1, 2, 3]);
        b
    }

    #[test]
    fn parse_and_read_ext() {
        let b = pkt();
        let h = parse(&b).expect("parse");
        assert!(h.marker);
        assert_eq!(h.pt, 0x60);
        assert_eq!((h.seq, h.ts, h.ssrc), (5, 9, 7));
        assert_eq!(ext_value(&b, &h, 1), Some(&b"3"[..]));
        assert_eq!(ext_value(&b, &h, 3), Some(&[0x88u8][..]));
        assert_eq!(payload(&b, &h), &[1, 2, 3]);
    }

    #[test]
    fn rebuild_swaps_ext_and_keeps_marker() {
        let b = pkt();
        let h = parse(&b).expect("parse");
        let mut out = Vec::new();
        rebuild(&b, &h, &Rewrite { pt: 111, seq: 100, ts: 200, ssrc: 300, ext: &[0xBE, 0xDE, 0, 1, 0x10, b'7', 0, 0] }, &mut out);
        let g = parse(&out).expect("parse out");
        assert!(g.marker, "marker 비트는 원본 그대로");
        assert_eq!((g.pt, g.seq, g.ts, g.ssrc), (111, 100, 200, 300));
        assert_eq!(ext_value(&out, &g, 1), Some(&b"7"[..]));
        assert_eq!(payload(&out, &g), &[1, 2, 3]);
        let mut r = Vec::new();
        rtx(&b, &h, 100, &Rewrite { pt: 97, seq: 1, ts: 200, ssrc: 9, ext: &[] }, &mut r);
        let rh = parse(&r).expect("rtx");
        assert_eq!(&payload(&r, &rh)[..2], &100u16.to_be_bytes(), "RTX OSN = 원 seq");
    }
}
