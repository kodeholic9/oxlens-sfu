// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§3-1 · 연§3-2 · model: claude-opus-5

//! WS 프레임 — 8B 헤더 + JSON body.
//!
//! 헤더는 `ver(1) | flags(1) | op(u16 BE) | pid(u32 BE)` 이고 body 는 그 뒤 전량이다.
//! 프레임은 두 갈래뿐이다 — 요청·통지(`00`) 하나에 응답(`01`) 또는 실패(`10`) 하나가 돌아온다.

use crate::code::Code;
use crate::op::Op;

/// 헤더 길이. body 길이 = 프레임 길이 − 이 값.
pub const HEADER_LEN: usize = 8;

/// 우리가 내는 유일한 버전. 다른 값을 받으면 끊는다.
pub const VER: u8 = 0x01;

/// 프레임 상한. 넘으면 끊는다.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// `flags` bit0-1 — 프레임의 갈래.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `00` 요청·통지 — 보내는 쪽이 먼저 말한다.
    Request,
    /// `01` 응답(성공). body 가 비면 그것이 ACK 이다.
    Ok,
    /// `10` 응답(실패). body 는 `Failure`.
    Fail,
}

impl Kind {
    const MASK: u8 = 0b11;

    fn from_flags(flags: u8) -> Result<Self, DecodeError> {
        match flags & Self::MASK {
            0b00 => Ok(Kind::Request),
            0b01 => Ok(Kind::Ok),
            0b10 => Ok(Kind::Fail),
            // ★`11` 은 예약이고 받으면 끊는다 — 조용히 받아 주면 나중에 뜻을 주는 순간 갈린다.
            _ => Err(DecodeError::ReservedKind),
        }
    }

    fn bits(self) -> u8 {
        match self {
            Kind::Request => 0b00,
            Kind::Ok => 0b01,
            Kind::Fail => 0b10,
        }
    }
}

/// 헤더 하나. `flags` 의 bit2-7 은 ★원본을 보존한다 — 나중에 늘려도 안 깨지게.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    /// bit2-7 예약 비트 원본(0b1111_1100 자리).
    pub reserved: u8,
    pub op: Op,
    pub pid: u32,
}

impl Header {
    pub fn new(kind: Kind, op: Op, pid: u32) -> Self {
        Self { kind, reserved: 0, op, pid }
    }

