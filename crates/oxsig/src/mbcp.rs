// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:RULE-FLR-HEADER 연:RULE-FLR-MSG-TYPES 연:RULE-FLR-TLV 연:RULE-FLR-TLV-EXT-RULES 연:FLR.wire 연:DEV-FLR-GRANTED-SEQ 연:DEV-FLR-ACK-SEQ 연:DEV-FLR-T21-SAME-SEQ · model: claude-opus-5-5
//! 발언권 MBCP — 머리(V · R · A · Type) · TLV(1B id · 1B len) 부호화 · 복호 · 상수.

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
    pub const T1_MS: u64 = 4000;
    pub const T1MAX_MS: u64 = 6000;
    pub const T2_MS: u64 = 30000;
    pub const T3_MS: u64 = 3000;
    pub const T7_MS: u64 = 1000;
    pub const T8_MS: u64 = 1000;
    pub const T20_MS: u64 = 1000;
    pub const T9_MS: u64 = 3000;
    pub const T21_MS: u64 = 1000;
    pub const T132_MS: u64 = 2000;
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

pub const RETIRED: &[u8] = &[7, 28, 30];
pub const MAX_VALUE: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Msg {
    pub msg_type: u8,
    pub ack: bool,
    pub tlvs: Vec<(u8, Vec<u8>)>,
}

pub fn adopted(t: u8) -> bool {
    matches!(t, REQUEST | GRANTED | TAKEN | DENY | RELEASE | IDLE | REVOKE | QPOS_REQ | QINFO | ACK)
}

pub fn ack_allowed(t: u8) -> bool {
    matches!(t, GRANTED | TAKEN | DENY | RELEASE | IDLE | QINFO)
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
        QPOS_REQ => "FLOOR_QUEUE_POS_REQUEST",
        QINFO => "FLOOR_QUEUE_INFO",
        ACK => "FLOOR_ACK",
        _ => "FLOOR_UNKNOWN",
    }
}

pub fn catalog_id(t: u8) -> String {
    format!("CP-MSG-{}", name(t).replace('_', "-"))
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

impl Msg {
    pub fn new(msg_type: u8, ack: bool) -> Msg {
        Msg { msg_type, ack, tlvs: Vec::new() }
    }

    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(i, _)| *i == id).map(|(_, v)| v.as_slice())
    }

    pub fn u8_of(&self, id: u8) -> Option<u8> {
        self.get(id).and_then(|v| v.first().copied())
    }

    pub fn u16_of(&self, id: u8) -> Option<u16> {
        self.get(id).filter(|v| v.len() >= 2).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }

    pub fn str_of(&self, id: u8) -> Option<String> {
        self.get(id).and_then(|v| std::str::from_utf8(v).ok()).map(str::to_string)
    }

    pub fn room(&self) -> Option<String> {
        self.str_of(tlv::ROOM)
    }

    pub fn priority(&self) -> u8 {
        self.u8_of(tlv::PRIORITY).unwrap_or(0)
    }

    pub fn seq(&self) -> Option<u16> {
        self.u16_of(tlv::SEQ)
    }

    pub fn with_u8spare(mut self, id: u8, v: u8) -> Msg {
        self.tlvs.push((id, vec![v, 0]));
        self
    }

    pub fn with_u8(mut self, id: u8, v: u8) -> Msg {
        self.tlvs.push((id, vec![v]));
        self
    }

    pub fn with_u16(mut self, id: u8, v: u16) -> Msg {
        self.tlvs.push((id, v.to_be_bytes().to_vec()));
        self
    }

    pub fn with_pair(mut self, id: u8, a: u8, b: u8) -> Msg {
        self.tlvs.push((id, vec![a, b]));
        self
    }

    pub fn with_str(mut self, id: u8, s: &str) -> Msg {
        let b = s.as_bytes();
        let n = b.len().min(MAX_VALUE);
        self.tlvs.push((id, b[..n].to_vec()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_bits_follow_rule_flr_header() {
        let m = Msg::new(TAKEN, true).with_str(tlv::SPEAKER, "u2").with_u16(tlv::SEQ, 7).with_str(tlv::ROOM, "r1");
        let b = encode(&m).expect("enc");
        assert_eq!(b[0], 0x12);
        assert_eq!(b[1], 3);
        let d = decode(&b).expect("dec");
        assert_eq!(d, m);
        assert_eq!(d.room().as_deref(), Some("r1"));
        assert_eq!(d.seq(), Some(7));
    }

    #[test]
    fn ack_bit_refused_where_not_allowed() {
        assert!(encode(&Msg::new(REVOKE, true)).is_none());
        assert!(encode(&Msg::new(REQUEST, true)).is_none());
    }

    #[test]
    fn unadopted_and_bad_version_dropped() {
        assert!(decode(&[7, 0]).is_none());
        assert!(decode(&[0x40, 0]).is_none());
        let truncated = [REQUEST, 2, tlv::ROOM, 5, b'r'];
        assert_eq!(decode(&truncated).map(|m| m.tlvs.len()), Some(0));
    }
}
