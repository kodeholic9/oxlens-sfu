// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 연:ERR.type.Failure · 연:RULE-ERR-UNKNOWN-BY-LEAD · 연:RULE-ERR-ONE-NUMBER-ONE-MEANING · 연:DEV-ERR-LEAVE-PERMANENT-NULL · 연:SES.fact.LeaveReasons · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! 실패 — 번호 공간 하나(응답 실패 · `LEAVE` 사유 · HTTP 실패 body 가 같은 표를 쓴다).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrCode {
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

const TABLE: [(ErrCode, u16, &str, Option<bool>); 39] = [
    (ErrCode::UnknownOp, 1001, "UNKNOWN_OP", Some(true)),
    (ErrCode::InvalidPayload, 1002, "INVALID_PAYLOAD", Some(true)),
    (ErrCode::MissingField, 1003, "MISSING_FIELD", Some(true)),
    (ErrCode::VersionMismatch, 1004, "VERSION_MISMATCH", Some(true)),
    (ErrCode::CodecRequired, 1005, "CODEC_REQUIRED", Some(true)),
    (ErrCode::CodecMismatch, 1006, "CODEC_MISMATCH", Some(true)),
    (ErrCode::FieldConflict, 1007, "FIELD_CONFLICT", Some(true)),
    (ErrCode::ProtocolError, 1008, "PROTOCOL_ERROR", Some(true)),
    (ErrCode::NotBound, 2001, "NOT_BOUND", Some(false)),
    (ErrCode::TokenInvalid, 2002, "TOKEN_INVALID", Some(true)),
    (ErrCode::TokenExpired, 2003, "TOKEN_EXPIRED", Some(false)),
    (ErrCode::InvalidApiKey, 2004, "INVALID_API_KEY", Some(true)),
    (ErrCode::ClaimNotAllowed, 2005, "CLAIM_NOT_ALLOWED", Some(true)),
    (ErrCode::NotAuthorized, 2006, "NOT_AUTHORIZED", Some(true)),
    (ErrCode::SessionNotFound, 2008, "SESSION_NOT_FOUND", Some(false)),
    (ErrCode::SessionRevoked, 2009, "SESSION_REVOKED", None),
    (ErrCode::DuplicateSession, 2010, "DUPLICATE_SESSION", None),
    (ErrCode::RoomNotFound, 3001, "ROOM_NOT_FOUND", Some(false)),
    (ErrCode::NotInRoom, 3002, "NOT_IN_ROOM", Some(false)),
    (ErrCode::RoomNotEmpty, 3004, "ROOM_NOT_EMPTY", Some(false)),
    (ErrCode::TrackNotFound, 3005, "TRACK_NOT_FOUND", Some(false)),
    (ErrCode::TrackOpUnsupported, 3006, "TRACK_OP_UNSUPPORTED", Some(true)),
    (ErrCode::SsrcCollision, 3008, "SSRC_COLLISION", Some(true)),
    (ErrCode::PreconditionFailed, 3009, "PRECONDITION_FAILED", Some(false)),
    (ErrCode::PeerNotFound, 3010, "PEER_NOT_FOUND", Some(false)),
    (ErrCode::NotPubRoom, 3011, "NOT_PUB_ROOM", Some(false)),
    (ErrCode::RoomFull, 4001, "ROOM_FULL", Some(false)),
    (ErrCode::TrackLimit, 4002, "TRACK_LIMIT", Some(false)),
    (ErrCode::QuotaExceeded, 4003, "QUOTA_EXCEEDED", Some(false)),
    (ErrCode::ListenLimit, 4004, "LISTEN_LIMIT", Some(false)),
    (ErrCode::MidLimit, 4005, "MID_LIMIT", Some(false)),
    (ErrCode::FlowTimeout, 4006, "FLOW_TIMEOUT", None),
    (ErrCode::FlowOverflow, 4007, "FLOW_OVERFLOW", None),
    (ErrCode::HeartbeatTimeout, 4008, "HEARTBEAT_TIMEOUT", None),
    (ErrCode::Overloaded, 4009, "OVERLOADED", Some(false)),
    (ErrCode::SfuUnavailable, 5001, "SFU_UNAVAILABLE", Some(false)),
    (ErrCode::SfuError, 5002, "SFU_ERROR", Some(false)),
    (ErrCode::InternalError, 5003, "INTERNAL_ERROR", Some(false)),
    (ErrCode::ServerShutdown, 5004, "SERVER_SHUTDOWN", None),
];

impl ErrCode {
    fn row(self) -> (u16, &'static str, Option<bool>) {
        TABLE
            .iter()
            .find(|r| r.0 == self)
            .map(|r| (r.1, r.2, r.3))
            .unwrap_or((5003, "INTERNAL_ERROR", Some(false)))
    }

    pub fn code(self) -> u16 {
        self.row().0
    }

    pub fn name(self) -> &'static str {
        self.row().1
    }

    pub fn permanent(self) -> Option<bool> {
        self.row().2
    }

    pub fn from_code(code: u16) -> Option<ErrCode> {
        TABLE.iter().find(|r| r.1 == code).map(|r| r.0)
    }

    pub fn all() -> impl Iterator<Item = ErrCode> {
        TABLE.iter().map(|r| r.0)
    }

    pub fn failure(self) -> Failure {
        Failure::new(self)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub code: u16,
    pub name: String,
    pub permanent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl Failure {
    pub fn new(code: ErrCode) -> Failure {
        Failure {
            code: code.code(),
            name: code.name().to_string(),
            permanent: code.permanent().unwrap_or(false),
            message: None,
            details: None,
        }
    }

    pub fn msg(mut self, m: impl Into<String>) -> Failure {
        self.message = Some(m.into());
        self
    }

    pub fn details(mut self, d: Value) -> Failure {
        self.details = Some(d);
        self
    }

    pub fn err_code(&self) -> Option<ErrCode> {
        ErrCode::from_code(self.code)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaveBody {
    pub code: u16,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl LeaveBody {
    pub fn new(code: ErrCode) -> LeaveBody {
        LeaveBody { code: code.code(), name: code.name().to_string(), message: None, details: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_one_space() {
        let mut seen = std::collections::BTreeSet::new();
        for c in ErrCode::all() {
            assert!(seen.insert(c.code()), "번호 겹침 {}", c.code());
            assert_eq!(ErrCode::from_code(c.code()), Some(c));
        }
        for retired in [2007u16, 3003, 3007] {
            assert_eq!(ErrCode::from_code(retired), None);
        }
    }

    #[test]
    fn leave_only_codes_have_no_permanent() {
        for c in [ErrCode::FlowTimeout, ErrCode::FlowOverflow, ErrCode::HeartbeatTimeout, ErrCode::ServerShutdown] {
            assert_eq!(c.permanent(), None);
        }
        assert_eq!(ErrCode::ProtocolError.permanent(), Some(true));
        let v = serde_json::to_value(LeaveBody::new(ErrCode::DuplicateSession)).unwrap_or_default();
        assert!(v.get("permanent").is_none());
    }
}
