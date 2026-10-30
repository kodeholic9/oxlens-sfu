// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:TRN.frame.DC · 연:RULE-FLR-TRANSPORT-DC · model: claude-opus-5-5
//! DC 프레임 — 4바이트 머리(ver · svc · len) + payload. 잘린 프레임은 조용히 버린다.

pub const VER: u8 = 1;
pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = 65_532;
pub const SVC_FLOOR: u8 = 0x01;
pub const SVC_VOICE_ACTIVITY: u8 = 0x02;
pub const CHANNEL_LABEL: &str = "unreliable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DcFrame<'a> {
    pub svc: u8,
    pub payload: &'a [u8],
}

pub fn decode(buf: &[u8]) -> Option<DcFrame<'_>> {
    if buf.len() < HEADER_LEN || buf[0] != VER {
        return None;
    }
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if buf.len() < HEADER_LEN + len {
        return None;
    }
    Some(DcFrame { svc: buf[1], payload: &buf[HEADER_LEN..HEADER_LEN + len] })
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
    fn roundtrip_and_truncated() {
        let b = encode(SVC_FLOOR, &[1, 2, 3]).unwrap();
        assert_eq!(decode(&b), Some(DcFrame { svc: SVC_FLOOR, payload: &[1, 2, 3] }));
        assert_eq!(decode(&b[..5]), None);
        assert!(encode(SVC_FLOOR, &vec![0; MAX_PAYLOAD + 1]).is_none());
    }
}
