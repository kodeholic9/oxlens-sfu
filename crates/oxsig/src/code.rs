// author: kodeholic (powered by Claude)
//! 실패 응답 body — 연§4-5·§10. 판단은 숫자 `code` 로, `name` 은 로그용.
//! 모르는 코드는 앞자리로 판단한다(연§10-2). 비워 둔 번호(2007·3003)는 쓰지 않는다.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum FailCode {
    UnknownOp = 1001,
    InvalidPayload = 1002,
    MissingField = 1003,
    VersionMismatch = 1004,
    CodecRequired = 1005,
    CodecMismatch = 1006,
    FieldConflict = 1007,
    NotBound = 2001,
    TokenInvalid = 2002,
    TokenExpired = 2003,
    InvalidApiKey = 2004,
    InvalidRole = 2005,
    NotAuthorized = 2006,
    SessionNotFound = 2008,
    RoomNotFound = 3001,
    NotInRoom = 3002,
    RoomNotEmpty = 3004,
    TrackNotFound = 3005,
    TrackOpUnsupported = 3006,
    RoomFull = 4001,
    TrackLimit = 4002,
    QuotaExceeded = 4003,
    ListenLimit = 4004,
    MidLimit = 4005,
    SfuUnavailable = 5001,
    SfuError = 5002,
    InternalError = 5003,
}

/// 연§10-2 앞자리 — "무엇을 고쳐야 하나".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// 1xxx 요청이 틀렸다(클라 버그, 전부 permanent)
    Bug,
    /// 2xxx 신원·권한
    Auth,
    /// 3xxx 지금 상태에서 안 된다
    State,
    /// 4xxx 한계(전부 permanent 아님)
    Limit,
    /// 5xxx 서버 사정(전부 permanent 아님)
    Server,
    Unknown,
}

impl Class {
    pub fn of(code: u16) -> Class {
        match code / 1000 {
            1 => Class::Bug,
            2 => Class::Auth,
            3 => Class::State,
            4 => Class::Limit,
            5 => Class::Server,
            _ => Class::Unknown,
        }
    }
}

/// 연§10-2 — 번호는 재사용하지 않고, 없앨 때는 비워 둔다.
pub const RETIRED: [u16; 2] = [2007, 3003];

impl FailCode {
    pub const ALL: [FailCode; 27] = [
        FailCode::UnknownOp, FailCode::InvalidPayload, FailCode::MissingField, FailCode::VersionMismatch,
        FailCode::CodecRequired, FailCode::CodecMismatch, FailCode::FieldConflict, FailCode::NotBound,
        FailCode::TokenInvalid, FailCode::TokenExpired, FailCode::InvalidApiKey, FailCode::InvalidRole,
        FailCode::NotAuthorized, FailCode::SessionNotFound, FailCode::RoomNotFound, FailCode::NotInRoom,
        FailCode::RoomNotEmpty, FailCode::TrackNotFound, FailCode::TrackOpUnsupported, FailCode::RoomFull,
        FailCode::TrackLimit, FailCode::QuotaExceeded, FailCode::ListenLimit, FailCode::MidLimit,
        FailCode::SfuUnavailable, FailCode::SfuError, FailCode::InternalError,
    ];

    pub fn code(self) -> u16 {
        self as u16
    }

    pub fn from_code(code: u16) -> Option<FailCode> {
        FailCode::ALL.iter().copied().find(|c| c.code() == code)
    }

    pub fn class(self) -> Class {
        Class::of(self.code())
    }

