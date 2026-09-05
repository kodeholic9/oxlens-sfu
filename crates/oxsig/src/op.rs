// author: kodeholic (powered by Claude)
//! WS 메시지 종류 — 연§6 전량 16. 번호에서 무엇도 파생하지 않는다.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Op {
    Bind = 0x0101,
    Resume = 0x0102,
    Heartbeat = 0x0103,
    RoomJoin = 0x0201,
    RoomLeave = 0x0202,
    PublishTracks = 0x0301,
    Ready = 0x0302,
    SubscribeLayer = 0x0303,
    TrackSet = 0x0304,
    Affiliation = 0x0401,
    Message = 0x0501,
    Task = 0x0601,
    ParticipantEvent = 0x0701,
    TrackEvent = 0x0702,
    TrackState = 0x0703,
    RoomEvent = 0x0704,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    C2S,
    S2C,
    Both,
}

impl Op {
    pub const ALL: [Op; 16] = [
        Op::Bind, Op::Resume, Op::Heartbeat, Op::RoomJoin, Op::RoomLeave, Op::PublishTracks, Op::Ready,
        Op::SubscribeLayer, Op::TrackSet, Op::Affiliation, Op::Message, Op::Task, Op::ParticipantEvent,
        Op::TrackEvent, Op::TrackState, Op::RoomEvent,
    ];

    pub fn code(self) -> u16 {
        self as u16
    }

    /// 모르는 번호는 `None` — 연§10-2 `1001 UNKNOWN_OP` 의 자리.
    pub fn from_code(code: u16) -> Option<Op> {
        Op::ALL.iter().copied().find(|o| o.code() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Bind => "BIND",
            Op::Resume => "RESUME",
            Op::Heartbeat => "HEARTBEAT",
            Op::RoomJoin => "ROOM_JOIN",
            Op::RoomLeave => "ROOM_LEAVE",
            Op::PublishTracks => "PUBLISH_TRACKS",
            Op::Ready => "READY",
            Op::SubscribeLayer => "SUBSCRIBE_LAYER",
            Op::TrackSet => "TRACK_SET",
            Op::Affiliation => "AFFILIATION",
            Op::Message => "MESSAGE",
            Op::Task => "TASK",
            Op::ParticipantEvent => "PARTICIPANT_EVENT",
            Op::TrackEvent => "TRACK_EVENT",
            Op::TrackState => "TRACK_STATE",
            Op::RoomEvent => "ROOM_EVENT",
        }
    }

    pub fn dir(self) -> Dir {
        match self {
            Op::Heartbeat | Op::Message | Op::Task => Dir::Both,
            Op::ParticipantEvent | Op::TrackEvent | Op::TrackState | Op::RoomEvent => Dir::S2C,
            _ => Dir::C2S,
        }
    }

    /// 연§6-7 — S→C 통지 넷. 클라 의무는 ACK 하나.
    pub fn is_notification(self) -> bool {
        self.dir() == Dir::S2C
    }

    /// 연§3-2 우선순위 단: 0 복구 · 1 세션·방·미디어 · 2 데이터 · 3 진단. 같은 단은 FIFO.
    pub fn tier(self) -> u8 {
        match self {
            Op::Resume => 0,
            Op::Message => 2,
            Op::Task => 3,
            _ => 1,
        }
    }

    /// 연§3-2 — `RESUME` 은 윈도우 계산에서 뺀다.
    pub fn counts_toward_window(self) -> bool {
        self != Op::Resume
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({:#06x})", self.name(), self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_unique_and_roundtrip() {
        let mut codes: Vec<u16> = Op::ALL.iter().map(|o| o.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 16);
        for o in Op::ALL {
            assert_eq!(Op::from_code(o.code()), Some(o));
        }
        assert_eq!(Op::from_code(0x1003), None);
        assert_eq!(Op::from_code(0xF001), None);
    }

    #[test]
    fn tiers_follow_spec_3_2() {
        assert_eq!(Op::Resume.tier(), 0);
        assert_eq!(Op::Message.tier(), 2);
        assert_eq!(Op::Task.tier(), 3);
        for o in [Op::RoomEvent, Op::ParticipantEvent, Op::TrackEvent, Op::TrackState, Op::Affiliation] {
            assert_eq!(o.tier(), 1);
        }
        assert!(!Op::Resume.counts_toward_window());
    }
}
