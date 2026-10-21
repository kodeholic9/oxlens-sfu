// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 연:TRN.frame.DC · model: claude-opus-5-5
//! DC 프레임 — 4B 헤더(ver · svc · len) + payload. 잘린 프레임은 조용히 버린다.

pub const VER: u8 = 0x01;
pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = 65_532;
pub const SVC_FLOOR: u8 = 0x01;
pub const SVC_VAD: u8 = 0x02;
pub const CHANNEL_LABEL: &str = "unreliable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcFrame<'a> {
    pub svc: u8,
    pub payload: &'a [u8],
}

pub fn decode(b: &[u8]) -> Option<DcFrame<'_>> {
    if b.len() < HEADER_LEN || b[0] != VER {
        return None;
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if b.len() < HEADER_LEN + len {
        return None;
    }
    Some(DcFrame { svc: b[1], payload: &b[HEADER_LEN..HEADER_LEN + len] })
}

pub fn encode(svc: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(VER);
    out.push(svc);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_truncation() {
        let b = encode(SVC_FLOOR, &[1, 2, 3]).expect("encode");
        let f = decode(&b).expect("decode");
        assert_eq!((f.svc, f.payload), (SVC_FLOOR, &[1u8, 2, 3][..]));
        assert!(decode(&b[..5]).is_none());
        assert!(encode(SVC_FLOOR, &vec![0; MAX_PAYLOAD + 1]).is_none());
    }
}