    pub fn name(self) -> &'static str {
        match self {
            FailCode::UnknownOp => "UNKNOWN_OP",
            FailCode::InvalidPayload => "INVALID_PAYLOAD",
            FailCode::MissingField => "MISSING_FIELD",
            FailCode::VersionMismatch => "VERSION_MISMATCH",
            FailCode::CodecRequired => "CODEC_REQUIRED",
            FailCode::CodecMismatch => "CODEC_MISMATCH",
            FailCode::FieldConflict => "FIELD_CONFLICT",
            FailCode::NotBound => "NOT_BOUND",
            FailCode::TokenInvalid => "TOKEN_INVALID",
            FailCode::TokenExpired => "TOKEN_EXPIRED",
            FailCode::InvalidApiKey => "INVALID_API_KEY",
            FailCode::InvalidRole => "INVALID_ROLE",
            FailCode::NotAuthorized => "NOT_AUTHORIZED",
            FailCode::SessionNotFound => "SESSION_NOT_FOUND",
            FailCode::RoomNotFound => "ROOM_NOT_FOUND",
            FailCode::NotInRoom => "NOT_IN_ROOM",
            FailCode::RoomNotEmpty => "ROOM_NOT_EMPTY",
            FailCode::TrackNotFound => "TRACK_NOT_FOUND",
            FailCode::TrackOpUnsupported => "TRACK_OP_UNSUPPORTED",
            FailCode::RoomFull => "ROOM_FULL",
            FailCode::TrackLimit => "TRACK_LIMIT",
            FailCode::QuotaExceeded => "QUOTA_EXCEEDED",
            FailCode::ListenLimit => "LISTEN_LIMIT",
            FailCode::MidLimit => "MID_LIMIT",
            FailCode::SfuUnavailable => "SFU_UNAVAILABLE",
            FailCode::SfuError => "SFU_ERROR",
            FailCode::InternalError => "INTERNAL_ERROR",
        }
    }

    /// 연§10-1 `permanent` — 같은 요청을 다시 보내도 같다.
    pub fn permanent(self) -> bool {
        match self.class() {
            Class::Bug => true,
            Class::Limit | Class::Server | Class::Unknown => false,
            Class::Auth => matches!(
                self,
                FailCode::TokenInvalid | FailCode::InvalidApiKey | FailCode::InvalidRole | FailCode::NotAuthorized
            ),
            Class::State => matches!(self, FailCode::TrackOpUnsupported),
        }
    }
}

/// 연§4-5 실패 body. `permanent` 가 오면 앞자리보다 우선한다(연§10-2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub fn new(code: FailCode) -> Self {
        Self { code: code.code(), name: code.name().to_owned(), permanent: code.permanent(), message: None, details: None }
    }
    pub fn message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }
    pub fn details(mut self, d: Value) -> Self {
        self.details = Some(d);
        self
    }
    pub fn class(&self) -> Class {
        Class::of(self.code)
    }
    /// 연§7-0-1 — 재시도할 수 있나: 1xxx 아님 ∧ permanent 아님.
    pub fn retryable(&self) -> bool {
        !self.permanent && self.class() != Class::Bug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete_and_retired_are_absent() {
        assert_eq!(FailCode::ALL.len(), 27);
        for r in RETIRED {
            assert_eq!(FailCode::from_code(r), None);
        }
        for c in FailCode::ALL {
            assert_eq!(FailCode::from_code(c.code()), Some(c));
        }
    }

    #[test]
    fn permanent_follows_spec_10_2() {
        assert!(FailCode::UnknownOp.permanent());
        assert!(FailCode::FieldConflict.permanent());
        assert!(!FailCode::TokenExpired.permanent());
        assert!(FailCode::TokenInvalid.permanent());
        assert!(!FailCode::NotBound.permanent());
        assert!(!FailCode::SessionNotFound.permanent());
        assert!(FailCode::TrackOpUnsupported.permanent());
        assert!(!FailCode::RoomNotFound.permanent());
        assert!(!FailCode::MidLimit.permanent());
        assert!(!FailCode::InternalError.permanent());
    }

    #[test]
    fn unknown_code_judged_by_leading_digit() {
        let f = Failure { code: 4999, name: "NEW".into(), permanent: false, message: None, details: None };
        assert_eq!(f.class(), Class::Limit);
        assert!(f.retryable());
        let bug = Failure { code: 1999, name: "NEW".into(), permanent: true, message: None, details: None };
        assert!(!bug.retryable());
    }

    #[test]
    fn failure_json_shape() {
        let j = serde_json::to_value(Failure::new(FailCode::RoomNotFound)).unwrap();
        assert_eq!(j["code"], 3001);
        assert_eq!(j["name"], "ROOM_NOT_FOUND");
        assert_eq!(j["permanent"], false);
        assert!(j.get("message").is_none());
    }
}