    fn flags(self) -> u8 {
        self.kind.bits() | (self.reserved & !Kind::MASK)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// 8B 헤더도 안 된다.
    TooShort,
    /// 상한 초과.
    TooLong,
    /// `ver` 가 우리 것이 아니다.
    Version(u8),
    /// `flags` 가 `11`.
    ReservedKind,
    /// 카탈로그에 없는 `op`. ★**`op`·`pid` 는 멀쩡하므로 응답으로 답할 수 있다** — 끊지 않는다.
    UnknownOp { op: u16, pid: u32 },
}

impl DecodeError {
    /// 이 실패로 끊을 때 `LEAVE` 가 나를 사유(연§3-1 끊는 조건).
    ///
    /// ★응답이 아니라 `LEAVE` 인 이유는 하나다 — `op`·`pid` 를 못 믿으면 응답 프레임을 지을 수가 없다.
    pub fn leave_reason(self) -> Code {
        match self {
            // ★모르는 op 은 여기 오지 않는다 — 부르는 쪽이 `1001` **응답**으로 답한다.
            DecodeError::UnknownOp { .. } => Code::UnknownOp,
            _ => Code::ProtocolError,
        }
    }
}

/// 헤더를 읽는다. body 는 빌려 준 버퍼의 뒤쪽을 그대로 가리킨다 — ★복사하지 않는다.
pub fn decode(buf: &[u8]) -> Result<(Header, &[u8]), DecodeError> {
    if buf.len() > MAX_FRAME_BYTES {
        return Err(DecodeError::TooLong);
    }
    if buf.len() < HEADER_LEN {
        return Err(DecodeError::TooShort);
    }
    let ver = buf[0];
    if ver != VER {
        return Err(DecodeError::Version(ver));
    }
    let flags = buf[1];
    let kind = Kind::from_flags(flags)?;
    let raw_op = u16::from_be_bytes([buf[2], buf[3]]);
    let pid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let op = Op::from_u16(raw_op).ok_or(DecodeError::UnknownOp { op: raw_op, pid })?;
    let header = Header { kind, reserved: flags & !Kind::MASK, op, pid };
    Ok((header, &buf[HEADER_LEN..]))
}

/// 헤더를 그릇 앞에 쓴다. 그릇은 부르는 쪽이 재사용한다 — ★프레임마다 새로 잡지 않는다.
pub fn encode_header(out: &mut Vec<u8>, header: Header) {
    out.clear();
    out.reserve(HEADER_LEN);
    out.push(VER);
    out.push(header.flags());
    out.extend_from_slice(&header.op.as_u16().to_be_bytes());
    out.extend_from_slice(&header.pid.to_be_bytes());
}

/// 헤더 + body 를 한 그릇에 잇는다. ★빈 body 는 0바이트다(헤더 8B 만).
pub fn encode(out: &mut Vec<u8>, header: Header, body: &[u8]) {
    encode_header(out, header);
    out.extend_from_slice(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 헤더_왕복() {
        let mut buf = Vec::new();
        encode(&mut buf, Header::new(Kind::Request, Op::Bind, 7), b"{}");
        let (h, body) = decode(&buf).expect("decode");
        assert_eq!(h.kind, Kind::Request);
        assert_eq!(h.op, Op::Bind);
        assert_eq!(h.pid, 7);
        assert_eq!(body, b"{}");
    }

    #[test]
    fn 빈_body_는_0바이트() {
        let mut buf = Vec::new();
        encode(&mut buf, Header::new(Kind::Ok, Op::Heartbeat, 1), b"");
        assert_eq!(buf.len(), HEADER_LEN);
        let (_, body) = decode(&buf).expect("decode");
        // ★ACK = body 가 빈 `01` 응답. `{}` 도 같게 받는다(그쪽은 소비자 몫).
        assert!(body.is_empty());
    }

    #[test]
    fn 예약_비트는_보존된다() {
        let mut buf = Vec::new();
        encode(&mut buf, Header::new(Kind::Request, Op::Bind, 1), b"");
        buf[1] |= 0b1111_1100;
        let (h, _) = decode(&buf).expect("decode");
        assert_eq!(h.reserved, 0b1111_1100, "★원본을 보존해야 나중에 늘려도 안 깨진다");
        let mut again = Vec::new();
        encode(&mut again, h, b"");
        assert_eq!(again[1], buf[1]);
    }

    #[test]
    fn 갈래_11_은_끊는다() {
        let mut buf = Vec::new();
        encode(&mut buf, Header::new(Kind::Request, Op::Bind, 1), b"");
        buf[1] = (buf[1] & !0b11) | 0b11;
        assert_eq!(decode(&buf), Err(DecodeError::ReservedKind));
    }

    #[test]
    fn 버전이_다르면_끊는다() {
        let mut buf = Vec::new();
        encode(&mut buf, Header::new(Kind::Request, Op::Bind, 1), b"");
        buf[0] = 0x02;
        assert_eq!(decode(&buf), Err(DecodeError::Version(0x02)));
        assert_eq!(DecodeError::Version(0x02).leave_reason(), Code::ProtocolError);
    }

    #[test]
    fn 모르는_op_은_끊는_사유가_다르다() {
        let mut buf = vec![VER, 0, 0xEE, 0xEE, 0, 0, 0, 1];
        buf.extend_from_slice(b"{}");
        // ★`op`·`pid` 를 그대로 돌려준다 — 그래야 부르는 쪽이 `1001` **응답**을 지을 수 있다.
        assert_eq!(decode(&buf), Err(DecodeError::UnknownOp { op: 0xEEEE, pid: 1 }));
    }

    #[test]
    fn 상한을_넘으면_끊는다() {
        let buf = vec![0u8; MAX_FRAME_BYTES + 1];
        assert_eq!(decode(&buf), Err(DecodeError::TooLong));
    }
}
