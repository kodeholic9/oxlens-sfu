// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:ERR.type.Failure · 연:RULE-ERR-FAILURE-FORM · 연:RULE-ERR-UNKNOWN-BY-LEAD · 연:DEV-ERR-LEAVE-PERMANENT-NULL · 연:SES.op.LEAVE · 연:RULE-ERR-NO-RETRY-AFTER · 연:RULE-ERR-ONE-NUMBER-ONE-MEANING · model: claude-opus-5-5
//! 오류 번호 공간 하나 — 실패 응답(`flags=10`)과 `LEAVE` 사유가 같은 번호를 쓴다.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

pub const RETIRED: &[u16] = &[2007, 3003, 3007];

impl Code {
    pub const ALL: &'static [Code] = &[
        Code::UnknownOp,
        Code::InvalidPayload,
        Code::MissingField,
        Code::VersionMismatch,
        Code::CodecRequired,
        Code::CodecMismatch,
        Code::FieldConflict,
        Code::ProtocolError,
        Code::NotBound,
        Code::TokenInvalid,
        Code::TokenExpired,
        Code::InvalidApiKey,
        Code::ClaimNotAllowed,
        Code::NotAuthorized,
        Code::SessionNotFound,
        Code::SessionRevoked,
        Code::DuplicateSession,
        Code::RoomNotFound,
        Code::NotInRoom,
        Code::RoomNotEmpty,
        Code::TrackNotFound,
        Code::TrackOpUnsupported,
        Code::SsrcCollision,
        Code::PreconditionFailed,
        Code::PeerNotFound,
        Code::NotPubRoom,
        Code::RoomFull,
        Code::TrackLimit,
        Code::QuotaExceeded,
        Code::ListenLimit,
        Code::MidLimit,
        Code::FlowTimeout,
        Code::FlowOverflow,
        Code::HeartbeatTimeout,
        Code::Overloaded,
        Code::SfuUnavailable,
        Code::SfuError,
        Code::InternalError,
        Code::ServerShutdown,
    ];

    pub fn num(self) -> u16 {
        self.row().0
    }

    pub fn name(self) -> &'static str {
        self.row().1
    }

    pub fn permanent(self) -> Option<bool> {
        self.row().2
    }

    fn row(self) -> (u16, &'static str, Option<bool>) {
        match self {
            Code::UnknownOp => (1001, "UNKNOWN_OP", Some(true)),
            Code::InvalidPayload => (1002, "INVALID_PAYLOAD", Some(true)),
            Code::MissingField => (1003, "MISSING_FIELD", Some(true)),
            Code::VersionMismatch => (1004, "VERSION_MISMATCH", Some(true)),
            Code::CodecRequired => (1005, "CODEC_REQUIRED", Some(true)),
            Code::CodecMismatch => (1006, "CODEC_MISMATCH", Some(true)),
            Code::FieldConflict => (1007, "FIELD_CONFLICT", Some(true)),
            Code::ProtocolError => (1008, "PROTOCOL_ERROR", Some(true)),
            Code::NotBound => (2001, "NOT_BOUND", Some(false)),
            Code::TokenInvalid => (2002, "TOKEN_INVALID", Some(true)),
            Code::TokenExpired => (2003, "TOKEN_EXPIRED", Some(false)),
            Code::InvalidApiKey => (2004, "INVALID_API_KEY", Some(true)),
            Code::ClaimNotAllowed => (2005, "CLAIM_NOT_ALLOWED", Some(true)),
            Code::NotAuthorized => (2006, "NOT_AUTHORIZED", Some(true)),
            Code::SessionNotFound => (2008, "SESSION_NOT_FOUND", Some(false)),
            Code::SessionRevoked => (2009, "SESSION_REVOKED", None),
            Code::DuplicateSession => (2010, "DUPLICATE_SESSION", None),
            Code::RoomNotFound => (3001, "ROOM_NOT_FOUND", Some(false)),
            Code::NotInRoom => (3002, "NOT_IN_ROOM", Some(false)),
            Code::RoomNotEmpty => (3004, "ROOM_NOT_EMPTY", Some(false)),
            Code::TrackNotFound => (3005, "TRACK_NOT_FOUND", Some(false)),
            Code::TrackOpUnsupported => (3006, "TRACK_OP_UNSUPPORTED", Some(true)),
            Code::SsrcCollision => (3008, "SSRC_COLLISION", Some(true)),
            Code::PreconditionFailed => (3009, "PRECONDITION_FAILED", Some(false)),
            Code::PeerNotFound => (3010, "PEER_NOT_FOUND", Some(false)),
            Code::NotPubRoom => (3011, "NOT_PUB_ROOM", Some(false)),
            Code::RoomFull => (4001, "ROOM_FULL", Some(false)),
            Code::TrackLimit => (4002, "TRACK_LIMIT", Some(false)),
            Code::QuotaExceeded => (4003, "QUOTA_EXCEEDED", Some(false)),
            Code::ListenLimit => (4004, "LISTEN_LIMIT", Some(false)),
            Code::MidLimit => (4005, "MID_LIMIT", Some(false)),
            Code::FlowTimeout => (4006, "FLOW_TIMEOUT", None),
            Code::FlowOverflow => (4007, "FLOW_OVERFLOW", None),
            Code::HeartbeatTimeout => (4008, "HEARTBEAT_TIMEOUT", None),
            Code::Overloaded => (4009, "OVERLOADED", Some(false)),
            Code::SfuUnavailable => (5001, "SFU_UNAVAILABLE", Some(false)),
            Code::SfuError => (5002, "SFU_ERROR", Some(false)),
            Code::InternalError => (5003, "INTERNAL_ERROR", Some(false)),
            Code::ServerShutdown => (5004, "SERVER_SHUTDOWN", None),
        }
    }

    pub fn from_num(n: u16) -> Option<Code> {
        Code::ALL.iter().copied().find(|c| c.num() == n)
    }

    pub fn failure(self) -> Failure {
        Failure {
            code: self.num(),
            name: self.name().to_string(),
            permanent: self.permanent().unwrap_or(false),
            message: None,
            details: None,
        }
    }

    pub fn leave(self) -> crate::msg::LeaveBody {
        crate::msg::LeaveBody { code: self.num(), name: self.name().to_string(), message: None, details: None }
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.num(), self.name())
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
    pub fn with_message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }

    pub fn with_details(mut self, d: serde_json::Value) -> Self {
        self.details = Some(d);
        self
    }

    pub fn code(&self) -> Option<Code> {
        Code::from_num(self.code)
    }
}

