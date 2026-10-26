// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:RULE-FLR-HEADER · 연:RULE-FLR-MSG-TYPES · 연:RULE-FLR-ACK-BIT · 연:RULE-FLR-TLV · 연:RULE-FLR-TLV-EXT-RULES · 연:RULE-FLR-SEQ · 연:DEV-FLR-TLV-EXT · 연:DEV-FLR-CAUSE-U8 · 연:DEV-FLR-DENY-101 · 연:DEV-FLR-DENY-102 · 연:DEV-FLR-DENY-255 · model: claude-opus-5-5
//! 발언권 MBCP — 머리 2바이트(V · R · A · Type / TLV 개수) + TLV. 모르는 TLV 는 담아 두고, 잘리면 거기서 멈춘다.

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

pub const F_PRIORITY: u8 = 0;
pub const F_DURATION: u8 = 1;
pub const F_CAUSE: u8 = 2;
pub const F_QUEUE_INFO: u8 = 3;
pub const F_GRANTED_PARTY: u8 = 4;
pub const F_SEQ: u8 = 8;
pub const F_ACK_TYPE: u8 = 12;
pub const F_PREV_SPEAKER: u8 = 0x1a;
pub const F_CAUSE_TEXT: u8 = 0x1b;
pub const F_ROOM: u8 = 0x1d;

pub const RETIRED_TLV: &[u8] = &[7, 0x1c, 0x1e];
pub const MAX_VALUE: usize = 255;

pub mod deny {
    pub const ONLY_ONE_PARTICIPANT: u8 = 3;
    pub const RETRY_AFTER_NOT_EXPIRED: u8 = 4;
    pub const RECEIVE_ONLY: u8 = 5;
    pub const QUEUE_FULL: u8 = 7;
    pub const NOT_PUB_ROOM: u8 = 101;
    pub const NO_PERMISSION: u8 = 102;
    pub const OTHER: u8 = 255;
}

pub mod revoke {
    pub const ONLY_ONE: u8 = 1;
    pub const TOO_LONG: u8 = 2;
    pub const NO_PERMISSION: u8 = 3;
    pub const PREEMPTED: u8 = 4;
    pub const NO_RESOURCES: u8 = 6;
    pub const BY_OTHER: u8 = 7;
    pub const OTHER: u8 = 255;
}

pub mod qpos {
    pub const NOT_IN_QUEUE: u8 = 254;
    pub const UNKNOWN: u8 = 255;
}

pub const SEQ_WRAP: u32 = 65_536;

pub fn adopted(t: u8) -> bool {
    matches!(t, REQUEST | GRANTED | TAKEN | DENY | RELEASE | IDLE | REVOKE | QUEUE_POS_REQUEST | QUEUE_INFO | ACK)
}

pub fn ack_allowed(t: u8) -> bool {
    matches!(t, GRANTED | TAKEN | DENY | RELEASE | IDLE | QUEUE_INFO)
}

pub fn type_name(t: u8) -> &'static str {
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
        _ => "UNKNOWN",
    }
}

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

    pub fn with_ack(mut self, ack: bool) -> Msg {
        self.ack = ack;
        self
    }

    pub fn put(mut self, id: u8, value: Vec<u8>) -> Msg {
        self.tlvs.push((id, value));
        self
    }

    pub fn put_u8spare(self, id: u8, v: u8) -> Msg {
        self.put(id, vec![v, 0])
    }

    pub fn put_u8(self, id: u8, v: u8) -> Msg {
        self.put(id, vec![v])
    }

    pub fn put_u16(self, id: u8, v: u16) -> Msg {
        self.put(id, v.to_be_bytes().to_vec())
    }

    pub fn put_str(self, id: u8, v: &str) -> Msg {
        self.put(id, v.as_bytes().to_vec())
    }

    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_slice())
    }

    pub fn get_u8spare(&self, id: u8) -> Option<u8> {
        match self.get(id) {
            Some([v, _]) => Some(*v),
            _ => None,
        }
    }

    pub fn get_u8(&self, id: u8) -> Option<u8> {
        match self.get(id) {
            Some([v]) => Some(*v),
            _ => None,
        }
    }

    pub fn get_u16(&self, id: u8) -> Option<u16> {
        match self.get(id) {
            Some([a, b]) => Some(u16::from_be_bytes([*a, *b])),
            _ => None,
        }
    }

    pub fn get_str(&self, id: u8) -> Option<String> {
        self.get(id).and_then(|v| std::str::from_utf8(v).ok()).map(str::to_string)
    }

    pub fn room(&self) -> Option<String> {
        self.get_str(F_ROOM)
    }

    pub fn seq(&self) -> Option<u16> {
        self.get_u16(F_SEQ)
    }
}

pub fn encode(m: &Msg) -> Option<Vec<u8>> {
    if m.ack && !ack_allowed(m.msg_type) {
        return None;
    }
    if m.tlvs.len() > 255 || m.tlvs.iter().any(|(_, v)| v.len() > MAX_VALUE) {
        return None;
    }
    let mut out = Vec::with_capacity(2 + m.tlvs.iter().map(|(_, v)| 2 + v.len()).sum::<usize>());
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
    if head & 0xc0 != 0 {
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

pub fn next_seq(seq: u16) -> u16 {
    seq.wrapping_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_bits_and_round_trip() {
        let m = Msg::new(GRANTED).with_ack(true).put_u8spare(F_PRIORITY, 3).put_u16(F_DURATION, 30).put_u16(F_SEQ, 7).put_str(F_ROOM, "r1");
        let b = encode(&m).expect("enc");
        assert_eq!(b[0], 0x11);
        assert_eq!(b[1], 4);
        let d = decode(&b).expect("dec");
        assert_eq!(d, m);
        assert_eq!(d.get_u8spare(F_PRIORITY), Some(3));
        assert_eq!(d.room().as_deref(), Some("r1"));
    }

    #[test]
    fn revoke_cannot_carry_ack_bit() {
        assert!(encode(&Msg::new(REVOKE).with_ack(true)).is_none());
    }

    #[test]
    fn version_bits_drop_and_r_bit_is_ignored() {
        assert!(decode(&[0x40 | REQUEST, 0]).is_none());
        assert!(decode(&[0x20 | REQUEST, 0]).is_some());
        assert!(decode(&[7, 0]).is_none());
    }

    #[test]
    fn truncated_tlv_stops_and_unknown_is_kept() {
        let d = decode(&[REQUEST, 3, 0x50, 1, 9, F_ROOM, 5, b'r']).expect("dec");
        assert_eq!(d.tlvs, vec![(0x50, vec![9])]);
    }

    #[test]
    fn seq_wraps() {
        assert_eq!(next_seq(65535), 0);
        assert_eq!(SEQ_WRAP, 65536);
    }
}
