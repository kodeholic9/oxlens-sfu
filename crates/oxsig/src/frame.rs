// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 연:TRN.frame.WS 연:RULE-TRN-TWO-FRAMES 연:RULE-TRN-CLOSE · model: claude-opus-5-5
//! WS 프레임 — 8B 헤더 + JSON body.

pub const VER: u8 = 0x01;
pub const HEADER_LEN: usize = 8;
pub const MAX_FRAME: usize = 1_048_576;

const KIND_MASK: u8 = 0b0000_0011;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooLarge,
    Short,
    BadVersion,
    ReservedKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub reserved: u8,
    pub op: u16,
    pub pid: u32,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn request(op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame { kind: Kind::Request, reserved: 0, op, pid, body }
    }

    pub fn ok(op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame { kind: Kind::Ok, reserved: 0, op, pid, body }
    }

    pub fn fail(op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame { kind: Kind::Fail, reserved: 0, op, pid, body }
    }

    pub fn decode(b: &[u8]) -> Result<Frame, FrameError> {
        if b.len() > MAX_FRAME {
            return Err(FrameError::TooLarge);
        }
        if b.len() < HEADER_LEN {
            return Err(FrameError::Short);
        }
        if b[0] != VER {
            return Err(FrameError::BadVersion);
        }
        let flags = b[1];
        let kind = match flags & KIND_MASK {
            0b00 => Kind::Request,
            0b01 => Kind::Ok,
            0b10 => Kind::Fail,
            _ => return Err(FrameError::ReservedKind),
        };
        Ok(Frame {
            kind,
            reserved: flags & !KIND_MASK,
            op: u16::from_be_bytes([b[2], b[3]]),
            pid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            body: b[HEADER_LEN..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.body.len());
        out.push(VER);
        out.push((self.reserved & !KIND_MASK) | self.kind.bits());
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&self.pid.to_be_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn body_json(&self) -> &[u8] {
        if self.body.is_empty() { b"{}" } else { &self.body }
    }

    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(self.body_json())
    }
}

#[derive(Debug, Default, Clone)]
pub struct PidCounter(u32);

impl PidCounter {
    pub fn take(&mut self) -> u32 {
        let v = self.0;
        self.0 = self.0.wrapping_add(1);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_reserved_bits() {
        let f = Frame { kind: Kind::Fail, reserved: 0b1010_0000, op: 0x0201, pid: 7, body: b"{\"a\":1}".to_vec() };
        let d = Frame::decode(&f.encode()).expect("decode");
        assert_eq!(d, f);
    }

    #[test]
    fn empty_body_reads_as_object() {
        let f = Frame::decode(&Frame::ok(0x0103, 1, vec![]).encode()).expect("decode");
        assert_eq!(f.body_json(), b"{}");
    }

    #[test]
    fn rejects_bad_frames() {
        assert_eq!(Frame::decode(&[1, 0, 0]), Err(FrameError::Short));
        assert_eq!(Frame::decode(&[2, 0, 0, 1, 0, 0, 0, 0]), Err(FrameError::BadVersion));
        assert_eq!(Frame::decode(&[1, 3, 0, 1, 0, 0, 0, 0]), Err(FrameError::ReservedKind));
        assert_eq!(Frame::decode(&vec![1u8; MAX_FRAME + 1]), Err(FrameError::TooLarge));
    }

    #[test]
    fn pid_wraps() {
        let mut p = PidCounter(u32::MAX);
        assert_eq!(p.take(), u32::MAX);
        assert_eq!(p.take(), 0);
    }
}
