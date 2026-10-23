// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:TRN.frame.WS · 연:TRN.frame.DC · 연:RULE-TRN-TWO-FRAMES · 연:RULE-TRN-CLOSE · 연:RULE-TRN-LANES · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-OP-NUMBERS · model: claude-opus-5-5
//! 운반 — WS 프레임(8B 머리 + JSON) · DC 프레임(4B 머리 + payload) · op 번호와 우선순위 단.

pub const WS_VER: u8 = 1;
pub const WS_HEADER: usize = 8;
pub const WS_MAX_FRAME: usize = 1_048_576;
pub const DC_VER: u8 = 1;
pub const DC_HEADER: usize = 4;
pub const DC_MAX_LEN: usize = 65_532;
pub const SVC_MBCP: u8 = 0x01;
pub const SVC_VAD: u8 = 0x02;
pub const WINDOW_MAX: usize = 10;
pub const QUEUE_MAX: usize = 1_000;

pub mod op {
    pub const BIND: u16 = 0x0101;
    pub const RESUME: u16 = 0x0102;
    pub const HEARTBEAT: u16 = 0x0103;
    pub const LEAVE: u16 = 0x0104;
    pub const ROOM_JOIN: u16 = 0x0201;
    pub const ROOM_LEAVE: u16 = 0x0202;
    pub const PUBLISH_TRACKS: u16 = 0x0301;
    pub const READY: u16 = 0x0302;
    pub const SUBSCRIBE_LAYER: u16 = 0x0303;
    pub const TRACK_SET: u16 = 0x0304;
    pub const ICE_RESTART: u16 = 0x0305;
    pub const AFFILIATION: u16 = 0x0401;
    pub const MESSAGE: u16 = 0x0501;
    pub const TASK: u16 = 0x0601;
    pub const PARTICIPANT_EVENT: u16 = 0x0701;
    pub const TRACK_EVENT: u16 = 0x0702;
    pub const TRACK_STATE: u16 = 0x0703;
    pub const ROOM_EVENT: u16 = 0x0704;
    pub const PARTICIPANT_STATE: u16 = 0x0705;
    pub const LINK_EVENT: u16 = 0x0706;
    pub const MLINE_EVENT: u16 = 0x0707;

    pub const ALL: [u16; 21] = [
        BIND, RESUME, HEARTBEAT, LEAVE, ROOM_JOIN, ROOM_LEAVE, PUBLISH_TRACKS, READY, SUBSCRIBE_LAYER,
        TRACK_SET, ICE_RESTART, AFFILIATION, MESSAGE, TASK, PARTICIPANT_EVENT, TRACK_EVENT, TRACK_STATE,
        ROOM_EVENT, PARTICIPANT_STATE, LINK_EVENT, MLINE_EVENT,
    ];

    pub fn known(op: u16) -> bool {
        ALL.contains(&op)
    }

    pub fn lane(op: u16) -> u8 {
        match op {
            MESSAGE => 2,
            TASK => 3,
            _ => 1,
        }
    }

    pub fn outside_window(op: u16) -> bool {
        op == HEARTBEAT
    }

    pub fn required(op: u16) -> &'static [&'static str] {
        match op {
            BIND => &["token"],
            ROOM_JOIN | ROOM_LEAVE | TRACK_SET => &["room_id"],
            READY => &["room_id", "arev"],
            SUBSCRIBE_LAYER => &["room_id", "targets"],
            ICE_RESTART => &["sfu_id"],
            MESSAGE => &["room_id", "content"],
            TASK => &["phase", "req_id", "type"],
            PUBLISH_TRACKS => &["room_id"],
            _ => &[],
        }
    }

    pub fn name(op: u16) -> &'static str {
        match op {
            BIND => "BIND",
            RESUME => "RESUME",
            HEARTBEAT => "HEARTBEAT",
            LEAVE => "LEAVE",
            ROOM_JOIN => "ROOM_JOIN",
            ROOM_LEAVE => "ROOM_LEAVE",
            PUBLISH_TRACKS => "PUBLISH_TRACKS",
            READY => "READY",
            SUBSCRIBE_LAYER => "SUBSCRIBE_LAYER",
            TRACK_SET => "TRACK_SET",
            ICE_RESTART => "ICE_RESTART",
            AFFILIATION => "AFFILIATION",
            MESSAGE => "MESSAGE",
            TASK => "TASK",
            PARTICIPANT_EVENT => "PARTICIPANT_EVENT",
            TRACK_EVENT => "TRACK_EVENT",
            TRACK_STATE => "TRACK_STATE",
            ROOM_EVENT => "ROOM_EVENT",
            PARTICIPANT_STATE => "PARTICIPANT_STATE",
            LINK_EVENT => "LINK_EVENT",
            MLINE_EVENT => "MLINE_EVENT",
            _ => "UNKNOWN",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Req,
    Ok,
    Fail,
}

