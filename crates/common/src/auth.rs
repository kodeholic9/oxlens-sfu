// author: kodeholic (powered by Claude)
//! 토큰 — 연§5-2 `POST /auth/token` · 정§3-4. HS256 JWT, claims `{sub, participant_type, hidden, metadata?, iat, exp}`.
//! 토큰은 *"누구냐·무엇이냐"* 만 나른다 — 권한 클레임도 발언권 정책도 없다(우선순위는 요청 TLV 0, 연§11-3).
//! 운영 자격은 별도다(정§16-1-1 운영 토큰 — 발급하지 않는다).

use chrono::Utc;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 연§5-2 `participant_type` — `0` 사람 · `1` 녹화 · `2` 봇. 표시용이고 서버가 이 값으로 가르는 것은 없다.
pub const PT_USER: u8 = 0;
pub const PT_RECORDER: u8 = 1;
pub const PT_BOT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    pub iat: i64,
    pub exp: i64,
}

pub fn is_known_participant_type(pt: u8) -> bool {
    matches!(pt, PT_USER | PT_RECORDER | PT_BOT)
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

pub fn issue(secret: &str, user_id: &str, participant_type: u8, hidden: bool, metadata: Option<Value>, ttl_secs: u64, now: i64)
-> Result<Issued, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: user_id.to_owned(),
        participant_type,
        hidden,
        metadata,
        iat: now,
        exp: now + i64::try_from(ttl_secs).unwrap_or(i64::MAX),
    };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))?;
    Ok(Issued { token, expires_in: ttl_secs })
}

/// 정§16-1-1 운영 토큰 — 운영자가 자기 `api_secret` 으로 직접 서명한다. hub 에 발급 경로가 없다.
/// `sub` 이 없다 — 사람의 신원이 아니라 **계정 자격**이라 A 평면 토큰과 형이 다르다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
    pub iat: i64,
    pub exp: i64,
}

/// 서명 검증 **전에** `iss`(= `api_key`)만 읽는다 — 그 계정의 비밀을 찾아야 검증할 수 있다.
/// 여기서 읽은 값은 신뢰하지 않는다. 계정을 고르는 열쇠일 뿐이고, 판정은 `verify_ops` 가 한다.
pub fn ops_issuer(token: &str) -> Option<String> {
    let mut v = Validation::default();
    v.insecure_disable_signature_validation();
    v.validate_exp = false;
    v.required_spec_claims.clear();
    decode::<OpsClaims>(token, &DecodingKey::from_secret(b""), &v).ok().map(|d| d.claims.iss)
}

/// 운영 도구(CLI·시험)가 쓰는 서명 함수. ★서버는 이것을 HTTP 로 **노출하지 않는다**(§16-1-1) —
/// 운영 토큰은 비밀키를 가진 쪽이 자기 손으로 만든다.
pub fn sign_ops(api_secret: &str, api_key: &str, ttl_secs: u64, now: i64)
-> Result<String, jsonwebtoken::errors::Error> {
    let claims = OpsClaims { iss: api_key.to_owned(), ops: true, iat: now, exp: now + i64::try_from(ttl_secs).unwrap_or(i64::MAX) };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(api_secret.as_bytes()))
}

pub fn verify_ops(secret: &str, token: &str) -> Result<OpsClaims, VerifyError> {
    decode::<OpsClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::default())
        .map(|d| d.claims)
        .map_err(|e| match e.kind() {
            ErrorKind::ExpiredSignature => VerifyError::Expired,
            _ => VerifyError::Invalid,
        })
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
        let t = issue("s", "u1", PT_RECORDER, true, None, 60, now).unwrap();
        let c = verify("s", &t.token).unwrap();
        assert_eq!((c.sub.as_str(), c.participant_type, c.hidden), ("u1", PT_RECORDER, true));
        assert_eq!(verify("other", &t.token), Err(VerifyError::Invalid));
        let old = issue("s", "u1", PT_USER, false, None, 1, now - 600).unwrap();
        assert_eq!(verify("s", &old.token), Err(VerifyError::Expired));
    }
}
