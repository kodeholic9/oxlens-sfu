// author: kodeholic (powered by Claude)
//! 발언권(MBCP) 메시지 — 연§11-1~§11-3. 3GPP TS 24.380 Table 8.2.2.1-1 의 subtype 값 그대로,
//! 다방 확장은 원문이 안 쓰는 TLV id(`0x1A`~)에만 얹는다.
//! 헤더 byte0: `V(2)=00 · R(1)=0 · A(1) · Type(4)`, byte1: TLV 개수, 이어 `[id(1)][len(1)][val]`.

use std::fmt;

const VER_MASK: u8 = 0xC0;
const RESERVED_MASK: u8 = 0x20;
const ACK_MASK: u8 = 0x10;
const TYPE_MASK: u8 = 0x0F;
pub const MAX_TLV_VALUE_LEN: usize = 255;

/// 연§11-2 — 원문 subtype 하위 4비트. 미채택(7·11·14·15)은 비워 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MsgType {
    Request = 0,
    Granted = 1,
    Taken = 2,
    Deny = 3,
    Release = 4,
    Idle = 5,
    Revoke = 6,
    QueuePosRequest = 8,
    QueueInfo = 9,
    Ack = 10,
}

impl MsgType {
    pub const ALL: [MsgType; 10] = [
        MsgType::Request, MsgType::Granted, MsgType::Taken, MsgType::Deny, MsgType::Release, MsgType::Idle,
        MsgType::Revoke, MsgType::QueuePosRequest, MsgType::QueueInfo, MsgType::Ack,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<MsgType> {
        MsgType::ALL.iter().copied().find(|t| t.code() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            MsgType::Request => "FLOOR_REQUEST",
            MsgType::Granted => "FLOOR_GRANTED",
            MsgType::Taken => "FLOOR_TAKEN",
            MsgType::Deny => "FLOOR_DENY",
            MsgType::Release => "FLOOR_RELEASE",
            MsgType::Idle => "FLOOR_IDLE",
            MsgType::Revoke => "FLOOR_REVOKE",
            MsgType::QueuePosRequest => "FLOOR_QUEUE_POS_REQUEST",
            MsgType::QueueInfo => "FLOOR_QUEUE_INFO",
            MsgType::Ack => "FLOOR_ACK",
        }
    }

    /// 연§11-2 — 원문 subtype 이 `x????` 꼴인 것만 `A` 를 세울 수 있다(인코딩이지 정책이 아니다).
    pub fn ack_bit_allowed(self) -> bool {
        matches!(
            self,
            MsgType::Granted | MsgType::Taken | MsgType::Deny | MsgType::Release | MsgType::Idle | MsgType::QueueInfo
        )
    }

    /// 연§11-2 "우리" 열 — 서버가 `A` 를 세우는 것은 GRANTED·DENY 둘뿐.
    pub fn server_sets_ack(self) -> bool {
        matches!(self, MsgType::Granted | MsgType::Deny)
    }

    /// 연§11-3 `8` — IDLE·TAKEN 은 방마다 1 증가하는 일련번호가 필수.
    pub fn requires_seq(self) -> bool {
        matches!(self, MsgType::Idle | MsgType::Taken)
    }
}

/// 연§11-3 TLV id. `0`~`12` 는 원문 Table 8.2.3.1-2 그대로, `0x1A`~ 는 자체 확장.
pub mod field {
    pub const PRIORITY: u8 = 0;
    pub const DURATION: u8 = 1;
    pub const CAUSE: u8 = 2;
    pub const QUEUE_INFO: u8 = 3;
    pub const GRANTED_PARTY: u8 = 4;
    pub const QUEUE_SIZE: u8 = 7;
    pub const SEQ: u8 = 8;
    pub const ACK_TYPE: u8 = 12;
    pub const PREV_SPEAKER: u8 = 0x1A;
    pub const CAUSE_TEXT: u8 = 0x1B;
    /// 방 — 양방향 전부에 실린다.
    pub const ROOM: u8 = 0x1D;
    /// 번호는 재사용하지 않는다.
    pub const RETIRED: [u8; 2] = [0x1C, 0x1E];
    /// 자체 확장 범위 — 원문이 `025` 까지 쓰고, `192` 부터는 len 이 2옥텟이 된다.
    pub const EXT_MIN: u8 = 0x1A;
    pub const EXT_MAX: u8 = 0xBF;
}

/// 연§11-3 거절 사유(DENY `2`) — 원문 §8.2.6.2 + 자체 `100`.
pub mod reject {
    pub const ANOTHER_HAS_PERMISSION: u8 = 1;
    pub const INTERNAL_ERROR: u8 = 2;
    pub const ONLY_ONE_PARTICIPANT: u8 = 3;
    pub const RETRY_AFTER_NOT_EXPIRED: u8 = 4;
    pub const RECEIVE_ONLY: u8 = 5;
    pub const NO_RESOURCES: u8 = 6;
    pub const QUEUE_FULL: u8 = 7;
    pub const OTHER: u8 = 255;
    /// 다른 방에서 발언권을 쥔 채 요청했다.
    pub const HELD_ELSEWHERE: u8 = 100;
}

/// 연§11-3 회수 사유(REVOKE `2`) — 원문 §8.2.10.
pub mod revoke {
    pub const ONLY_ONE_CLIENT: u8 = 1;
    pub const BURST_TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
    pub const NO_RESOURCES: u8 = 6;
    pub const REVOKED_BY_ANOTHER: u8 = 7;
    pub const OTHER: u8 = 255;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tlv {
    pub id: u8,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub msg_type: MsgType,
    pub ack_req: bool,
    pub tlvs: Vec<Tlv>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MbcpError {
    /// 값은 255바이트 이하(연§11-1 — `room_id` 도 UTF-8 255B 상한).
    ValueTooLong { id: u8, len: usize },
    /// 원문에 없는 subtype 이 나간다(예: `REVOKE` + `A`).
    AckNotAllowed(MsgType),
    TooManyTlvs(usize),
}

impl fmt::Display for MbcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MbcpError::ValueTooLong { id, len } => write!(f, "tlv {id:#04x} value {len}B > 255B"),
            MbcpError::AckNotAllowed(t) => write!(f, "{} cannot carry A bit", t.name()),
            MbcpError::TooManyTlvs(n) => write!(f, "{n} tlvs > 255"),
        }
    }
}

