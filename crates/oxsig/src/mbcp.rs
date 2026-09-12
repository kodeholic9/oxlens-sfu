// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§11-1 · §11-2 · §11-3 · model: claude-opus-5

//! 발언권 wire(MBCP) — ★**DC 프레임 안에 실려 간다**(`svc = 0x01`, 연§3-3).
//!
//! ```text
//!  byte0 : V(2)=00 | R(1)=0 | A(1) | Type(4)
//!  byte1 : TLV 개수
//!  이후  : [id(1)][len(1)][값(len)] …
//! ```
//!
//! ★**모르는 TLV 는 그대로 싣고 지나간다** — 서버가 필드를 늘려도 옛 클라가 안 깨진다.
//! ★**잘린 TLV 는 거기까지 읽고 멈춘다** — 던지지 않는다(연§3-3 *"잘린 프레임은 조용히"*).

use std::collections::BTreeMap;

const VER_MASK: u8 = 0xC0;
const RESERVED_MASK: u8 = 0x20;
const ACK_MASK: u8 = 0x10;
const TYPE_MASK: u8 = 0x0F;

/// 종류 — ★**원문 subtype 값 그대로**다(연§11-2). 미채택 `7`·`11`·`14`·`15` 는 비워 둔다.
pub const REQUEST: u8 = 0;
pub const GRANTED: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENY: u8 = 3;
pub const RELEASE: u8 = 4;
pub const IDLE: u8 = 5;
pub const REVOKE: u8 = 6;
pub const QUEUE_POS_REQUEST: u8 = 8;
pub const QUEUE_INFO: u8 = 9;
pub const ACK: u8 = 10;

/// TLV id(연§11-3).
pub const F_PRIORITY: u8 = 0;
pub const F_DURATION: u8 = 1;
pub const F_CAUSE: u8 = 2;
pub const F_QUEUE_INFO: u8 = 3;
pub const F_GRANTED_PARTY: u8 = 4;
pub const F_QUEUE_SIZE: u8 = 7;
pub const F_SEQ: u8 = 8;
pub const F_ACK_TYPE: u8 = 12;
pub const F_PREV_SPEAKER: u8 = 0x1A;
pub const F_CAUSE_TEXT: u8 = 0x1B;
/// ★**방은 단수다** — 옛 `destinations` 목록은 폐기됐다(연§11-5).
pub const F_ROOM: u8 = 0x1D;

/// TLV 값의 상한 — 길이 칸이 1바이트다.
pub const MAX_TLV_VALUE: usize = 255;

/// ★**`A` 비트를 실을 수 있는 것**(연§11-2) — 그 밖에 실으면 짓는 쪽이 거부한다.
pub fn ack_bit_allowed(t: u8) -> bool {
    matches!(t, GRANTED | TAKEN | DENY | RELEASE | IDLE | QUEUE_INFO)
}

pub fn name(t: u8) -> &'static str {
    match t {
        REQUEST => "FLOOR_REQUEST",
        GRANTED => "FLOOR_GRANTED",
        TAKEN => "FLOOR_TAKEN",
        DENY => "FLOOR_DENY",
        RELEASE => "FLOOR_RELEASE",
        IDLE => "FLOOR_IDLE",
        REVOKE => "FLOOR_REVOKE",
        QUEUE_POS_REQUEST => "FLOOR_QUEUE_POS_REQUEST",
        QUEUE_INFO => "FLOOR_QUEUE_INFO",
        ACK => "FLOOR_ACK",
        _ => "?",
    }
}

fn known(t: u8) -> bool {
    name(t) != "?"
}

/// 한 통.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub msg_type: u8,
    pub ack_req: bool,
    /// ★**순서를 지킨다** — 원문이 순서를 뜻으로 쓰지는 않지만, 지으면 그대로 나가야
    /// 바이트 대조 시험이 선다.
    pub tlvs: Vec<(u8, Vec<u8>)>,
}

