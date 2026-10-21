// author: kodeholic (powered by Claude)
// spec: v1.2 @5f1b3b9 · 연:ERR 연:ERR.type.Failure 연:RULE-ERR-UNKNOWN-BY-LEAD 연:RULE-ERR-FAILURE-FORM · model: claude-opus-5-5
//! 실패 코드 — 단일 숫자 체계 하나(번호 · 이름 · permanent). 폐기 번호는 비워 둔다.

use crate::wire::{Failure, LeaveBody};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Code(pub u16);

impl Code {
    pub const UNKNOWN_OP: Code = Code(1001);
    pub const INVALID_PAYLOAD: Code = Code(1002);
    pub const MISSING_FIELD: Code = Code(1003);
    pub const VERSION_MISMATCH: Code = Code(1004);
    pub const CODEC_REQUIRED: Code = Code(1005);
    pub const CODEC_MISMATCH: Code = Code(1006);
    pub const FIELD_CONFLICT: Code = Code(1007);
    pub const PROTOCOL_ERROR: Code = Code(1008);
    pub const NOT_BOUND: Code = Code(2001);
    pub const TOKEN_INVALID: Code = Code(2002);
    pub const TOKEN_EXPIRED: Code = Code(2003);
    pub const INVALID_API_KEY: Code = Code(2004);
    pub const CLAIM_NOT_ALLOWED: Code = Code(2005);
    pub const NOT_AUTHORIZED: Code = Code(2006);
    pub const SESSION_NOT_FOUND: Code = Code(2008);
    pub const SESSION_REVOKED: Code = Code(2009);
    pub const DUPLICATE_SESSION: Code = Code(2010);
    pub const ROOM_NOT_FOUND: Code = Code(3001);
    pub const NOT_IN_ROOM: Code = Code(3002);
    pub const ROOM_NOT_EMPTY: Code = Code(3004);
    pub const TRACK_NOT_FOUND: Code = Code(3005);
    pub const TRACK_OP_UNSUPPORTED: Code = Code(3006);
    pub const SSRC_COLLISION: Code = Code(3008);
    pub const PRECONDITION_FAILED: Code = Code(3009);
    pub const PEER_NOT_FOUND: Code = Code(3010);
    pub const NOT_PUB_ROOM: Code = Code(3011);
    pub const ROOM_FULL: Code = Code(4001);
    pub const TRACK_LIMIT: Code = Code(4002);
    pub const QUOTA_EXCEEDED: Code = Code(4003);
    pub const LISTEN_LIMIT: Code = Code(4004);
    pub const MID_LIMIT: Code = Code(4005);
    pub const FLOW_TIMEOUT: Code = Code(4006);
    pub const FLOW_OVERFLOW: Code = Code(4007);
    pub const HEARTBEAT_TIMEOUT: Code = Code(4008);
    pub const OVERLOADED: Code = Code(4009);
    pub const SFU_UNAVAILABLE: Code = Code(5001);
    pub const SFU_ERROR: Code = Code(5002);
    pub const INTERNAL_ERROR: Code = Code(5003);
    pub const SERVER_SHUTDOWN: Code = Code(5004);

    pub const TABLE: &'static [(u16, &'static str, Option<bool>)] = &[
        (1001, "UNKNOWN_OP", Some(true)),
        (1002, "INVALID_PAYLOAD", Some(true)),
        (1003, "MISSING_FIELD", Some(true)),
        (1004, "VERSION_MISMATCH", Some(true)),
        (1005, "CODEC_REQUIRED", Some(true)),
        (1006, "CODEC_MISMATCH", Some(true)),
        (1007, "FIELD_CONFLICT", Some(true)),
        (1008, "PROTOCOL_ERROR", Some(true)),
        (2001, "NOT_BOUND", Some(false)),
        (2002, "TOKEN_INVALID", Some(true)),
        (2003, "TOKEN_EXPIRED", Some(false)),
        (2004, "INVALID_API_KEY", Some(true)),
        (2005, "CLAIM_NOT_ALLOWED", Some(true)),
        (2006, "NOT_AUTHORIZED", Some(true)),
        (2008, "SESSION_NOT_FOUND", Some(false)),
        (2009, "SESSION_REVOKED", None),
        (2010, "DUPLICATE_SESSION", None),
        (3001, "ROOM_NOT_FOUND", Some(false)),
        (3002, "NOT_IN_ROOM", Some(false)),
        (3004, "ROOM_NOT_EMPTY", Some(false)),
        (3005, "TRACK_NOT_FOUND", Some(false)),
        (3006, "TRACK_OP_UNSUPPORTED", Some(true)),
        (3008, "SSRC_COLLISION", Some(true)),
        (3009, "PRECONDITION_FAILED", Some(false)),
        (3010, "PEER_NOT_FOUND", Some(false)),
        (3011, "NOT_PUB_ROOM", Some(false)),
        (4001, "ROOM_FULL", Some(false)),
        (4002, "TRACK_LIMIT", Some(false)),
        (4003, "QUOTA_EXCEEDED", Some(false)),
        (4004, "LISTEN_LIMIT", Some(false)),
        (4005, "MID_LIMIT", Some(false)),
        (4006, "FLOW_TIMEOUT", None),
        (4007, "FLOW_OVERFLOW", None),
        (4008, "HEARTBEAT_TIMEOUT", None),
        (4009, "OVERLOADED", Some(false)),
        (5001, "SFU_UNAVAILABLE", Some(false)),
        (5002, "SFU_ERROR", Some(false)),
        (5003, "INTERNAL_ERROR", Some(false)),
        (5004, "SERVER_SHUTDOWN", None),
    ];

    pub const RETIRED: &'static [u16] = &[2007, 3003, 3007];

    fn row(self) -> Option<&'static (u16, &'static str, Option<bool>)> {
        Self::TABLE.iter().find(|r| r.0 == self.0)
    }

    pub fn name(self) -> &'static str {
        self.row().map(|r| r.1).unwrap_or("INTERNAL_ERROR")
    }

    pub fn permanent(self) -> bool {
        match self.row().and_then(|r| r.2) {
            Some(p) => p,
            None => matches!(self.0 / 1000, 1),
        }
    }

    pub fn failure(self) -> Failure {
        Failure { code: self.0, name: self.name().to_string(), permanent: self.permanent(), message: None, details: None }
    }

    pub fn failure_msg(self, message: impl Into<String>) -> Failure {
        Failure { message: Some(message.into()), ..self.failure() }
    }

    pub fn failure_details(self, message: impl Into<String>, details: serde_json::Value) -> Failure {
        Failure { message: Some(message.into()), details: Some(details), ..self.failure() }
    }

    pub fn leave(self) -> LeaveBody {
        LeaveBody { code: self.0, name: self.name().to_string(), message: None, details: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_not_in_table() {
        for r in Code::RETIRED {
            assert!(Code::TABLE.iter().all(|t| t.0 != *r));
        }
    }

    #[test]
    fn failure_shape() {
        let f = Code::ROOM_NOT_FOUND.failure();
        assert_eq!((f.code, f.name.as_str(), f.permanent), (3001, "ROOM_NOT_FOUND", false));
        assert!(Code::CODEC_MISMATCH.permanent());
    }
}
