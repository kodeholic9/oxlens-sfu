// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-ERR-ISSUERS · 연:RULE-ERR-FAILURE-FORM · 연:RULE-ERR-UNKNOWN-BY-LEAD · 연:DEV-ERR-LEAVE-PERMANENT-NULL · 연:RULE-ERR-ONE-NUMBER-ONE-MEANING · 연:RULE-ERR-CLOSE-EMPTY · model: claude-opus-5-5
//! 오류 번호 공간 하나 — 실패 응답 body(`Failure`)와 `LEAVE` 사유가 같은 표를 쓴다.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Code(pub u16);

pub const CODES: &[(u16, &str, Option<bool>)] = &[
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

pub const RETIRED: &[u16] = &[2007, 3003, 3007];

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

    fn row(self) -> Option<&'static (u16, &'static str, Option<bool>)> {
        CODES.iter().find(|r| r.0 == self.0)
    }

    pub fn name(self) -> &'static str {
        self.row().map(|r| r.1).unwrap_or("UNKNOWN")
    }

    pub fn permanent(self) -> Option<bool> {
        match self.row() {
            Some(r) => r.2,
            None => Some(self.0 / 1000 == 1),
        }
    }

    pub fn lead(self) -> u16 {
        self.0 / 1000
    }

    pub fn http_status(self) -> u16 {
        match self.0 {
            1000..=1999 => 400,
            2001..=2004 | 2008 => 401,
            2006 => 403,
            3001 => 404,
            3009 => 409,
            3000..=3999 => 409,
            4000..=4999 => 429,
            5001 => 503,
            5002 => 502,
            _ => 500,
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
    pub fn of(code: Code) -> Failure {
        Failure { code: code.0, name: code.name().to_string(), permanent: code.permanent().unwrap_or(false), message: None, details: None }
    }

    pub fn msg(mut self, message: impl Into<String>) -> Failure {
        self.message = Some(message.into());
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> Failure {
        self.details = Some(details);
        self
    }

    pub fn code(&self) -> Code {
        Code(self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_number_one_name() {
        let mut seen = std::collections::BTreeSet::new();
        for (c, n, _) in CODES {
            assert!(seen.insert(*c), "번호 겹침 {c}");
            assert!(!RETIRED.contains(c), "폐기 번호 재사용 {c} {n}");
        }
    }

    #[test]
    fn leave_only_reasons_have_no_permanent() {
        for c in [Code::FLOW_TIMEOUT, Code::FLOW_OVERFLOW, Code::HEARTBEAT_TIMEOUT, Code::SERVER_SHUTDOWN, Code::SESSION_REVOKED, Code::DUPLICATE_SESSION] {
            assert_eq!(c.permanent(), None);
        }
        assert_eq!(Code::PROTOCOL_ERROR.permanent(), Some(true));
    }

    #[test]
    fn unknown_code_falls_back_to_lead() {
        assert_eq!(Code(1099).permanent(), Some(true));
        assert_eq!(Code(3099).permanent(), Some(false));
        assert_eq!(Code(3099).http_status(), 409);
        assert_eq!(Code::SFU_UNAVAILABLE.http_status(), 503);
    }
}