impl Msg {
    pub fn new(msg_type: u8) -> Self {
        Self { msg_type, ack_req: false, tlvs: Vec::new() }
    }

    pub fn with(mut self, id: u8, val: Vec<u8>) -> Self {
        self.tlvs.push((id, val));
        self
    }

    pub fn with_u8(self, id: u8, v: u8) -> Self {
        self.with(id, vec![v])
    }

    pub fn with_u16(self, id: u8, v: u16) -> Self {
        self.with(id, v.to_be_bytes().to_vec())
    }

    pub fn with_str(self, id: u8, v: &str) -> Self {
        self.with(id, v.as_bytes().to_vec())
    }

    pub fn ack(mut self) -> Self {
        self.ack_req = true;
        self
    }

    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_slice())
    }

    pub fn get_u8(&self, id: u8) -> Option<u8> {
        self.get(id).and_then(|v| v.first().copied())
    }

    pub fn get_u16(&self, id: u8) -> Option<u16> {
        self.get(id).filter(|v| v.len() >= 2).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }

    pub fn get_str(&self, id: u8) -> Option<&str> {
        self.get(id).and_then(|v| std::str::from_utf8(v).ok())
    }

    pub fn room(&self) -> Option<&str> {
        self.get_str(F_ROOM)
    }

    /// ★**지을 수 없으면 `None`** — 규격 밖의 것을 바이트로 내보내지 않는다.
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.ack_req && !ack_bit_allowed(self.msg_type) {
            return None;
        }
        if self.tlvs.len() > 255 || self.tlvs.iter().any(|(_, v)| v.len() > MAX_TLV_VALUE) {
            return None;
        }
        let mut out = Vec::with_capacity(2 + self.tlvs.len() * 4);
        out.push(self.msg_type | if self.ack_req { ACK_MASK } else { 0 });
        out.push(self.tlvs.len() as u8);
        for (id, v) in &self.tlvs {
            out.push(*id);
            out.push(v.len() as u8);
            out.extend_from_slice(v);
        }
        Some(out)
    }
}

/// ★`None` = 처리 불가(판이 다르다 · 예약 비트가 섰다 · 모르는 종류 · 너무 짧다).
pub fn decode(payload: &[u8]) -> Option<Msg> {
    if payload.len() < 2 {
        return None;
    }
    let head = payload[0];
    if head & VER_MASK != 0 || head & RESERVED_MASK != 0 {
        return None;
    }
    let t = head & TYPE_MASK;
    if !known(t) {
        return None;
    }
    let mut m = Msg { msg_type: t, ack_req: head & ACK_MASK != 0, tlvs: Vec::new() };
    let mut rest = &payload[2..];
    for _ in 0..payload[1] {
        if rest.len() < 2 {
            break;
        }
        let (id, len) = (rest[0], rest[1] as usize);
        if rest.len() < 2 + len {
            // ★거기까지 읽고 멈춘다 — 반쯤 온 값을 값으로 쓰지 않는다.
            break;
        }
        m.tlvs.push((id, rest[2..2 + len].to_vec()));
        rest = &rest[2 + len..];
    }
    Some(m)
}

/// 거절 사유(`DENY` TLV `2`) — 원문 §8.2.6.2 + 자체 확장.
pub mod reject {
    pub const ANOTHER_HAS_PERMISSION: u8 = 1;
    pub const INTERNAL_ERROR: u8 = 2;
    pub const ONLY_ONE_PARTICIPANT: u8 = 3;
    pub const RETRY_AFTER_NOT_EXPIRED: u8 = 4;
    pub const RECEIVE_ONLY: u8 = 5;
    pub const NO_RESOURCES: u8 = 6;
    pub const QUEUE_FULL: u8 = 7;
    /// ★자체 확장 — 다른 방에서 이미 쥐고 있다.
    pub const HELD_ELSEWHERE: u8 = 100;
    /// ★자체 확장 — `pub_room` 이 아닌 방에서 요청했다(14차 `101`).
    pub const NOT_PUB_ROOM: u8 = 101;
    /// ★자체 확장 — 발언 요청 자격이 없다(18차 `102`).
    pub const NO_PERMISSION: u8 = 102;
    pub const OTHER: u8 = 255;
}

