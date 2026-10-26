// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:RULE-TRN-TWO-FRAMES · 연:RULE-TRN-ONE-REPLY · 연:RULE-TRN-CLOSE · 연:RULE-TRN-UNKNOWN-JSON-FIELD · 연:REJ-TRN-MISREADINGS · model: claude-opus-5-5
//! WS 프레임 — 8B 머리(ver · flags · op · pid) + JSON 몸통. 두 갈래(요청 · 통지 / 응답)만 있다.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::code::Failure;

pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 8;
pub const MAX_FRAME: usize = 1_048_576;

const KIND_MASK: u8 = 0b0000_0011;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Request,
    Ok,
    Fail,
}

impl FrameKind {
    pub fn bits(self) -> u8 {
        match self {
            FrameKind::Request => 0b00,
            FrameKind::Ok => 0b01,
            FrameKind::Fail => 0b10,
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
    pub flags: u8,
    pub op: u16,
    pub pid: u32,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn new(kind: FrameKind, op: u16, pid: u32, body: Vec<u8>) -> Frame {
        Frame { flags: kind.bits(), op, pid, body }
    }

    pub fn request<T: Serialize>(op: u16, pid: u32, body: &T) -> Frame {
        Frame::new(FrameKind::Request, op, pid, to_body(body))
    }

    pub fn ok<T: Serialize>(&self, body: &T) -> Frame {
        Frame { flags: (self.flags & !KIND_MASK) | FrameKind::Ok.bits(), op: self.op, pid: self.pid, body: to_body(body) }
    }

    pub fn ack(&self) -> Frame {
        Frame { flags: (self.flags & !KIND_MASK) | FrameKind::Ok.bits(), op: self.op, pid: self.pid, body: Vec::new() }
    }

    pub fn fail(&self, failure: &Failure) -> Frame {
        Frame { flags: (self.flags & !KIND_MASK) | FrameKind::Fail.bits(), op: self.op, pid: self.pid, body: to_body(failure) }
    }

    pub fn kind(&self) -> FrameKind {
        match self.flags & KIND_MASK {
            0b00 => FrameKind::Request,
            0b01 => FrameKind::Ok,
            _ => FrameKind::Fail,
        }
    }

    pub fn reserved(&self) -> u8 {
        self.flags & !KIND_MASK
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.body.len());
        out.push(VERSION);
        out.push(self.flags);
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&self.pid.to_be_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        if bytes.len() > MAX_FRAME {
            return Err(FrameError::TooLarge);
        }
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Short);
        }
        if bytes[0] != VERSION {
            return Err(FrameError::BadVersion);
        }
        let flags = bytes[1];
        if flags & KIND_MASK == KIND_MASK {
            return Err(FrameError::ReservedKind);
        }
        let op = u16::from_be_bytes([bytes[2], bytes[3]]);
        let pid = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        Ok(Frame { flags, op, pid, body: bytes[HEADER_LEN..].to_vec() })
    }

    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        parse_body(&self.body)
    }

    pub fn json(&self) -> Result<serde_json::Value, serde_json::Error> {
        parse_body(&self.body)
    }
}

pub fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, serde_json::Error> {
    if body.is_empty() { serde_json::from_slice(b"{}") } else { serde_json::from_slice(body) }
}

pub fn to_body<T: Serialize>(body: &T) -> Vec<u8> {
    match serde_json::to_vec(body) {
        Ok(v) if v == b"{}" => Vec::new(),
        Ok(v) => v,
        Err(_) => Vec::new(),
    }
}

pub fn next_pid(pid: u32) -> u32 {
    pid.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Code;

    #[test]
    fn round_trip_keeps_reserved_bits() {
        let f = Frame { flags: 0b1010_0000, op: 0x0201, pid: 7, body: b"{\"room_id\":\"r\"}".to_vec() };
        let g = Frame::decode(&f.encode()).expect("decode");
        assert_eq!(g, f);
        assert_eq!(g.kind(), FrameKind::Request);
        assert_eq!(g.ok(&serde_json::json!({})).reserved(), 0b1010_0000);
    }

    #[test]
    fn empty_body_is_zero_bytes_and_braces_read_the_same() {
        let f = Frame::request(0x0103, 1, &serde_json::json!({}));
        assert_eq!(f.encode().len(), HEADER_LEN);
        let a: serde_json::Value = parse_body(b"").expect("empty");
        let b: serde_json::Value = parse_body(b"{}").expect("braces");
        assert_eq!(a, b);
    }

    #[test]
    fn broken_frames_are_refused() {
        assert_eq!(Frame::decode(&[1, 0, 0]), Err(FrameError::Short));
        assert_eq!(Frame::decode(&[2, 0, 0, 1, 0, 0, 0, 1]), Err(FrameError::BadVersion));
        assert_eq!(Frame::decode(&[1, 3, 0, 1, 0, 0, 0, 1]), Err(FrameError::ReservedKind));
        assert_eq!(Frame::decode(&vec![1u8; MAX_FRAME + 1]), Err(FrameError::TooLarge));
    }

    #[test]
    fn failure_reply_mirrors_op_and_pid() {
        let req = Frame::request(0x0201, 42, &serde_json::json!({"room_id": "r"}));
        let f = req.fail(&Failure::of(Code::ROOM_NOT_FOUND));
        assert_eq!((f.op, f.pid, f.kind()), (0x0201, 42, FrameKind::Fail));
        let v = f.json().expect("json");
        assert_eq!(v["code"], 3001);
        assert_eq!(v["permanent"], false);
    }

    #[test]
    fn pid_wraps_to_zero() {
        assert_eq!(next_pid(u32::MAX), 0);
    }
}
