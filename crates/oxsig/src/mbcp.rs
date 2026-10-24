// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:FLR.wire · 연:RULE-FLR-HEADER · 연:RULE-FLR-TLV · 연:RULE-FLR-TIMERS · 연:DEV-FLR-TLV-EXT · 연:DEV-FLR-CAUSE-U8 · 연:DEV-FLR-ACK-SEQ · 연:DEV-FLR-GRANTED-SEQ · model: claude-opus-5-5
//! MBCP — DC 위 발언권 메시지(머리 V·R·A·Type + TLV 개수 · TLV id·len·값) · 원장 상수.

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
        out.push(id.to_owned());
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
    pub fn new(msg_type: u8) -> Msg {
        Msg { msg_type, ack: false, tlvs: Vec::new() }
    }

    pub fn with_ack(mut self, ack: bool) -> Msg {
        self.ack = ack;
        self
    }

    pub fn put_u8spare(mut self, id: u8, v: u8) -> Msg {
        self.tlvs.push((id, vec![v, 0]));
        self
    }

    pub fn put_u8(mut self, id: u8, v: u8) -> Msg {
        self.tlvs.push((id, vec![v]));
        self
    }

    pub fn put_u16(mut self, id: u8, v: u16) -> Msg {
        self.tlvs.push((id, v.to_be_bytes().to_vec()));
        self
    }

    pub fn put_pair(mut self, id: u8, a: u8, b: u8) -> Msg {
        self.tlvs.push((id, vec![a, b]));
        self
    }

    pub fn put_str(mut self, id: u8, v: &str) -> Msg {
        let b = v.as_bytes();
        self.tlvs.push((id, b[..b.len().min(MAX_VALUE)].to_vec()));
        self
    }

    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.tlvs.iter().find(|(i, _)| i == &id).map(|(_, v)| v.as_slice())
    }

    pub fn get_u8spare(&self, id: u8) -> Option<u8> {
        self.get(id).filter(|v| v.len() == 2).map(|v| v[0])
    }

    pub fn get_u8(&self, id: u8) -> Option<u8> {
        self.get(id).filter(|v| !v.is_empty()).map(|v| v[0])
    }

    pub fn get_u16(&self, id: u8) -> Option<u16> {
        self.get(id).filter(|v| v.len() == 2).map(|v| u16::from_be_bytes([v[0], v[1]]))
    }

    pub fn get_str(&self, id: u8) -> Option<String> {
        self.get(id).and_then(|v| std::str::from_utf8(v).ok()).map(str::to_string)
    }

    pub fn room(&self) -> Option<String> {
        self.get_str(tlv::ROOM)
    }
}

pub fn newer(a: u16, b: u16) -> bool {
    a != b && (a.wrapping_sub(b) as i16) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_round_trip() {
        let m = Msg::new(TAKEN).with_ack(true).put_str(tlv::SPEAKER, "u1").put_u16(tlv::SEQ, 41).put_str(tlv::ROOM, "r1");
        let b = encode(&m).expect("enc");
        assert_eq!(b[0], 0x12);
        let d = decode(&b).expect("dec");
        assert_eq!(d, m);
        assert_eq!(d.get_u16(tlv::SEQ), Some(41));
        assert_eq!(d.room().as_deref(), Some("r1"));
    }

    #[test]
    fn rejects_version_and_unknown_and_bad_ack() {
        assert!(decode(&[0x40, 0]).is_none());
        assert!(decode(&[0x07, 0]).is_none());
        assert!(encode(&Msg::new(REVOKE).with_ack(true)).is_none());
        let trunc = decode(&[0x05, 2, 8, 2, 0, 1, 29, 9, b'r']).expect("partial");
        assert_eq!(trunc.tlvs.len(), 1);
    }

    #[test]
    fn serial_newer_wraps() {
        assert!(newer(0, 65535));
        assert!(!newer(65535, 0));
        assert!(!newer(5, 5));
    }
}
