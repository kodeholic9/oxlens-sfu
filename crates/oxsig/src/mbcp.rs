// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 생성본 spec/protocol/out/floor/codec/wire.rs · spec/protocol/out/floor/consts/floor_consts.rs · 연:RULE-FLR-TLV · model: claude-opus-5-5
//! 발언권 DC 메시지(MBCP 꼴) 코덱 · 상수 — 생성본을 그대로 옮긴다.

pub const REQUEST: u8 = 0;
pub const GRANTED: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENY: u8 = 3;
pub const RELEASE: u8 = 4;
pub const IDLE: u8 = 5;
pub const REVOKE: u8 = 6;
pub const QPOS_REQ: u8 = 8;
pub const QINFO: u8 = 9;
pub const ACK: u8 = 10;

pub mod tlv {
    pub const PRIORITY: u8 = 0;
    pub const DURATION: u8 = 1;
    pub const CAUSE: u8 = 2;
    pub const QINFO: u8 = 3;
    pub const SPEAKER: u8 = 4;
    pub const SEQ: u8 = 8;
    pub const ACKTYPE: u8 = 12;
    pub const PREV: u8 = 26;
    pub const REASON: u8 = 27;
    pub const ROOM: u8 = 29;
}

pub mod timers {
    pub const T101_MS: u64 = 500;
    pub const T100_MS: u64 = 500;
    pub const T104_MS: u64 = 500;
    pub const T132_MS: u64 = 2000;
    pub const T1_MS: u64 = 4000;
    pub const T1MAX_MS: u64 = 6000;
    pub const T2_MS: u64 = 30000;
    pub const T3_MS: u64 = 3000;
    pub const T7_MS: u64 = 1000;
    pub const T8_MS: u64 = 1000;
    pub const T20_MS: u64 = 1000;
    pub const T9_MS: u64 = 3000;
    pub const T21_MS: u64 = 1000;
    pub const C101: u8 = 3;
    pub const C100: u8 = 3;
    pub const C104: u8 = 3;
    pub const C7: u8 = 10;
    pub const C8: u8 = 3;
    pub const C20: u8 = 3;
    pub const C21: u8 = 3;
}

pub mod deny {
    pub const ONLY_ONE: u8 = 3;
    pub const RETRY_AFTER: u8 = 4;
    pub const RECV_ONLY: u8 = 5;
    pub const QUEUE_FULL: u8 = 7;
    pub const NOT_PUBROOM: u8 = 101;
    pub const NO_PERMISSION: u8 = 102;
    pub const OTHER_REASON: u8 = 255;
}

pub mod revoke {
    pub const TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
}

pub mod qpos {
    pub const NOT_IN_QUEUE: u8 = 254;
    pub const UNKNOWN: u8 = 255;
}

pub const SEQ_WRAP: u32 = 65536;
pub const RETIRED: &[u8] = &[7, 28, 30];
pub const MAX_VALUE: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub msg_type: u8,
    pub ack: bool,
    pub tlvs: Vec<(u8, Vec<u8>)>,
}

impl Msg {
    pub fn new(msg_type: u8) -> Msg {
        Msg { msg_type, ack: false, tlvs: Vec::new() }
    }

    pub fn tlv(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_slice())
    }

    pub fn push(&mut self, id: u8, v: Vec<u8>) {
        self.tlvs.push((id, v));
    }
}

pub fn adopted(t: u8) -> bool {
    matches!(t, REQUEST | GRANTED | TAKEN | DENY | RELEASE | IDLE | REVOKE | QPOS_REQ | QINFO | ACK)
}

pub fn ack_allowed(t: u8) -> bool {
    matches!(t, GRANTED | TAKEN | DENY | RELEASE | IDLE | QINFO)
}

pub fn encode(m: &Msg) -> Option<Vec<u8>> {
    if m.ack && !ack_allowed(m.msg_type) {
        return None;
    }
    if m.tlvs.len() > 255 || m.tlvs.iter().any(|(_, v)| v.len() > MAX_VALUE) {
        return None;
    }
    let mut out = Vec::with_capacity(2 + m.tlvs.len() * 4);
    out.push((m.msg_type & 0x0f) | if m.ack { 0x10 } else { 0 });
    out.push(m.tlvs.len() as u8);
    for (id, v) in &m.tlvs {
        out.push(*id);
        out.push(v.len() as u8);
        out.extend_from_slice(v);
    }
    Some(out)
}

pub fn decode(b: &[u8]) -> Option<Msg> {
    if b.len() < 2 {
        return None;
    }
    let head = b[0];
    if head & 0xc0 != 0x00 {
        return None;
    }
    let t = head & 0x0f;
    if !adopted(t) {
        return None;
    }
    let mut m = Msg { msg_type: t, ack: head & 0x10 != 0, tlvs: Vec::new() };
    let mut rest = &b[2..];
    for _ in 0..b[1] {
        if rest.len() < 2 {
            break;
        }
        let (id, len) = (rest[0], rest[1] as usize);
        if rest.len() < 2 + len {
            break;
        }
        m.tlvs.push((id, rest[2..2 + len].to_vec()));
        rest = &rest[2 + len..];
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut m = Msg::new(GRANTED);
        m.ack = true;
        m.push(tlv::DURATION, vec![0, 30]);
        m.push(tlv::ROOM, b"r1".to_vec());
        let d = decode(&encode(&m).expect("encode")).expect("decode");
        assert_eq!(d, m);
        assert_eq!(d.tlv(tlv::ROOM), Some(&b"r1"[..]));
    }

    #[test]
    fn rejects_ack_on_request_and_retired() {
        let mut m = Msg::new(REQUEST);
        m.ack = true;
        assert!(encode(&m).is_none());
        assert!(decode(&[7, 0]).is_none());
    }
}
