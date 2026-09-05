// author: kodeholic (powered by Claude)
//! WS 프레임 — 연§3-1. `ver(1)=0x01 · flags(1) · op(u16 BE) · pid(u32 BE) · body(JSON)`.
//! 프레임은 두 갈래뿐이다: `00` 요청·통지, `01`/`10` 응답(성공/실패). `11` 은 예약 — 받으면 끊는다.

use std::fmt;

/// 연§3-1 `ver` — 다르면 끊는다.
pub const VER: u8 = 0x01;
pub const HEADER_LEN: usize = 8;
/// 연§3-1 끊는 조건 — 프레임 길이 상한.
pub const MAX_FRAME_LEN: usize = 1_048_576;
const KIND_MASK: u8 = 0b0000_0011;

/// `flags` bit0-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `00` — 요청 또는 통지. 보내는 쪽이 `pid` 를 매긴다.
    Msg,
    /// `01` — 성공 응답. `op`·`pid` 를 받은 그대로 되돌린다. 빈 body 면 ACK 이다.
    Ok,
    /// `10` — 실패 응답. body 는 `Failure`(연§4-5).
    Fail,
}

impl Kind {
    pub fn bits(self) -> u8 {
        match self {
            Kind::Msg => 0b00,
            Kind::Ok => 0b01,
            Kind::Fail => 0b10,
        }
    }
    pub fn from_flags(flags: u8) -> Result<Self, FrameError> {
        match flags & KIND_MASK {
            0b00 => Ok(Kind::Msg),
            0b01 => Ok(Kind::Ok),
            0b10 => Ok(Kind::Fail),
            _ => Err(FrameError::ReservedKind),
        }
    }
    pub fn is_response(self) -> bool {
        !matches!(self, Kind::Msg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    /// 연§3-1 — bit2-7 예약. 파서가 원본을 보존한다.
    pub reserved: u8,
    pub op: u16,
    pub pid: u32,
}

impl Header {
    pub fn msg(op: u16, pid: u32) -> Self {
        Self { kind: Kind::Msg, reserved: 0, op, pid }
    }
    /// 응답 — 받은 헤더의 `op`·`pid` 를 그대로 되돌린다.
    pub fn reply(req: &Header, kind: Kind) -> Self {
        Self { kind, reserved: 0, op: req.op, pid: req.pid }
    }
    pub fn flags(&self) -> u8 {
        (self.reserved & !KIND_MASK) | self.kind.bits()
    }
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0] = VER;
        b[1] = self.flags();
        b[2..4].copy_from_slice(&self.op.to_be_bytes());
        b[4..8].copy_from_slice(&self.pid.to_be_bytes());
        b
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::TooShort(bytes.len()));
        }
        if bytes[0] != VER {
            return Err(FrameError::BadVersion(bytes[0]));
        }
        let kind = Kind::from_flags(bytes[1])?;
        Ok(Self {
            kind,
            reserved: bytes[1] & !KIND_MASK,
            op: u16::from_be_bytes([bytes[2], bytes[3]]),
            pid: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }
}

/// 연§3-1 "끊는 조건" — 응답을 지을 수 없는 경우. Close 사유는 `close::CloseCode::ProtocolError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooShort(usize),
    BadVersion(u8),
    ReservedKind,
    TooLarge(usize),
    BodyNotJson,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooShort(n) => write!(f, "frame shorter than header ({n}B)"),
            FrameError::BadVersion(v) => write!(f, "ver {v:#04x} != 0x01"),
            FrameError::ReservedKind => f.write_str("flags 11 reserved"),
            FrameError::TooLarge(n) => write!(f, "frame {n}B > {MAX_FRAME_LEN}B"),
            FrameError::BodyNotJson => f.write_str("body is not JSON"),
        }
    }
}

impl std::error::Error for FrameError {}

/// 헤더 + body. ★빈 body 는 0바이트로 나간다(연§3-1).
pub fn encode(h: &Header, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(&h.encode());
    out.extend_from_slice(body);
    out
}

/// JSON 값 → 프레임. `Null` 이나 빈 객체는 0바이트 body.
pub fn encode_json(h: &Header, body: &serde_json::Value) -> Vec<u8> {
    let empty = body.is_null() || body.as_object().is_some_and(|o| o.is_empty());
    if empty {
        encode(h, &[])
    } else {
        encode(h, body.to_string().as_bytes())
    }
}

pub fn decode(frame: &[u8]) -> Result<(Header, &[u8]), FrameError> {
    if frame.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(frame.len()));
    }
    let h = Header::decode(frame)?;
    Ok((h, &frame[HEADER_LEN..]))
}

/// body → JSON. 0바이트와 `{}` 는 같은 빈 객체다. JSON 이 아니면 끊는 조건(연§3-1).
pub fn body_json(body: &[u8]) -> Result<serde_json::Value, FrameError> {
    if body.is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    serde_json::from_slice(body).map_err(|_| FrameError::BodyNotJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_and_reserved_bits_preserved() {
        let h = Header { kind: Kind::Ok, reserved: 0b1010_0100, op: 0x0201, pid: 7 };
        let f = encode(&h, b"{}");
        let (d, body) = decode(&f).unwrap();
        assert_eq!(d, h);
        assert_eq!(d.flags(), 0b1010_0101);
        assert_eq!(body_json(body).unwrap(), json!({}));
    }

    #[test]
    fn empty_body_is_zero_bytes_and_equals_braces() {
        let h = Header::msg(0x0103, 1);
        assert_eq!(encode_json(&h, &json!({})).len(), HEADER_LEN);
        assert_eq!(encode_json(&h, &serde_json::Value::Null).len(), HEADER_LEN);
        assert_eq!(body_json(b"").unwrap(), body_json(b"{}").unwrap());
    }

    #[test]
    fn disconnect_conditions() {
        assert_eq!(Header::decode(&[1, 0, 0]).unwrap_err(), FrameError::TooShort(3));
        assert_eq!(Header::decode(&[2, 0, 0, 0, 0, 0, 0, 0]).unwrap_err(), FrameError::BadVersion(2));
        assert_eq!(Header::decode(&[1, 0b11, 0, 0, 0, 0, 0, 0]).unwrap_err(), FrameError::ReservedKind);
        assert_eq!(body_json(b"nope").unwrap_err(), FrameError::BodyNotJson);
        let big = vec![0u8; MAX_FRAME_LEN + 1];
        assert!(matches!(decode(&big), Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn reply_echoes_op_and_pid() {
        let req = Header { kind: Kind::Msg, reserved: 0, op: 0x0101, pid: 0xFFFF_FFFF };
        let r = Header::reply(&req, Kind::Fail);
        assert_eq!((r.op, r.pid, r.kind), (0x0101, 0xFFFF_FFFF, Kind::Fail));
    }
}
