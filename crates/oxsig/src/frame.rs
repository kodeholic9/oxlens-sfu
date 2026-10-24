// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:TRN.frame.WS · 연:RULE-TRN-TWO-FRAMES · 연:RULE-TRN-ONE-REPLY · 연:RULE-TRN-CLOSE · 연:REJ-TRN-MISREADINGS · model: claude-opus-5-5
//! WS 프레임 — 8바이트 머리(ver · flags · op · pid) + JSON body.

pub const VERSION: u8 = 0x01;
pub const HEAD_LEN: usize = 8;
pub const MAX_FRAME: usize = 1_048_576;
const KIND_MASK: u8 = 0b11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Ok,
    Fail,
}

impl Kind {
    pub fn bits(self) -> u8 {
        match self {
            Kind::Request => 0b00,
            Kind::Ok => 0b01,
            Kind::Fail => 0b10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooLarge,
    TooShort,
    BadVersion,
    ReservedKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub flags: u8,
    pub op: u16,
    pub pid: u32,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn new(kind: Kind, op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame { flags: kind.bits(), op, pid, body }
    }

    pub fn kind(&self) -> Kind {
        match self.flags & KIND_MASK {
            0b00 => Kind::Request,
            0b01 => Kind::Ok,
            _ => Kind::Fail,
        }
    }

    pub fn reply(&self, kind: Kind, body: Vec<u8>) -> Frame {
        Frame { flags: (self.flags & !KIND_MASK) | kind.bits(), op: self.op, pid: self.pid, body }
    }

    pub fn body_or_empty(&self) -> &[u8] {
        if self.body.is_empty() { b"{}" } else { &self.body }
    }

    pub fn decode(b: &[u8]) -> Result<Frame, FrameError> {
        if b.len() > MAX_FRAME {
            return Err(FrameError::TooLarge);
        }
        if b.len() < HEAD_LEN {
            return Err(FrameError::TooShort);
        }
        if b[0] != VERSION {
            return Err(FrameError::BadVersion);
        }
        if b[1] & KIND_MASK == 0b11 {
            return Err(FrameError::ReservedKind);
        }
        Ok(Frame {
            flags: b[1],
            op: u16::from_be_bytes([b[2], b[3]]),
            pid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            body: b[HEAD_LEN..].to_vec(),
        })
    }

    pub fn peek_op_pid(b: &[u8]) -> Option<(u16, u32)> {
        if b.len() < HEAD_LEN || b[0] != VERSION || b[1] & KIND_MASK == 0b11 {
            return None;
        }
        Some((u16::from_be_bytes([b[2], b[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]])))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEAD_LEN + self.body.len());
        out.push(VERSION);
        out.push(self.flags);
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&self.pid.to_be_bytes());
        out.extend_from_slice(&self.body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_reserved_bits() {
        let f = Frame { flags: 0b1010_0001, op: 0x0101, pid: 7, body: b"{}".to_vec() };
        let g = Frame::decode(&f.encode()).expect("decode");
        assert_eq!(g, f);
        assert_eq!(g.kind(), Kind::Ok);
        assert_eq!(g.reply(Kind::Fail, vec![]).flags, 0b1010_0010);
    }

    #[test]
    fn rejects_bad_frames() {
        assert_eq!(Frame::decode(&[1, 0, 0]), Err(FrameError::TooShort));
        assert_eq!(Frame::decode(&[2, 0, 0, 1, 0, 0, 0, 1]), Err(FrameError::BadVersion));
        assert_eq!(Frame::decode(&[1, 3, 0, 1, 0, 0, 0, 1]), Err(FrameError::ReservedKind));
        assert_eq!(Frame::decode(&vec![1u8; MAX_FRAME + 1]), Err(FrameError::TooLarge));
        assert_eq!(Frame::decode(&[1, 0, 1, 3, 0, 0, 0, 9]).map(|f| f.body_or_empty().to_vec()), Ok(b"{}".to_vec()));
    }
}
