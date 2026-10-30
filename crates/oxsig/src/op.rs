// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:TRN.oplist · 연:RULE-TRN-LANES · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-OP-NUMBERS · model: claude-opus-5-5
//! WS 메시지 전량 — 번호 · 이름 · 방향 · 우선순위 단 · 윈도우 밖 여부.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Recovery = 0,
    Session = 1,
    Data = 2,
    Diagnostic = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    C2S,
    S2C,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Bind,
    Resume,
    Heartbeat,
    Leave,
    RoomJoin,
    RoomLeave,
    PublishTracks,
    Ready,
    SubscribeLayer,
    TrackSet,
    IceRestart,
    Affiliation,
    Message,
    Task,
    ParticipantEvent,
    TrackEvent,
    TrackState,
    RoomEvent,
    ParticipantState,
    LinkEvent,
    MlineEvent,
}

impl Op {
    pub const ALL: &'static [Op] = &[
        Op::Bind,
        Op::Resume,
        Op::Heartbeat,
        Op::Leave,
        Op::RoomJoin,
        Op::RoomLeave,
        Op::PublishTracks,
        Op::Ready,
        Op::SubscribeLayer,
        Op::TrackSet,
        Op::IceRestart,
        Op::Affiliation,
        Op::Message,
        Op::Task,
        Op::ParticipantEvent,
        Op::TrackEvent,
        Op::TrackState,
        Op::RoomEvent,
        Op::ParticipantState,
        Op::LinkEvent,
        Op::MlineEvent,
    ];

    pub fn as_u16(self) -> u16 {
        match self {
            Op::Bind => 0x0101,
            Op::Resume => 0x0102,
            Op::Heartbeat => 0x0103,
            Op::Leave => 0x0104,
            Op::RoomJoin => 0x0201,
            Op::RoomLeave => 0x0202,
            Op::PublishTracks => 0x0301,
            Op::Ready => 0x0302,
            Op::SubscribeLayer => 0x0303,
            Op::TrackSet => 0x0304,
            Op::IceRestart => 0x0305,
            Op::Affiliation => 0x0401,
            Op::Message => 0x0501,
            Op::Task => 0x0601,
            Op::ParticipantEvent => 0x0701,
            Op::TrackEvent => 0x0702,
            Op::TrackState => 0x0703,
            Op::RoomEvent => 0x0704,
            Op::ParticipantState => 0x0705,
            Op::LinkEvent => 0x0706,
            Op::MlineEvent => 0x0707,
        }
    }

    pub fn from_u16(v: u16) -> Option<Op> {
        Op::ALL.iter().copied().find(|o| o.as_u16() == v)
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Bind => "BIND",
            Op::Resume => "RESUME",
            Op::Heartbeat => "HEARTBEAT",
            Op::Leave => "LEAVE",
            Op::RoomJoin => "ROOM_JOIN",
            Op::RoomLeave => "ROOM_LEAVE",
            Op::PublishTracks => "PUBLISH_TRACKS",
            Op::Ready => "READY",
            Op::SubscribeLayer => "SUBSCRIBE_LAYER",
            Op::TrackSet => "TRACK_SET",
            Op::IceRestart => "ICE_RESTART",
            Op::Affiliation => "AFFILIATION",
            Op::Message => "MESSAGE",
            Op::Task => "TASK",
            Op::ParticipantEvent => "PARTICIPANT_EVENT",
            Op::TrackEvent => "TRACK_EVENT",
            Op::TrackState => "TRACK_STATE",
            Op::RoomEvent => "ROOM_EVENT",
            Op::ParticipantState => "PARTICIPANT_STATE",
            Op::LinkEvent => "LINK_EVENT",
            Op::MlineEvent => "MLINE_EVENT",
        }
    }

    pub fn dir(self) -> Dir {
        match self {
            Op::Heartbeat | Op::Leave | Op::Message | Op::Task => Dir::Both,
            Op::ParticipantEvent
            | Op::TrackEvent
            | Op::TrackState
            | Op::RoomEvent
            | Op::ParticipantState
            | Op::LinkEvent
            | Op::MlineEvent => Dir::S2C,
            _ => Dir::C2S,
        }
    }

    pub fn lane(self) -> Lane {
        match self {
            Op::Message => Lane::Data,
            Op::Task => Lane::Diagnostic,
            _ => Lane::Session,
        }
    }

    pub fn outside_window(self) -> bool {
        matches!(self, Op::Heartbeat)
    }

    pub fn carries_room_state(self) -> bool {
        matches!(
            self,
            Op::RoomEvent | Op::ParticipantEvent | Op::TrackEvent | Op::TrackState | Op::ParticipantState | Op::Affiliation
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_unique_and_roundtrip() {
        let mut seen = std::collections::HashSet::new();
        for o in Op::ALL {
            assert!(seen.insert(o.as_u16()));
            assert_eq!(Op::from_u16(o.as_u16()), Some(*o));
        }
        assert_eq!(Op::ALL.len(), 21);
        assert_eq!(Op::from_u16(0x0999), None);
    }

    #[test]
    fn room_state_one_lane() {
        for o in Op::ALL.iter().filter(|o| o.carries_room_state()) {
            assert_eq!(o.lane(), Lane::Session);
        }
    }
}