impl std::error::Error for MbcpError {}

impl Msg {
    pub fn new(msg_type: MsgType) -> Self {
        Self { msg_type, ack_req: false, tlvs: Vec::new() }
    }
    pub fn ack(mut self, ack_req: bool) -> Self {
        self.ack_req = ack_req;
        self
    }
    pub fn tlv(mut self, id: u8, value: impl Into<Vec<u8>>) -> Self {
        self.tlvs.push(Tlv { id, value: value.into() });
        self
    }
    pub fn str(self, id: u8, s: &str) -> Self {
        self.tlv(id, s.as_bytes())
    }
    pub fn u8(self, id: u8, v: u8) -> Self {
        self.tlv(id, [v])
    }
    pub fn u16(self, id: u8, v: u16) -> Self {
        self.tlv(id, v.to_be_bytes())
    }
    pub fn room(self, room_id: &str) -> Self {
        self.str(field::ROOM, room_id)
    }

    /// 같은 id 가 여럿이면 첫 것 — 순서는 계약이 아니다.
    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|t| t.id == id).map(|t| t.value.as_slice())
    }
    pub fn get_u8(&self, id: u8) -> Option<u8> {
        self.get(id).and_then(|v| v.first().copied())
    }
    pub fn get_u16(&self, id: u8) -> Option<u16> {
        self.get(id).and_then(|v| v.get(..2)).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }
    pub fn get_str(&self, id: u8) -> Option<&str> {
        self.get(id).and_then(|v| std::str::from_utf8(v).ok())
    }
    pub fn room_id(&self) -> Option<&str> {
        self.get_str(field::ROOM)
    }
    pub fn seq(&self) -> Option<u16> {
        self.get_u16(field::SEQ)
    }

    pub fn encode(&self) -> Result<Vec<u8>, MbcpError> {
        if self.ack_req && !self.msg_type.ack_bit_allowed() {
            return Err(MbcpError::AckNotAllowed(self.msg_type));
        }
        if self.tlvs.len() > 255 {
            return Err(MbcpError::TooManyTlvs(self.tlvs.len()));
        }
        let mut out = Vec::with_capacity(2 + self.tlvs.iter().map(|t| 2 + t.value.len()).sum::<usize>());
        out.push(self.msg_type.code() | if self.ack_req { ACK_MASK } else { 0 });
        out.push(self.tlvs.len() as u8);
        for t in &self.tlvs {
            if t.value.len() > MAX_TLV_VALUE_LEN {
                return Err(MbcpError::ValueTooLong { id: t.id, len: t.value.len() });
            }
            out.push(t.id);
            out.push(t.value.len() as u8);
            out.extend_from_slice(&t.value);
        }
        Ok(out)
    }

    /// `None` = 처리 불가(짧다 · 버전 비트 · 모르는 Type). 잘린 TLV 는 거기까지 읽고 멈춘다 —
    /// 그때까지 읽은 것은 유효하다. 모르는 id 는 그대로 싣는다(호출자가 무시한다).
    pub fn decode(payload: &[u8]) -> Option<Msg> {
        let (&head, rest) = payload.split_first()?;
        if head & VER_MASK != 0 || head & RESERVED_MASK != 0 {
            return None;
        }
        let msg_type = MsgType::from_code(head & TYPE_MASK)?;
        let ack_req = head & ACK_MASK != 0;
        let (&count, mut rest) = rest.split_first()?;
        let mut tlvs = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let (Some(&id), Some(&len)) = (rest.first(), rest.get(1)) else { break };
            let len = usize::from(len);
            let Some(value) = rest.get(2..2 + len) else { break };
            tlvs.push(Tlv { id, value: value.to_vec() });
            rest = &rest[2 + len..];
        }
        Some(Msg { msg_type, ack_req, tlvs })
    }
}

