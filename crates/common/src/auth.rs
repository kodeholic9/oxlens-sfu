// author: kodeholic (powered by Claude)
//! 토큰 — 연§5-2 `POST /auth/token` · 정§3-4. HS256 JWT, claims `{sub, role, floor_priority, metadata?, iat, exp}`.
//! `floor_priority` 는 발언권 우선순위의 권위(연§11-3) — 앱 백엔드가 서명하므로 클라가 못 바꾼다.

use chrono::Utc;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ROLE_USER: &str = "user";
pub const ROLE_ADMIN: &str = "admin";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    #[serde(default)]
    pub floor_priority: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub iat: i64,
    pub exp: i64,
}

impl Claims {
    pub fn is_admin(&self) -> bool {
        self.role == ROLE_ADMIN
    }
}

pub fn is_known_role(role: &str) -> bool {
    role == ROLE_USER || role == ROLE_ADMIN
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// 연§10-2 `2003` — 새 토큰을 받아 다시.
    Expired,
    /// 연§10-2 `2002` — 위조·손상. permanent.
    Invalid,
}

pub struct Issued {
    pub token: String,
    pub expires_in: u64,
}

pub fn issue(secret: &str, user_id: &str, role: &str, floor_priority: u8, metadata: Option<Value>, ttl_secs: u64, now: i64)
-> Result<Issued, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: user_id.to_owned(),
        role: role.to_owned(),
        floor_priority,
        metadata,
        iat: now,
        exp: now + i64::try_from(ttl_secs).unwrap_or(i64::MAX),
    };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))?;
    Ok(Issued { token, expires_in: ttl_secs })
}

pub fn now_unix() -> i64 {
    Utc::now().timestamp()
}

pub fn verify(secret: &str, token: &str) -> Result<Claims, VerifyError> {
    decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::default())
        .map(|d| d.claims)
        .map_err(|e| match e.kind() {
            ErrorKind::ExpiredSignature => VerifyError::Expired,
            _ => VerifyError::Invalid,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_verify_expired_and_forged() {
        let now = now_unix();
        let t = issue("s", "u1", ROLE_USER, 7, None, 60, now).unwrap();
        let c = verify("s", &t.token).unwrap();
        assert_eq!((c.sub.as_str(), c.role.as_str(), c.floor_priority), ("u1", "user", 7));
        assert_eq!(verify("other", &t.token), Err(VerifyError::Invalid));
        let old = issue("s", "u1", ROLE_USER, 0, None, 1, now - 600).unwrap();
        assert_eq!(verify("s", &old.token), Err(VerifyError::Expired));
    }
}
