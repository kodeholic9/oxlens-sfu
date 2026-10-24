// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:TRN.frame.DC · 정:IMPL-AUX-DC-SEND · model: claude-opus-5-5
//! DC 프레임 — ver · svc · len(BE) · payload. 잘린 프레임은 조용히 버린다.

pub const VERSION: u8 = 0x01;
pub const HEAD_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = 65_532;
pub const SVC_FLOOR: u8 = 0x01;
pub const SVC_VAD: u8 = 0x02;
pub const CHANNEL_LABEL: &str = "unreliable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DcFrame {
    pub svc: u8,
    pub payload: Vec<u8>,
}

impl DcFrame {
    pub fn decode(b: &[u8]) -> Option<DcFrame> {
        if b.len() < HEAD_LEN || b[0] != VERSION {
            return None;
        }
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        if b.len() < HEAD_LEN + len {
            return None;
        }
        Some(DcFrame { svc: b[1], payload: b[HEAD_LEN..HEAD_LEN + len].to_vec() })
    }

    pub fn encode(svc: u8, payload: &[u8]) -> Option<Vec<u8>> {
        if payload.len() > MAX_PAYLOAD {
            return None;
        }
        let mut out = Vec::with_capacity(HEAD_LEN + payload.len());
        out.push(VERSION);
        out.push(svc);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(payload);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_and_truncation() {
        let b = DcFrame::encode(SVC_FLOOR, &[1, 2, 3]).expect("enc");
        assert_eq!(b, vec![1, 1, 0, 3, 1, 2, 3]);
        assert_eq!(DcFrame::decode(&b), Some(DcFrame { svc: 1, payload: vec![1, 2, 3] }));
        assert_eq!(DcFrame::decode(&b[..5]), None);
        assert!(DcFrame::encode(SVC_FLOOR, &vec![0; MAX_PAYLOAD + 1]).is_none());
    }
}
