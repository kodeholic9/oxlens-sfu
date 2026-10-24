// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:RULE-TRN-OP-NUMBERS · 연:RULE-TRN-LANES · 연:RULE-TRN-ROOM-STATE-ONE-LANE · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-S2C-NAMING · 연:RULE-TRN-NOTIFY-ACK · model: claude-opus-5-5
//! op 표 — 번호 · 우선순위 단 · 윈도우 밖 · 응답 갈래 · 방향.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane { Recovery, Session, Data, Diagnostic }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op { Affiliation, Message, Task, IceRestart, LinkEvent, MlineEvent, PublishTracks, TrackSet, RoomJoin, RoomLeave, ParticipantEvent, ParticipantState, RoomEvent, Bind, Resume, Heartbeat, Leave, Ready, SubscribeLayer, TrackEvent, TrackState }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply { Response, None, Ack }

impl Op {
    pub const ALL: &'static [Op] = &[Op::Affiliation, Op::Message, Op::Task, Op::IceRestart, Op::LinkEvent, Op::MlineEvent, Op::PublishTracks, Op::TrackSet, Op::RoomJoin, Op::RoomLeave, Op::ParticipantEvent, Op::ParticipantState, Op::RoomEvent, Op::Bind, Op::Resume, Op::Heartbeat, Op::Leave, Op::Ready, Op::SubscribeLayer, Op::TrackEvent, Op::TrackState];
    pub fn from_u16(v: u16) -> Option<Op> {
        match v { 0x0401 => Some(Op::Affiliation), 0x0501 => Some(Op::Message), 0x0601 => Some(Op::Task), 0x0305 => Some(Op::IceRestart), 0x0706 => Some(Op::LinkEvent), 0x0707 => Some(Op::MlineEvent), 0x0301 => Some(Op::PublishTracks), 0x0304 => Some(Op::TrackSet), 0x0201 => Some(Op::RoomJoin), 0x0202 => Some(Op::RoomLeave), 0x0701 => Some(Op::ParticipantEvent), 0x0705 => Some(Op::ParticipantState), 0x0704 => Some(Op::RoomEvent), 0x0101 => Some(Op::Bind), 0x0102 => Some(Op::Resume), 0x0103 => Some(Op::Heartbeat), 0x0104 => Some(Op::Leave), 0x0302 => Some(Op::Ready), 0x0303 => Some(Op::SubscribeLayer), 0x0702 => Some(Op::TrackEvent), 0x0703 => Some(Op::TrackState), _ => None }
    }
    pub fn as_u16(self) -> u16 {
        match self { Op::Affiliation => 0x0401, Op::Message => 0x0501, Op::Task => 0x0601, Op::IceRestart => 0x0305, Op::LinkEvent => 0x0706, Op::MlineEvent => 0x0707, Op::PublishTracks => 0x0301, Op::TrackSet => 0x0304, Op::RoomJoin => 0x0201, Op::RoomLeave => 0x0202, Op::ParticipantEvent => 0x0701, Op::ParticipantState => 0x0705, Op::RoomEvent => 0x0704, Op::Bind => 0x0101, Op::Resume => 0x0102, Op::Heartbeat => 0x0103, Op::Leave => 0x0104, Op::Ready => 0x0302, Op::SubscribeLayer => 0x0303, Op::TrackEvent => 0x0702, Op::TrackState => 0x0703, }
    }
    pub fn lane(self) -> Lane {
        match self { Op::Affiliation => Lane::Session, Op::Message => Lane::Data, Op::Task => Lane::Diagnostic, Op::IceRestart => Lane::Session, Op::LinkEvent => Lane::Session, Op::MlineEvent => Lane::Session, Op::PublishTracks => Lane::Session, Op::TrackSet => Lane::Session, Op::RoomJoin => Lane::Session, Op::RoomLeave => Lane::Session, Op::ParticipantEvent => Lane::Session, Op::ParticipantState => Lane::Session, Op::RoomEvent => Lane::Session, Op::Bind => Lane::Session, Op::Resume => Lane::Session, Op::Heartbeat => Lane::Session, Op::Leave => Lane::Session, Op::Ready => Lane::Session, Op::SubscribeLayer => Lane::Session, Op::TrackEvent => Lane::Session, Op::TrackState => Lane::Session, }
    }
    pub fn outside_window(self) -> bool {
        matches!(self, Op::Heartbeat)
    }
    pub fn outside_queue(self) -> bool {
        matches!(self, Op::Leave)
    }
    pub fn reply(self) -> Reply {
        match self {
            Op::Leave => Reply::None,
            Op::ParticipantEvent | Op::ParticipantState | Op::RoomEvent | Op::TrackEvent | Op::TrackState | Op::LinkEvent | Op::MlineEvent => Reply::Ack,
            _ => Reply::Response,
        }
    }
    pub fn client_sends(self) -> bool {
        !matches!(self.reply(), Reply::Ack)
    }
    pub fn name(self) -> &'static str {
        match self {
            Op::Affiliation => "AFFILIATION", Op::Message => "MESSAGE", Op::Task => "TASK", Op::IceRestart => "ICE_RESTART",
            Op::LinkEvent => "LINK_EVENT", Op::MlineEvent => "MLINE_EVENT", Op::PublishTracks => "PUBLISH_TRACKS", Op::TrackSet => "TRACK_SET",
            Op::RoomJoin => "ROOM_JOIN", Op::RoomLeave => "ROOM_LEAVE", Op::ParticipantEvent => "PARTICIPANT_EVENT",
            Op::ParticipantState => "PARTICIPANT_STATE", Op::RoomEvent => "ROOM_EVENT", Op::Bind => "BIND", Op::Resume => "RESUME",
            Op::Heartbeat => "HEARTBEAT", Op::Leave => "LEAVE", Op::Ready => "READY", Op::SubscribeLayer => "SUBSCRIBE_LAYER",
            Op::TrackEvent => "TRACK_EVENT", Op::TrackState => "TRACK_STATE",
        }
    }
    pub fn catalog(self) -> String {
        format!("CP-OP-{}", self.name().replace('_', "-"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_round_trips_and_has_one_lane() {
        for op in Op::ALL {
            assert_eq!(Op::from_u16(op.as_u16()), Some(*op));
        }
        assert_eq!(Op::ALL.len(), 21);
        assert_eq!(Op::Message.lane(), Lane::Data);
        assert_eq!(Op::Task.lane(), Lane::Diagnostic);
        assert!(Op::Heartbeat.outside_window());
        assert_eq!(Op::RoomJoin.catalog(), "CP-OP-ROOM-JOIN");
    }
}
