// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:TRN.frame.WS · 연:RULE-TRN-TWO-FRAMES · 연:RULE-TRN-CLOSE · 연:RULE-TRN-UNKNOWN-JSON-FIELD · model: claude-opus-5-5
//! WS 프레임 — 8바이트 머리(ver · flags · op · pid) + JSON body. 빈 body 는 0바이트이고 `{}` 와 같게 받는다.

pub const VER: u8 = 1;
pub const HEADER_LEN: usize = 8;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
pub const KIND_MASK: u8 = 0b11;

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
pub struct Header {
    pub flags: u8,
    pub op: u16,
    pub pid: u32,
}

impl Header {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<'a> {
    pub header: Header,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    TooLarge,
    BadVersion,
    ReservedKind,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FrameError::TooShort => "frame shorter than 8 bytes",
            FrameError::TooLarge => "frame larger than 1048576 bytes",
            FrameError::BadVersion => "frame ver is not 1",
            FrameError::ReservedKind => "flags kind bits 11",
        };
        f.write_str(s)
    }
}

impl std::error::Error for FrameError {}

pub fn decode(buf: &[u8]) -> Result<Frame<'_>, FrameError> {
    if buf.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge);
    }
    if buf.len() < HEADER_LEN {
        return Err(FrameError::TooShort);
    }
    if buf[0] != VER {
        return Err(FrameError::BadVersion);
    }
    let flags = buf[1];
    if flags & KIND_MASK == KIND_MASK {
        return Err(FrameError::ReservedKind);
    }
    let op = u16::from_be_bytes([buf[2], buf[3]]);
    let pid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    Ok(Frame { header: Header { flags, op, pid }, body: &buf[HEADER_LEN..] })
}

pub fn encode(kind: FrameKind, op: u16, pid: u32, body: &[u8]) -> Vec<u8> {
    encode_flags(kind.bits(), op, pid, body)
}

pub fn encode_flags(flags: u8, op: u16, pid: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.push(VER);
    out.push(flags);
    out.extend_from_slice(&op.to_be_bytes());
    out.extend_from_slice(&pid.to_be_bytes());
    out.extend_from_slice(body);
    out
}

pub fn reply_flags(req: &Header, kind: FrameKind) -> u8 {
    req.reserved() | kind.bits()
}

pub fn body_value(body: &[u8]) -> Result<serde_json::Value, serde_json::Error> {
    if body.is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    serde_json::from_slice(body)
}

pub fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, serde_json::Error> {
    if body.is_empty() {
        return serde_json::from_slice(b"{}");
    }
    serde_json::from_slice(body)
}

pub fn empty_body() -> Vec<u8> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_reserved_bits_kept() {
        let b = encode_flags(0b1000_0001, 0x0101, 7, b"{}");
        let f = decode(&b).unwrap();
        assert_eq!(f.header.kind(), FrameKind::Ok);
        assert_eq!(f.header.reserved(), 0b1000_0000);
        assert_eq!(f.header.op, 0x0101);
        assert_eq!(f.header.pid, 7);
        assert_eq!(f.body, b"{}");
        assert_eq!(reply_flags(&f.header, FrameKind::Fail), 0b1000_0010);
    }

    #[test]
    fn rejects() {
        assert_eq!(decode(&[1, 0, 0]), Err(FrameError::TooShort));
        assert_eq!(decode(&[2, 0, 0, 1, 0, 0, 0, 1]), Err(FrameError::BadVersion));
        assert_eq!(decode(&[1, 3, 0, 1, 0, 0, 0, 1]), Err(FrameError::ReservedKind));
        let big = vec![1u8; MAX_FRAME_BYTES + 1];
        assert_eq!(decode(&big), Err(FrameError::TooLarge));
    }

    #[test]
    fn empty_body_is_object() {
        assert_eq!(body_value(b"").unwrap(), serde_json::json!({}));
    }
}
