// author: kodeholic (powered by Claude)
//! 실패 하나 — wire 의 실패 응답(연:ERR.type.Failure)이면 그 code · name · permanent, SDK 가 낸 것이면 code 0 + 이름.
//! 분류(category)는 SDK 정의서 오류 장의 갈래다(SDK:IMPL-ERR-TYPE) — 앱이 다시 부를지 가른다.
//! wire 실패의 `details` 는 그대로 나르고, SDK 가 wire 실패를 감싸 올리면(두 번째 3008 → `NEGOTIATION_SSRC_COLLISION`) 원 실패는 `cause` 에 둔다.

use std::fmt;

use serde_json::Value;

/// 실패 갈래(SDK:IMPL-ERR-TYPE) — `auth` 토큰 · `state` 방 · `limit` 상한 · `negotiation` 연결 모드 · `closed` 끝남 · `bug` 클라 잘못 · `transport` 망
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Auth,
    State,
    Limit,
    Negotiation,
    Closed,
    Bug,
    Transport,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Auth => "auth",
            Category::State => "state",
            Category::Limit => "limit",
            Category::Negotiation => "negotiation",
            Category::Closed => "closed",
            Category::Bug => "bug",
            Category::Transport => "transport",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub code: u16,
    pub name: String,
    pub category: Category,
    pub permanent: bool,
    pub message: Option<String>,
    pub details: Option<Value>,
    pub cause: Option<Box<Error>>,
}

impl Error {
    pub(crate) fn coded(code: u16, name: &str, category: Category, permanent: bool) -> Error {
        Error { code, name: name.to_string(), category, permanent, message: None, details: None, cause: None }
    }

    pub(crate) fn sdk(name: &str, category: Category) -> Error {
        Error::coded(0, name, category, false)
    }

    pub(crate) fn negotiation(stage: &str, cause: Error) -> Error {
        Error { cause: Some(Box::new(cause)), ..Error::coded(0, &format!("NEGOTIATION_{stage}"), Category::Negotiation, true) }
    }

    pub(crate) fn closed(name: &str) -> Error {
        Error::sdk(name, Category::Closed)
    }

    pub(crate) fn transport(why: impl Into<String>) -> Error {
        Error { message: Some(why.into()), ..Error::sdk("TRANSPORT", Category::Transport) }
    }

    /// 실패 응답 body(연:ERR.type.Failure) → 실패 — 갈래는 번호대로(연:ERR.fact.Codes)
    pub(crate) fn from_failure(body: &Value) -> Error {
        let code = body.get("code").and_then(Value::as_u64).unwrap_or(5003) as u16;
        let name = body.get("name").and_then(Value::as_str).unwrap_or("INTERNAL_ERROR").to_string();
        let permanent = body.get("permanent").and_then(Value::as_bool).unwrap_or(false);
        let message = body.get("message").and_then(Value::as_str).map(str::to_string);
        let details = body.get("details").filter(|d| d.is_object()).cloned();
        let category = match code {
            2002..=2005 => Category::Auth,
            1001..=1008 => Category::Bug,
            3001..=3011 => Category::State,
            4001..=4009 => Category::Limit,
            _ => Category::Transport,
        };
        Error { code, name, category, permanent, message, details, cause: None }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code, self.name)?;
        if let Some(m) = &self.message {
            write!(f, " — {m}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|c| c as &(dyn std::error::Error + 'static))
    }
}
