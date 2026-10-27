// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:RULE-CONC-SANS-IO · 정:REJ-SUB-EXT-SWAP · model: claude-opus-5-5
//! RTP 머리(순수) — 읽기 · egress 다시 짓기(확장 영역만 구독자 것 · 페이로드 · 패딩은 원본 그대로 · PT 는 byte1 하위 7비트) · RTX 감싸기/풀기.

use super::core::pt_byte1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rtp<'a> {
    pub b: &'a [u8],
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub csrc_end: usize,
    pub ext: Option<(usize, usize)>,
    pub payload_start: usize,
    pub payload_end: usize,
}

pub fn parse(b: &[u8]) -> Option<Rtp<'_>> {
    if b.len() < 12 || b[0] >> 6 != 2 {
        return None;
    }
    let cc = usize::from(b[0] & 0x0f);
    let csrc_end = 12 + cc * 4;
    if b.len() < csrc_end {
        return None;
    }
    let mut i = csrc_end;
    let mut ext = None;
    if b[0] & 0x10 != 0 {
        if b.len() < i + 4 {
            return None;
        }
        let words = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        let end = i + 4 + words * 4;
        if b.len() < end {
            return None;
        }
        ext = Some((i, end));
        i = end;
    }
    let mut end = b.len();
    if b[0] & 0x20 != 0 {
        let pad = usize::from(*b.last()?);
        if pad == 0 || pad > end - i {
            return None;
        }
        end -= pad;
    }
    Some(Rtp {
        b,
        marker: b[1] & 0x80 != 0,
        pt: b[1] & 0x7f,
        seq: u16::from_be_bytes([b[2], b[3]]),
        ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        csrc_end,
        ext,
        payload_start: i,
        payload_end: end,
    })
}

impl<'a> Rtp<'a> {
    pub fn payload(&self) -> &'a [u8] {
        &self.b[self.payload_start..self.payload_end]
    }

    pub fn ext_block(&self) -> Option<&'a [u8]> {
        self.ext.map(|(s, e)| &self.b[s..e])
    }

    pub fn ext_value(&self, id: Option<u8>) -> Option<&'a [u8]> {
        crate::sub::ext::read(self.ext_block()?, id?)
    }
}

pub fn rebuild(rtp: &Rtp<'_>, ssrc: u32, seq: u16, ts: u32, pt: u8, ext_block: &[u8]) -> Vec<u8> {
    let b = rtp.b;
    let mut out = Vec::with_capacity(b.len() + ext_block.len());
    let ext_bit = if ext_block.is_empty() { 0 } else { 0x10 };
    out.push(0x80 | (b[0] & 0x20) | ext_bit | (b[0] & 0x0f));
    out.push(pt_byte1(b[1], pt));
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&b[12..rtp.csrc_end]);
    out.extend_from_slice(ext_block);
    out.extend_from_slice(&b[rtp.payload_start..]);
    out
}

pub fn rtx_wrap(rtp: &Rtp<'_>, rtx_ssrc: u32, rtx_seq: u16, ts: u32, rtx_pt: u8, osn: u16, ext_block: &[u8]) -> Vec<u8> {
    let b = rtp.b;
    let mut out = Vec::with_capacity(b.len() + ext_block.len() + 2);
    let ext_bit = if ext_block.is_empty() { 0 } else { 0x10 };
    out.push(0x80 | ext_bit | (b[0] & 0x0f));
    out.push(pt_byte1(b[1], rtx_pt));
    out.extend_from_slice(&rtx_seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&rtx_ssrc.to_be_bytes());
    out.extend_from_slice(&b[12..rtp.csrc_end]);
    out.extend_from_slice(ext_block);
    out.extend_from_slice(&osn.to_be_bytes());
    out.extend_from_slice(rtp.payload());
    out
}

pub fn rtx_unwrap(rtp: &Rtp<'_>, ssrc: u32, pt: u8) -> Option<Vec<u8>> {
    let p = rtp.payload();
    if p.len() < 2 {
        return None;
    }
    let osn = u16::from_be_bytes([p[0], p[1]]);
    let b = rtp.b;
    let mut out = Vec::with_capacity(b.len());
    out.push(b[0] & !0x20);
    out.push(pt_byte1(b[1], pt));
    out.extend_from_slice(&osn.to_be_bytes());
    out.extend_from_slice(&b[4..8]);
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&b[12..rtp.payload_start]);
    out.extend_from_slice(&p[2..]);
    Some(out)
}

pub fn synth(ssrc: u32, pt: u8, seq: u16, ts: u32, marker: bool, ext_block: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + ext_block.len() + payload.len());
    out.push(if ext_block.is_empty() { 0x80 } else { 0x90 });
    out.push((if marker { 0x80 } else { 0 }) | (pt & 0x7f));
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(ext_block);
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_keeps_payload_and_padding_and_marker() {
        let mut p = vec![0xb0, 0xe0, 0, 5, 0, 0, 0, 9, 0, 0, 0, 7];
        p.extend_from_slice(&[0xbe, 0xde, 0, 1, 0x10, 0x41, 0, 0]);
        p.extend_from_slice(&[1, 2, 3, 0, 0, 3]);
        let r = parse(&p).expect("rtp");
        assert_eq!(r.payload(), &[1, 2, 3]);
        assert_eq!(r.ext_value(Some(1)), Some(&[0x41][..]));
        let out = rebuild(&r, 99, 6, 10, 102, &[]);
        let o = parse(&out).expect("out");
        assert_eq!((o.ssrc, o.seq, o.ts, o.pt, o.marker), (99, 6, 10, 102, true));
        assert_eq!(o.payload(), &[1, 2, 3]);
        assert!(o.ext.is_none());
        let rtx = rtx_wrap(&o, 5, 1, 10, 103, 6, &[]);
        let back = rtx_unwrap(&parse(&rtx).expect("rtx"), 99, 102).expect("unwrap");
        let b = parse(&back).expect("back");
        assert_eq!((b.seq, b.pt), (6, 102));
        assert_eq!(b.payload(), &[1, 2, 3]);
    }
}
