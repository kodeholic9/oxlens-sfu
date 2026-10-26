// author: kodeholic (powered by Claude)
// spec: v1.4 · 연:RULE-TRN-OP-NUMBERS · 연:RULE-TRN-LANES · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-ROOM-STATE-ONE-LANE · 연:RULE-TRN-S2C-NAMING · 연:RULE-TRN-SESSION-AXIS-LEAVE · 연:RULE-TRN-OUTSIDE-NAMING · model: claude-opus-5-5
//! WS op 전량 — 번호 · 이름 · 방향 · 우선순위 단 · 윈도 밖.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Recovery,
    Session,
    Data,
    Diagnostic,
}

impl Lane {
    pub fn index(self) -> usize {
        match self {
            Lane::Recovery => 0,
            Lane::Session => 1,
            Lane::Data => 2,
            Lane::Diagnostic => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    ClientToServer,
    ServerToClient,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Response,
    Ack,
    None,
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

    pub fn from_u16(v: u16) -> Option<Op> {
        match v {
            0x0101 => Some(Op::Bind),
            0x0102 => Some(Op::Resume),
            0x0103 => Some(Op::Heartbeat),
            0x0104 => Some(Op::Leave),
            0x0201 => Some(Op::RoomJoin),
            0x0202 => Some(Op::RoomLeave),
            0x0301 => Some(Op::PublishTracks),
            0x0302 => Some(Op::Ready),
            0x0303 => Some(Op::SubscribeLayer),
            0x0304 => Some(Op::TrackSet),
            0x0305 => Some(Op::IceRestart),
            0x0401 => Some(Op::Affiliation),
            0x0501 => Some(Op::Message),
            0x0601 => Some(Op::Task),
            0x0701 => Some(Op::ParticipantEvent),
            0x0702 => Some(Op::TrackEvent),
            0x0703 => Some(Op::TrackState),
            0x0704 => Some(Op::RoomEvent),
            0x0705 => Some(Op::ParticipantState),
            0x0706 => Some(Op::LinkEvent),
            0x0707 => Some(Op::MlineEvent),
            _ => None,
        }
    }

    pub fn code(self) -> u16 {
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

    pub fn lane(self) -> Lane {
        match self {
            Op::Message => Lane::Data,
            Op::Task => Lane::Diagnostic,
            _ => Lane::Session,
        }
    }

    pub fn dir(self) -> Dir {
        match self {
            Op::Heartbeat | Op::Leave | Op::Message | Op::Task => Dir::Both,
            Op::ParticipantEvent | Op::TrackEvent | Op::TrackState | Op::RoomEvent | Op::ParticipantState | Op::LinkEvent | Op::MlineEvent => {
                Dir::ServerToClient
            }
            _ => Dir::ClientToServer,
        }
    }

    pub fn reply(self) -> Reply {
        match self {
            Op::Leave => Reply::None,
            Op::ParticipantEvent | Op::TrackEvent | Op::TrackState | Op::RoomEvent | Op::ParticipantState | Op::LinkEvent | Op::MlineEvent => Reply::Ack,
            _ => Reply::Response,
        }
    }

    pub fn outside_window(self) -> bool {
        matches!(self, Op::Heartbeat)
    }

    pub fn outside_queue(self) -> bool {
        matches!(self, Op::Leave)
    }

    pub fn session_level(self) -> bool {
        matches!(self, Op::Bind | Op::Resume | Op::Heartbeat | Op::Leave)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for op in Op::ALL {
            assert_eq!(Op::from_u16(op.code()), Some(*op));
            assert!(seen.insert(op.code()));
        }
        assert_eq!(Op::ALL.len(), 21);
    }

    #[test]
    fn room_state_carriers_share_one_lane() {
        for op in [Op::RoomEvent, Op::ParticipantEvent, Op::TrackEvent, Op::TrackState, Op::ParticipantState, Op::Affiliation] {
            assert_eq!(op.lane(), Lane::Session);
        }
        assert_eq!(Op::Message.lane(), Lane::Data);
        assert_eq!(Op::Task.lane(), Lane::Diagnostic);
        assert!(Op::Heartbeat.outside_window());
        assert_eq!(Op::ALL.iter().filter(|o| o.outside_window()).count(), 1);
    }
}