impl Kind {
    pub fn bits(self) -> u8 {
        match self {
            Kind::Req => 0b00,
            Kind::Ok => 0b01,
            Kind::Fail => 0b10,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsFrame {
    pub flags: u8,
    pub op: u16,
    pub pid: u32,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WsDecodeError {
    TooLarge,
    TooShort,
    BadVersion,
    ReservedKind,
}

impl WsFrame {
    pub fn kind(&self) -> Option<Kind> {
        match self.flags & 0b11 {
            0b00 => Some(Kind::Req),
            0b01 => Some(Kind::Ok),
            0b10 => Some(Kind::Fail),
            _ => None,
        }
    }

    pub fn new(kind: Kind, op: u16, pid: u32, body: Vec<u8>) -> Self {
        WsFrame { flags: kind.bits(), op, pid, body }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(WS_HEADER + self.body.len());
        out.push(WS_VER);
        out.push(self.flags);
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&self.pid.to_be_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(b: &[u8]) -> Result<WsFrame, WsDecodeError> {
        if b.len() > WS_MAX_FRAME {
            return Err(WsDecodeError::TooLarge);
        }
        if b.len() < WS_HEADER {
            return Err(WsDecodeError::TooShort);
        }
        if b[0] != WS_VER {
            return Err(WsDecodeError::BadVersion);
        }
        if b[1] & 0b11 == 0b11 {
            return Err(WsDecodeError::ReservedKind);
        }
        Ok(WsFrame {
            flags: b[1],
            op: u16::from_be_bytes([b[2], b[3]]),
            pid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            body: b[WS_HEADER..].to_vec(),
        })
    }

    pub fn body_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        if self.body.is_empty() {
            return Ok(serde_json::Value::Object(serde_json::Map::new()));
        }
        serde_json::from_slice(&self.body)
    }
}

pub fn encode_dc(svc: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > DC_MAX_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(DC_HEADER + payload.len());
    out.push(DC_VER);
    out.push(svc);
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

pub fn decode_dc(b: &[u8]) -> Option<(u8, &[u8])> {
    if b.len() < DC_HEADER || b[0] != DC_VER {
        return None;
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if b.len() < DC_HEADER + len {
        return None;
    }
    Some((b[1], &b[DC_HEADER..DC_HEADER + len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_round_trip_keeps_reserved_bits() {
        let f = WsFrame { flags: 0b1010_0001, op: op::ROOM_JOIN, pid: 7, body: b"{}".to_vec() };
        let g = WsFrame::decode(&f.encode()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(g, f);
        assert_eq!(g.kind(), Some(Kind::Ok));
    }

    #[test]
    fn ws_rejects_reserved_kind_and_version() {
        let mut b = WsFrame::new(Kind::Req, op::BIND, 1, vec![]).encode();
        b[1] = 0b11;
        assert_eq!(WsFrame::decode(&b), Err(WsDecodeError::ReservedKind));
        b[1] = 0;
        b[0] = 2;
        assert_eq!(WsFrame::decode(&b), Err(WsDecodeError::BadVersion));
        assert_eq!(WsFrame::decode(&b[..5]), Err(WsDecodeError::TooShort));
    }

    #[test]
    fn dc_truncated_is_dropped() {
        let b = encode_dc(SVC_MBCP, &[1, 2, 3]).unwrap_or_default();
        assert_eq!(decode_dc(&b), Some((SVC_MBCP, &[1u8, 2, 3][..])));
        assert_eq!(decode_dc(&b[..5]), None);
        assert!(encode_dc(SVC_MBCP, &vec![0; DC_MAX_LEN + 1]).is_none());
    }
}
