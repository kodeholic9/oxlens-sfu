// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 연:ERR.type.Failure 연:ERR.lead 연:RULE-ERR-UNKNOWN-BY-LEAD 연:RULE-ERR-FLOOR-OUTSIDE 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! 오류 코드 번호 공간 하나 — 번호 · 이름 · `permanent` · 실패 응답 body.

use serde::{Deserialize, Serialize};

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

    pub fn num(self) -> u16 {
        self.row().1
    }

    pub fn name(self) -> &'static str {
        self.row().2
    }

    pub fn permanent(self) -> Option<bool> {
        self.row().3
    }

    pub fn from_num(n: u16) -> Option<Code> {
        CODES.iter().find(|r| r.1 == n).map(|r| r.0)
    }

    pub fn catalog_id(self) -> String {
        format!("CP-CODE-{}", self.name().replace('_', "-"))
    }

    pub fn failure(self) -> Failure {
        Failure { code: self.num(), name: self.name().to_string(), permanent: self.permanent().unwrap_or(false), message: None, details: None }
    }

    pub fn failure_msg(self, message: impl Into<String>) -> Failure {
        Failure { message: Some(message.into()), ..self.failure() }
    }

    pub fn http_status(self) -> u16 {
        let n = self.num();
        match self {
            Code::NotBound | Code::TokenInvalid | Code::TokenExpired | Code::InvalidApiKey | Code::SessionNotFound => 401,
            Code::NotAuthorized => 403,
            Code::RoomNotFound => 404,
            Code::PreconditionFailed => 409,
            Code::SfuUnavailable => 503,
            Code::SfuError => 502,
            _ => match n / 1000 {
                1 => 400,
                3 => 409,
                4 => 429,
                5 => 500,
                _ => 400,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub code: u16,
    pub name: String,
    pub permanent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Failure {
    pub fn code(&self) -> Option<Code> {
        Code::from_num(self.code)
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_total_and_unique() {
        let mut nums: Vec<u16> = CODES.iter().map(|r| r.1).collect();
        nums.sort_unstable();
        nums.dedup();
        assert_eq!(nums.len(), CODES.len());
        for r in CODES {
            assert_eq!(Code::from_num(r.1), Some(r.0));
            assert!(!RETIRED.contains(&r.1));
        }
    }

    #[test]
    fn leave_only_codes_have_null_permanent() {
        for c in [Code::FlowTimeout, Code::FlowOverflow, Code::HeartbeatTimeout, Code::ServerShutdown, Code::SessionRevoked, Code::DuplicateSession] {
            assert_eq!(c.permanent(), None, "연:RULE-ERR-UNKNOWN-BY-LEAD");
        }
        assert_eq!(Code::ProtocolError.permanent(), Some(true));
    }

    #[test]
    fn http_map_follows_impl_err_issuers() {
        assert_eq!(Code::InvalidPayload.http_status(), 400);
        assert_eq!(Code::SessionNotFound.http_status(), 401);
        assert_eq!(Code::NotAuthorized.http_status(), 403);
        assert_eq!(Code::RoomNotFound.http_status(), 404);
        assert_eq!(Code::NotInRoom.http_status(), 409);
        assert_eq!(Code::QuotaExceeded.http_status(), 429);
        assert_eq!(Code::SfuUnavailable.http_status(), 503);
        assert_eq!(Code::SfuError.http_status(), 502);
        assert_eq!(Code::InternalError.http_status(), 500);
    }
}
