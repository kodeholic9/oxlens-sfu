// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-FLR-HEADER · 연:RULE-FLR-MSG-TYPES · 연:RULE-FLR-TLV · 연:RULE-FLR-TIMERS · 연:FLR.wire · model: claude-opus-5-5
//! 발언권 메시지(MBCP) 부호화 — 머리(V · R · A · Type) · TLV 개수 · TLV. 원문 번호 · 사유 · 타이머 값.

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

pub const RETIRED: &[u8] = &[7, 28, 30];
pub const MAX_VALUE: usize = 255;
pub const SEQ_WRAP: u32 = 65_536;

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
        REQUEST => "REQUEST",
        GRANTED => "GRANTED",
        TAKEN => "TAKEN",
        DENY => "DENY",
        RELEASE => "RELEASE",
        IDLE => "IDLE",
        REVOKE => "REVOKE",
        QPOS_REQ => "QPOS_REQ",
        QINFO => "QINFO",
        ACK => "ACK",
        _ => "UNKNOWN",
    }
}

pub fn encode(m: &Msg) -> Option<Vec<u8>> {
    if m.ack && !ack_allowed(m.msg_type) {
        return None;
    }
    if m.tlvs.len() > 255 || m.tlvs.iter().any(|(_, v)| v.len() > MAX_VALUE) {
        return None;
    }
    let mut out = Vec::with_capacity(2 + m.tlvs.iter().map(|(_, v)| v.len() + 2).sum::<usize>());
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

    pub fn room(&self) -> Option<String> {
        self.get(tlv::ROOM).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    pub fn priority(&self) -> Option<u8> {
        self.get(tlv::PRIORITY).and_then(|b| if b.len() == 2 { Some(b[0]) } else { None })
    }

    pub fn seq(&self) -> Option<u16> {
        self.get(tlv::SEQ).and_then(|b| if b.len() == 2 { Some(u16::from_be_bytes([b[0], b[1]])) } else { None })
    }

    pub fn ack_of(&self) -> Option<u8> {
        self.get(tlv::ACKTYPE).and_then(|b| if b.len() == 2 { Some(b[0]) } else { None })
    }

    pub fn cause(&self) -> Option<u8> {
        self.get(tlv::CAUSE).and_then(|b| b.first().copied())
    }

    pub fn text(&self, id: u8) -> Option<String> {
        self.get(id).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn push(mut self, id: u8, v: Vec<u8>) -> Msg {
        self.tlvs.push((id, v));
        self
    }
}

fn u8spare(v: u8) -> Vec<u8> {
    vec![v, 0]
}

fn u16be(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn utf8(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

pub fn request(room: &str, priority: Option<u8>) -> Msg {
    let m = Msg::new(REQUEST, false).push(tlv::ROOM, utf8(room));
    match priority {
        Some(p) => m.push(tlv::PRIORITY, u8spare(p)),
        None => m,
    }
}

pub fn granted(priority: u8, duration_s: u16, seq: u16, room: &str) -> Msg {
    Msg::new(GRANTED, true)
        .push(tlv::PRIORITY, u8spare(priority))
        .push(tlv::DURATION, u16be(duration_s))
        .push(tlv::SEQ, u16be(seq))
        .push(tlv::ROOM, utf8(room))
}

pub fn taken(speaker: &str, seq: u16, room: &str) -> Msg {
    Msg::new(TAKEN, true).push(tlv::SPEAKER, utf8(speaker)).push(tlv::SEQ, u16be(seq)).push(tlv::ROOM, utf8(room))
}

pub fn deny_msg(cause: u8, room: &str, reason: Option<&str>) -> Msg {
    let m = Msg::new(DENY, true).push(tlv::CAUSE, vec![cause]).push(tlv::ROOM, utf8(room));
    match reason {
        Some(r) => m.push(tlv::REASON, utf8(r)),
        None => m,
    }
}

pub fn release(room: &str) -> Msg {
    Msg::new(RELEASE, false).push(tlv::ROOM, utf8(room))
}

pub fn idle(seq: u16, room: &str, prev: Option<&str>) -> Msg {
    let m = Msg::new(IDLE, false).push(tlv::SEQ, u16be(seq)).push(tlv::ROOM, utf8(room));
    match prev {
        Some(p) => m.push(tlv::PREV, utf8(p)),
        None => m,
    }
}

pub fn revoke_msg(cause: u8, room: &str, reason: Option<&str>) -> Msg {
    let m = Msg::new(REVOKE, false).push(tlv::CAUSE, vec![cause]).push(tlv::ROOM, utf8(room));
    match reason {
        Some(r) => m.push(tlv::REASON, utf8(r)),
        None => m,
    }
}

pub fn queue_pos_request(room: &str) -> Msg {
    Msg::new(QPOS_REQ, false).push(tlv::ROOM, utf8(room))
}

pub fn queue_info(position: u8, priority: u8, room: &str) -> Msg {
    Msg::new(QINFO, false).push(tlv::QINFO, vec![position, priority]).push(tlv::ROOM, utf8(room))
}

pub fn ack(of: u8, room: &str, seq: Option<u16>) -> Msg {
    let m = Msg::new(ACK, false).push(tlv::ACKTYPE, u8spare(of)).push(tlv::ROOM, utf8(room));
    match seq {
        Some(s) => m.push(tlv::SEQ, u16be(s)),
        None => m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_roundtrip() {
        let m = granted(5, 30, 7, "r1");
        let b = encode(&m).unwrap();
        assert_eq!(b[0], 0x11);
        let d = decode(&b).unwrap();
        assert_eq!(d.priority(), Some(5));
        assert_eq!(d.seq(), Some(7));
        assert_eq!(d.room().as_deref(), Some("r1"));
        assert!(d.ack);
    }

    #[test]
    fn rejects_version_and_retired() {
        assert!(decode(&[0x40, 0]).is_none());
        assert!(decode(&[7, 0]).is_none());
        assert!(encode(&Msg::new(REVOKE, true)).is_none());
    }

    #[test]
    fn truncated_tlv_stops() {
        let d = decode(&[REQUEST, 2, tlv::ROOM, 2, b'a', b'b', tlv::PRIORITY, 9]).unwrap();
        assert_eq!(d.tlvs.len(), 1);
        assert_eq!(d.priority(), None);
    }
}