pub fn body<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Result<T, Failure> {
    serde_json::from_value(v.clone()).map_err(|e| {
        let m = e.to_string();
        let code = if m.starts_with("missing field") { Code::MissingField } else { Code::InvalidPayload };
        code.failure().with_message(m)
    })
}

pub fn field<T: serde::de::DeserializeOwned>(v: &serde_json::Value, k: &str) -> Result<Option<T>, Failure> {
    match v.get(k) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(x) => serde_json::from_value(x.clone()).map(Some).map_err(|_| Code::InvalidPayload.failure().with_message(k.to_string())),
    }
}

pub fn need<T: serde::de::DeserializeOwned>(v: &serde_json::Value, k: &str) -> Result<T, Failure> {
    field(v, k)?.ok_or_else(|| Code::MissingField.failure().with_message(k.to_string()))
}

impl From<Code> for Failure {
    fn from(c: Code) -> Self {
        c.failure()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_unique_not_retired() {
        let mut seen = std::collections::HashSet::new();
        for c in Code::ALL {
            assert!(seen.insert(c.num()));
            assert!(!RETIRED.contains(&c.num()));
        }
    }

    #[test]
    fn leave_only_have_no_permanent() {
        for c in [Code::FlowTimeout, Code::FlowOverflow, Code::HeartbeatTimeout, Code::ServerShutdown] {
            assert_eq!(c.permanent(), None);
        }
        assert_eq!(Code::ProtocolError.permanent(), Some(true));
    }
}
