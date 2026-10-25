// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:TRN.frame.WS 연:TRN.frame.DC 연:RULE-TRN-TWO-FRAMES 연:RULE-TRN-ONE-REPLY 연:RULE-TRN-CLOSE 연:REJ-TRN-MISREADINGS · model: claude-opus-5-5
//! WS 프레임(8B 머리 + JSON) · DC 프레임(4B 머리 + payload) 부호화 · 복호.

use crate::op::Op;

pub const WS_VERSION: u8 = 0x01;
pub const WS_HEADER_LEN: usize = 8;
pub const WS_MAX_FRAME: usize = 1_048_576;
pub const DC_VERSION: u8 = 0x01;
pub const DC_HEADER_LEN: usize = 4;
pub const DC_MAX_PAYLOAD: usize = 65_532;
pub const DC_SVC_FLOOR: u8 = 0x01;
pub const DC_CHANNEL_LABEL: &str = "unreliable";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Request,
    Ok,
    Fail,
}

impl Kind {
    fn bits(self) -> u8 {
        match self {
            Kind::Request => 0b00,
            Kind::Ok => 0b01,
            Kind::Fail => 0b10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub reserved: u8,
    pub op: u16,
    pub pid: u32,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooLarge,
    TooShort,
    BadVersion,
    ReservedKind,
}

impl Frame {
    pub fn request(op: Op, pid: u32, body: Vec<u8>) -> Frame {
        Frame {
            kind: Kind::Request,
            reserved: 0,
            op: op.as_u16(),
            pid,
            body,
        }
    }

    pub fn reply_ok(op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame {
            kind: Kind::Ok,
            reserved: 0,
            op,
            pid,
            body,
        }
    }

    pub fn reply_fail(op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame {
            kind: Kind::Fail,
            reserved: 0,
            op,
            pid,
            body,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(WS_HEADER_LEN + self.body.len());
        out.push(WS_VERSION);
        out.push((self.reserved << 2) | self.kind.bits());
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&self.pid.to_be_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Frame, FrameError> {
        if b.len() > WS_MAX_FRAME {
            return Err(FrameError::TooLarge);
        }
        if b.len() < WS_HEADER_LEN {
            return Err(FrameError::TooShort);
        }
        if b[0] != WS_VERSION {
            return Err(FrameError::BadVersion);
        }
        let kind = match b[1] & 0b11 {
            0b00 => Kind::Request,
            0b01 => Kind::Ok,
            0b10 => Kind::Fail,
            _ => return Err(FrameError::ReservedKind),
        };
        Ok(Frame {
            kind,
            reserved: b[1] >> 2,
            op: u16::from_be_bytes([b[2], b[3]]),
            pid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            body: b[WS_HEADER_LEN..].to_vec(),
        })
    }

    pub fn body_is_empty(&self) -> bool {
        self.body.is_empty()
            || self
                .body
                .iter()
                .filter(|c| !c.is_ascii_whitespace())
                .eq(b"{}".iter())
    }
}

pub fn dc_encode(svc: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > DC_MAX_PAYLOAD {
        return None;
    }
    let mut out = Vec::with_capacity(DC_HEADER_LEN + payload.len());
    out.push(DC_VERSION);
    out.push(svc);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

pub fn dc_decode(b: &[u8]) -> Option<(u8, &[u8])> {
    if b.len() < DC_HEADER_LEN || b[0] != DC_VERSION {
        return None;
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if b.len() < DC_HEADER_LEN + len {
        return None;
    }
    Some((b[1], &b[DC_HEADER_LEN..DC_HEADER_LEN + len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_round_trip_keeps_reserved_bits() {
        let f = Frame {
            kind: Kind::Fail,
            reserved: 0b101,
            op: 0x0201,
            pid: 7,
            body: b"{\"a\":1}".to_vec(),
        };
        let g = Frame::decode(&f.encode()).expect("decode");
        assert_eq!(f, g);
    }

    #[test]
    fn ws_rejects_per_rule_trn_close() {
        assert_eq!(Frame::decode(&[1, 0, 0]), Err(FrameError::TooShort));
        assert_eq!(
            Frame::decode(&[2, 0, 0, 1, 0, 0, 0, 1]),
            Err(FrameError::BadVersion)
        );
        assert_eq!(
            Frame::decode(&[1, 3, 0, 1, 0, 0, 0, 1]),
            Err(FrameError::ReservedKind)
        );
        assert_eq!(
            Frame::decode(&vec![1u8; WS_MAX_FRAME + 1]),
            Err(FrameError::TooLarge)
        );
    }

    #[test]
    fn empty_body_forms() {
        assert!(Frame::reply_ok(1, 1, vec![]).body_is_empty());
        assert!(Frame::reply_ok(1, 1, b"{}".to_vec()).body_is_empty());
        assert!(!Frame::reply_ok(1, 1, b"{\"x\":1}".to_vec()).body_is_empty());
    }

    #[test]
    fn dc_truncated_is_dropped() {
        let e = dc_encode(DC_SVC_FLOOR, &[1, 2, 3]).expect("enc");
        assert_eq!(dc_decode(&e), Some((DC_SVC_FLOOR, &[1u8, 2, 3][..])));
        assert_eq!(dc_decode(&e[..5]), None);
        assert!(dc_encode(1, &vec![0; DC_MAX_PAYLOAD + 1]).is_none());
    }
}
