// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 연:RULE-TRN-UNKNOWN-JSON-FIELD · 연:RULE-TRN-CLOSE · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! body 읽기 — 형으로 읽고, 실패를 번호로 가른다(JSON 아님 · 필수 칸 없음 · 형식 오류).

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::code::Code;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyError {
    NotJson(String),
    Missing(String),
    Invalid(String),
}

impl BodyError {
    pub fn code(&self) -> Code {
        match self {
            BodyError::NotJson(_) => Code::ProtocolError,
            BodyError::Missing(_) => Code::MissingField,
            BodyError::Invalid(_) => Code::InvalidPayload,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            BodyError::NotJson(m) | BodyError::Missing(m) | BodyError::Invalid(m) => m,
        }
    }
}

pub fn json_value(body: &[u8]) -> Result<serde_json::Value, BodyError> {
    let b: &[u8] = if body.is_empty() { b"{}" } else { body };
    serde_json::from_slice(b).map_err(|e| BodyError::NotJson(e.to_string()))
}

pub fn body<T: DeserializeOwned>(raw: &[u8]) -> Result<T, BodyError> {
    let v = json_value(raw)?;
    typed(v)
}

pub fn typed<T: DeserializeOwned>(v: serde_json::Value) -> Result<T, BodyError> {
    serde_json::from_value(v).map_err(|e| {
        let m = e.to_string();
        if m.starts_with("missing field") { BodyError::Missing(m) } else { BodyError::Invalid(m) }
    })
}

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{BindReq, ReadyReq};

    #[test]
    fn classifies_failures() {
        assert!(matches!(body::<BindReq>(b"not json"), Err(BodyError::NotJson(_))));
        assert_eq!(body::<ReadyReq>(br#"{"room_id":"r1"}"#).map_err(|e| e.code()), Err(Code::MissingField));
        assert_eq!(body::<ReadyReq>(br#"{"room_id":1,"arev":1}"#).map_err(|e| e.code()), Err(Code::InvalidPayload));
        assert!(body::<ReadyReq>(br#"{"room_id":"r","arev":2,"extra":true}"#).is_ok());
        assert!(body::<crate::body::HeartbeatReq>(b"").is_ok());
    }
}