/// 회수 사유(`REVOKE` TLV `2`) — 원문 §8.2.10.
pub mod revoke {
    pub const ONLY_ONE_CLIENT: u8 = 1;
    pub const BURST_TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
    pub const NO_RESOURCES: u8 = 6;
    pub const REVOKED_BY_ANOTHER: u8 = 7;
    pub const OTHER: u8 = 255;
}

/// ★**폐기된 TLV** — 번호를 다시 쓰지 않는다(옛 클라가 다른 뜻으로 읽는다).
pub const RETIRED_FIELDS: &[u8] = &[0x1C, 0x1E];

/// 이름 표를 한 번에 보는 자리 — 덤프·시험이 쓴다.
pub fn names() -> BTreeMap<u8, &'static str> {
    [REQUEST, GRANTED, TAKEN, DENY, RELEASE, IDLE, REVOKE, QUEUE_POS_REQUEST, QUEUE_INFO, ACK]
        .into_iter()
        .map(|t| (t, name(t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 짓고_되읽으면_같다() {
        let m = Msg::new(REQUEST).with_str(F_ROOM, "r1").with_u8(F_PRIORITY, 7);
        let w = m.encode().expect("짓는다");
        assert_eq!(w[0], REQUEST, "★A 비트 없이 나간다");
        assert_eq!(w[1], 2, "★TLV 두 개");
        let back = decode(&w).expect("읽힌다");
        assert_eq!(back, m);
        assert_eq!(back.room(), Some("r1"));
        assert_eq!(back.get_u8(F_PRIORITY), Some(7));
    }

    #[test]
    fn ack_비트는_실을_수_있는_것에만() {
        assert!(Msg::new(GRANTED).ack().encode().is_some());
        // ★요청에 `A` 를 실으면 짓는 쪽이 거부한다 — 바이트로 내보내지 않는다.
        assert!(Msg::new(REQUEST).ack().encode().is_none());
    }

    #[test]
    fn 모르는_종류는_처리_불가다() {
        // 종류 `7` 은 미채택 — 비워 둔 번호다.
        assert!(decode(&[7, 0]).is_none());
        // 판이 다르거나 예약 비트가 서면 우리 것이 아니다.
        assert!(decode(&[0x40, 0]).is_none());
        assert!(decode(&[0x20, 0]).is_none());
    }

    #[test]
    fn 잘린_tlv_는_거기까지만_읽는다() {
        let w = Msg::new(TAKEN).with_str(F_ROOM, "r1").with_u16(F_SEQ, 9).encode().expect("짓는다");
        let cut = &w[..w.len() - 1];
        let m = decode(cut).expect("★던지지 않는다");
        assert_eq!(m.room(), Some("r1"));
        assert_eq!(m.get_u16(F_SEQ), None, "★반쯤 온 값을 값으로 쓰지 않는다");
    }

    #[test]
    fn 모르는_tlv_는_그대로_실려_간다() {
        let mut w = Msg::new(IDLE).with_u16(F_SEQ, 3).encode().expect("짓는다");
        // 서버가 늘린 필드가 하나 더 있다고 하자.
        w[1] = 2;
        w.extend_from_slice(&[0x7E, 1, 0xAB]);
        let m = decode(&w).expect("읽힌다");
        assert_eq!(m.get(0x7E), Some(&[0xAB][..]), "★옛 클라가 안 깨진다");
    }

    #[test]
    fn 값이_상한을_넘으면_안_짓는다() {
        assert!(Msg::new(DENY).with(F_CAUSE_TEXT, vec![0; 256]).encode().is_none());
        assert!(Msg::new(DENY).with(F_CAUSE_TEXT, vec![0; 255]).encode().is_some());
    }
}
