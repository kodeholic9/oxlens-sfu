// author: kodeholic (powered by Claude)
// spec: v1.5 · 연:ERR.type.Failure · 연:TRN.oplist · 연:FLR.wire · 연:RULE-FLR-TIMERS · model: claude-opus-5-5
//! 1층 — wire 상수가 연동규격서 원장 행과 같다: 오류 코드(번호 · 이름 · permanent · 폐기 번호) · op(번호 · 이름 · 방향) · 발언권 메시지 번호 · 발언권 타이머 · 재전송 횟수.

use oxsig::Code;
use oxsig::code::RETIRED;
use oxsig::floor::{self, timers};
use oxsig::op::{Dir, Op};

fn row(c: Code) -> (u16, &'static str, Option<bool>) {
    (c.num(), c.name(), c.permanent())
}

fn op_row(o: Op) -> (u16, &'static str, Dir) {
    (o.as_u16(), o.name(), o.dir())
}

fn unused(n: u16) -> bool {
    RETIRED.contains(&n) && Code::ALL.iter().all(|c| c.num() != n)
}

#[test]
fn error_codes_match_ledger() {
    assert_eq!(row(Code::UnknownOp), (1001, "UNKNOWN_OP", Some(true)), "CP-CODE-UNKNOWN-OP");
    assert_eq!(row(Code::VersionMismatch), (1004, "VERSION_MISMATCH", Some(true)), "CP-CODE-VERSION-MISMATCH");
    assert_eq!(row(Code::FieldConflict), (1007, "FIELD_CONFLICT", Some(true)), "CP-CODE-FIELD-CONFLICT");
    assert_eq!(row(Code::TokenExpired), (2003, "TOKEN_EXPIRED", Some(false)), "CP-CODE-TOKEN-EXPIRED");
    assert_eq!(row(Code::InvalidApiKey), (2004, "INVALID_API_KEY", Some(true)), "CP-CODE-INVALID-API-KEY");
    assert_eq!(row(Code::ClaimNotAllowed), (2005, "CLAIM_NOT_ALLOWED", Some(true)), "CP-CODE-CLAIM-NOT-ALLOWED");
    assert_eq!(row(Code::SessionRevoked), (2009, "SESSION_REVOKED", None), "CP-CODE-SESSION-REVOKED");
    assert_eq!(row(Code::RoomNotEmpty), (3004, "ROOM_NOT_EMPTY", Some(false)), "CP-CODE-ROOM-NOT-EMPTY");
    assert_eq!(row(Code::TrackNotFound), (3005, "TRACK_NOT_FOUND", Some(false)), "CP-CODE-TRACK-NOT-FOUND");
    assert_eq!(row(Code::TrackOpUnsupported), (3006, "TRACK_OP_UNSUPPORTED", Some(true)), "CP-CODE-TRACK-OP-UNSUPPORTED");
    assert_eq!(row(Code::SsrcCollision), (3008, "SSRC_COLLISION", Some(true)), "CP-CODE-SSRC-COLLISION");
    assert_eq!(row(Code::PreconditionFailed), (3009, "PRECONDITION_FAILED", Some(false)), "CP-CODE-PRECONDITION-FAILED");
    assert_eq!(row(Code::PeerNotFound), (3010, "PEER_NOT_FOUND", Some(false)), "CP-CODE-PEER-NOT-FOUND");
    assert_eq!(row(Code::NotPubRoom), (3011, "NOT_PUB_ROOM", Some(false)), "CP-CODE-NOT-PUB-ROOM");
    assert_eq!(row(Code::RoomFull), (4001, "ROOM_FULL", Some(false)), "CP-CODE-ROOM-FULL");
    assert_eq!(row(Code::QuotaExceeded), (4003, "QUOTA_EXCEEDED", Some(false)), "CP-CODE-QUOTA-EXCEEDED");
    assert_eq!(row(Code::ListenLimit), (4004, "LISTEN_LIMIT", Some(false)), "CP-CODE-LISTEN-LIMIT");
    assert_eq!(row(Code::MidLimit), (4005, "MID_LIMIT", Some(false)), "CP-CODE-MID-LIMIT");
    assert_eq!(row(Code::FlowTimeout), (4006, "FLOW_TIMEOUT", None), "CP-CODE-FLOW-TIMEOUT");
    assert_eq!(row(Code::FlowOverflow), (4007, "FLOW_OVERFLOW", None), "CP-CODE-FLOW-OVERFLOW");
    assert_eq!(row(Code::HeartbeatTimeout), (4008, "HEARTBEAT_TIMEOUT", None), "CP-CODE-HEARTBEAT-TIMEOUT");
    assert_eq!(row(Code::Overloaded), (4009, "OVERLOADED", Some(false)), "CP-CODE-OVERLOADED");
    assert_eq!(row(Code::SfuError), (5002, "SFU_ERROR", Some(false)), "CP-CODE-SFU-ERROR");
    assert_eq!(row(Code::InternalError), (5003, "INTERNAL_ERROR", Some(false)), "CP-CODE-INTERNAL-ERROR");
}

#[test]
fn retired_numbers_stay_empty() {
    assert!(unused(2007), "CP-CODE-2007");
    assert!(unused(3003), "CP-CODE-3003");
    assert!(unused(3007), "CP-CODE-3007");
}

#[test]
fn ops_match_ledger() {
    assert_eq!(op_row(Op::SubscribeLayer), (0x0303, "SUBSCRIBE_LAYER", Dir::C2S), "CP-OP-SUBSCRIBE-LAYER");
    assert_eq!(op_row(Op::TrackSet), (0x0304, "TRACK_SET", Dir::C2S), "CP-OP-TRACK-SET");
    assert_eq!(op_row(Op::IceRestart), (0x0305, "ICE_RESTART", Dir::C2S), "CP-OP-ICE-RESTART");
    assert_eq!(op_row(Op::TrackState), (0x0703, "TRACK_STATE", Dir::S2C), "CP-OP-TRACK-STATE");
    assert_eq!(op_row(Op::ParticipantState), (0x0705, "PARTICIPANT_STATE", Dir::S2C), "CP-OP-PARTICIPANT-STATE");
}

#[test]
fn floor_message_and_timers_match_ledger() {
    assert_eq!(floor::QPOS_REQ, 8, "CP-MSG-FLOOR-QUEUE-POS-REQUEST");
    assert_eq!((timers::T101_MS, timers::C101), (500, 3), "CP-TIMER-T101");
    assert_eq!((timers::T100_MS, timers::C100), (500, 3), "CP-TIMER-T100");
    assert_eq!((timers::T104_MS, timers::C104), (500, 3), "CP-TIMER-T104");
    assert_eq!(timers::T132_MS, 2000, "CP-TIMER-T132");
    assert_eq!((timers::T1_MS, timers::T1MAX_MS), (4000, 6000), "CP-TIMER-T1");
    assert_eq!(timers::T2_MS, 30000, "CP-TIMER-T2");
    assert_eq!(timers::T3_MS, 3000, "CP-TIMER-T3");
    assert_eq!((timers::T7_MS, timers::C7), (1000, 10), "CP-TIMER-T7");
    assert_eq!((timers::T8_MS, timers::C8), (1000, 3), "CP-TIMER-T8");
    assert_eq!((timers::T20_MS, timers::C20), (1000, 3), "CP-TIMER-T20");
    assert_eq!(timers::T9_MS, 3000, "CP-TIMER-T9");
}