/// 연§11-5 — 클라가 내는 셋 + ACK. 화자·신원은 싣지 않는다(연결이 준다).
pub fn floor_request(room_id: &str, priority: Option<u8>) -> Msg {
    let m = Msg::new(MsgType::Request).room(room_id);
    match priority {
        Some(p) => m.u8(field::PRIORITY, p),
        None => m,
    }
}
pub fn floor_release(room_id: &str) -> Msg {
    Msg::new(MsgType::Release).room(room_id)
}
pub fn queue_pos_request(room_id: &str) -> Msg {
    Msg::new(MsgType::QueuePosRequest).room(room_id)
}
/// TLV `12` = ACK 하는 메시지의 Type 값, `0x1D` = 대상 것 에코.
pub fn floor_ack(room_id: &str, of: MsgType) -> Msg {
    Msg::new(MsgType::Ack).u8(field::ACK_TYPE, of.code()).room(room_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtype_values_follow_ts_24_380() {
        assert_eq!(MsgType::Taken.code(), 2);
        assert_eq!(MsgType::Deny.code(), 3);
        assert_eq!(MsgType::Release.code(), 4);
        assert_eq!(MsgType::Idle.code(), 5);
        for retired in [7u8, 11, 14, 15, 12, 13] {
            assert_eq!(MsgType::from_code(retired), None);
        }
    }

    #[test]
    fn ack_bit_is_encoding_not_policy() {
        assert!(Msg::new(MsgType::Revoke).ack(true).encode().is_err());
        assert!(Msg::new(MsgType::Request).ack(true).encode().is_err());
        assert!(Msg::new(MsgType::Granted).ack(true).encode().is_ok());
        assert_eq!(Msg::new(MsgType::Granted).ack(true).encode().unwrap()[0], 0x11);
    }

    #[test]
    fn tlv_rules_unknown_id_order_truncation() {
        let bytes = Msg::new(MsgType::Taken).u16(field::SEQ, 7).str(field::GRANTED_PARTY, "u2").room("r1")
            .tlv(0x40, [1, 2]).encode().unwrap();
        let m = Msg::decode(&bytes).unwrap();
        assert_eq!(m.room_id(), Some("r1"));
        assert_eq!(m.seq(), Some(7));
        assert_eq!(m.get(0x40), Some(&[1u8, 2][..]));
        let truncated = &bytes[..bytes.len() - 3];
        let t = Msg::decode(truncated).unwrap();
        assert_eq!(t.seq(), Some(7));
        assert_eq!(t.get_str(field::GRANTED_PARTY), Some("u2"));
        assert!(t.get(field::ROOM).is_none() || t.get(0x40).is_none());
        assert_eq!(Msg::decode(&[0x40, 0]), None);
        assert_eq!(Msg::decode(&[0x07, 0]), None);
        assert_eq!(Msg::decode(&[]), None);
    }

    #[test]
    fn value_len_cap_is_255() {
        let long = "x".repeat(256);
        assert!(matches!(Msg::new(MsgType::Request).str(field::ROOM, &long).encode(), Err(MbcpError::ValueTooLong { .. })));
    }
}
