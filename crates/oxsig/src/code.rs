// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ERR-ISSUERS · 연:ERR.type.Failure · 연:RULE-ERR-FAILURE-FORM · 연:RULE-ERR-UNKNOWN-BY-LEAD · 연:RULE-ERR-CLOSE-EMPTY · 연:RULE-ERR-FLOOR-OUTSIDE · 연:RULE-ERR-NOT-MIXED · model: claude-opus-5-5
//! 오류 번호 공간 하나 — 번호 · 이름 · `permanent` · `Failure` · `LEAVE` 사유.

use crate::body::LeaveBody;
use crate::types::Failure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Code {
    UnknownOp,
    InvalidPayload,
    MissingField,
    VersionMismatch,
    CodecRequired,
    CodecMismatch,
    FieldConflict,
    ProtocolError,
    NotBound,
    TokenInvalid,
    TokenExpired,
    InvalidApiKey,
    ClaimNotAllowed,
    NotAuthorized,
    SessionNotFound,
    SessionRevoked,
    DuplicateSession,
    RoomNotFound,
    NotInRoom,
    RoomNotEmpty,
    TrackNotFound,
    TrackOpUnsupported,
    SsrcCollision,
    PreconditionFailed,
    PeerNotFound,
    NotPubRoom,
    RoomFull,
    TrackLimit,
    QuotaExceeded,
    ListenLimit,
    MidLimit,
    FlowTimeout,
    FlowOverflow,
    HeartbeatTimeout,
    Overloaded,
    SfuUnavailable,
    SfuError,
    InternalError,
    ServerShutdown,
}

pub const CODES: &[(Code, u16, &str, Option<bool>)] = &[
    (Code::UnknownOp, 1001, "UNKNOWN_OP", Some(true)),
    (Code::InvalidPayload, 1002, "INVALID_PAYLOAD", Some(true)),
    (Code::MissingField, 1003, "MISSING_FIELD", Some(true)),
    (Code::VersionMismatch, 1004, "VERSION_MISMATCH", Some(true)),
    (Code::CodecRequired, 1005, "CODEC_REQUIRED", Some(true)),
    (Code::CodecMismatch, 1006, "CODEC_MISMATCH", Some(true)),
    (Code::FieldConflict, 1007, "FIELD_CONFLICT", Some(true)),
    (Code::ProtocolError, 1008, "PROTOCOL_ERROR", Some(true)),
    (Code::NotBound, 2001, "NOT_BOUND", Some(false)),
    (Code::TokenInvalid, 2002, "TOKEN_INVALID", Some(true)),
    (Code::TokenExpired, 2003, "TOKEN_EXPIRED", Some(false)),
    (Code::InvalidApiKey, 2004, "INVALID_API_KEY", Some(true)),
    (Code::ClaimNotAllowed, 2005, "CLAIM_NOT_ALLOWED", Some(true)),
    (Code::NotAuthorized, 2006, "NOT_AUTHORIZED", Some(true)),
    (Code::SessionNotFound, 2008, "SESSION_NOT_FOUND", Some(false)),
    (Code::SessionRevoked, 2009, "SESSION_REVOKED", None),
    (Code::DuplicateSession, 2010, "DUPLICATE_SESSION", None),
    (Code::RoomNotFound, 3001, "ROOM_NOT_FOUND", Some(false)),
    (Code::NotInRoom, 3002, "NOT_IN_ROOM", Some(false)),
    (Code::RoomNotEmpty, 3004, "ROOM_NOT_EMPTY", Some(false)),
    (Code::TrackNotFound, 3005, "TRACK_NOT_FOUND", Some(false)),
    (Code::TrackOpUnsupported, 3006, "TRACK_OP_UNSUPPORTED", Some(true)),
    (Code::SsrcCollision, 3008, "SSRC_COLLISION", Some(true)),
    (Code::PreconditionFailed, 3009, "PRECONDITION_FAILED", Some(false)),
    (Code::PeerNotFound, 3010, "PEER_NOT_FOUND", Some(false)),
    (Code::NotPubRoom, 3011, "NOT_PUB_ROOM", Some(false)),
    (Code::RoomFull, 4001, "ROOM_FULL", Some(false)),
    (Code::TrackLimit, 4002, "TRACK_LIMIT", Some(false)),
    (Code::QuotaExceeded, 4003, "QUOTA_EXCEEDED", Some(false)),
    (Code::ListenLimit, 4004, "LISTEN_LIMIT", Some(false)),
    (Code::MidLimit, 4005, "MID_LIMIT", Some(false)),
    (Code::FlowTimeout, 4006, "FLOW_TIMEOUT", None),
    (Code::FlowOverflow, 4007, "FLOW_OVERFLOW", None),
    (Code::HeartbeatTimeout, 4008, "HEARTBEAT_TIMEOUT", None),
    (Code::Overloaded, 4009, "OVERLOADED", Some(false)),
    (Code::SfuUnavailable, 5001, "SFU_UNAVAILABLE", Some(false)),
    (Code::SfuError, 5002, "SFU_ERROR", Some(false)),
    (Code::InternalError, 5003, "INTERNAL_ERROR", Some(false)),
    (Code::ServerShutdown, 5004, "SERVER_SHUTDOWN", None),
];
pub const RETIRED: &[u16] = &[2007, 3003, 3007];

impl Code {
    fn row(self) -> &'static (Code, u16, &'static str, Option<bool>) {
        CODES.iter().find(|r| r.0 == self).unwrap_or(&CODES[CODES.len() - 3])
    }

    pub fn number(self) -> u16 {
        self.row().1
    }

    pub fn name(self) -> &'static str {
        self.row().2
    }

    pub fn permanent(self) -> Option<bool> {
        self.row().3
    }

    pub fn from_number(n: u16) -> Option<Code> {
        CODES.iter().find(|r| r.1 == n).map(|r| r.0)
    }

    pub fn failure(self) -> Failure {
        Failure { code: self.number(), name: self.name().to_string(), permanent: self.permanent().unwrap_or(false), message: None, details: None }
    }

    pub fn failure_with(self, message: impl Into<String>, details: Option<serde_json::Value>) -> Failure {
        Failure { message: Some(message.into()), details, ..self.failure() }
    }

    pub fn leave(self) -> LeaveBody {
        LeaveBody { code: self.number(), name: self.name().to_string(), message: None, details: None }
    }

    pub fn catalog(self) -> String {
        format!("CP-CODE-{}", self.name().replace('_', "-"))
    }

    pub fn http_status(self) -> u16 {
        match self.number() {
            2001..=2004 | 2008 => 401,
            2006 => 403,
            3001 => 404,
            3009 => 409,
            5001 => 503,
            5002 => 502,
            n if n >= 5000 => 500,
            n if (4000..5000).contains(&n) => 429,
            n if (3000..4000).contains(&n) => 409,
            _ => 400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_one_space() {
        let mut seen = std::collections::HashSet::new();
        for (c, n, name, _) in CODES {
            assert!(seen.insert(*n));
            assert_eq!(Code::from_number(*n), Some(*c));
            assert_eq!(c.name(), *name);
            assert!(!RETIRED.contains(n));
        }
        assert_eq!(CODES.len(), 39);
        assert_eq!(Code::ProtocolError.permanent(), Some(true));
        assert_eq!(Code::FlowTimeout.permanent(), None);
        assert_eq!(Code::RoomNotFound.failure().code, 3001);
    }
}
