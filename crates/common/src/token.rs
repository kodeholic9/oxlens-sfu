// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-TOKEN · 정:CAND-SES-TOKEN-CLAIM-NAMES · 연:SES.http.TOKEN · 연:RULE-SES-TOKEN-CLAIMS · 연:RULE-SES-TOKEN-NO-EXPIRY · model: claude-opus-5-5
//! 토큰 — HS256 JWT. A 평면(`sub` = user_id · 종류 · 투명 · metadata · permission)과 운영(`iss` · `ops` · `sub` 없음) 두 형.

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use oxsig::Permission;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
    pub iat: u64,
    pub exp: u64,
}

impl UserClaims {
    pub fn user(&self) -> Option<&str> {
        self.sub.as_deref().or(self.user_id.as_deref()).filter(|u| !u.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    Invalid,
    Expired,
    UnknownIssuer,
}

fn validation(check_exp: bool) -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    v.validate_exp = check_exp;
    v.required_spec_claims = ["exp".to_string()].into_iter().collect();
    v
}

pub fn sign<T: Serialize>(claims: &T, secret: &str) -> Option<String> {
    encode(&Header::new(Algorithm::HS256), claims, &EncodingKey::from_secret(secret.as_bytes())).ok()
}

pub fn verify_user(token: &str, secrets: &[&str], now_s: u64) -> Result<UserClaims, TokenError> {
    for s in secrets {
        if let Ok(d) = decode::<UserClaims>(token, &DecodingKey::from_secret(s.as_bytes()), &validation(false)) {
            if d.claims.exp <= now_s {
                return Err(TokenError::Expired);
            }
            if d.claims.user().is_none() {
                return Err(TokenError::Invalid);
            }
            return Ok(d.claims);
        }
    }
    Err(TokenError::Invalid)
}

pub fn peek_issuer(token: &str) -> Option<String> {
    let mut v = validation(false);
    v.insecure_disable_signature_validation();
    decode::<OpsClaims>(token, &DecodingKey::from_secret(b""), &v).ok().map(|d| d.claims.iss)
}

pub fn verify_ops(token: &str, secret: &str, now_s: u64) -> Result<OpsClaims, TokenError> {
    let d = decode::<OpsClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation(false)).map_err(|_| TokenError::Invalid)?;
    if d.claims.exp <= now_s {
        return Err(TokenError::Expired);
    }
    Ok(d.claims)
}

pub fn now_s() -> u64 {
    crate::time::wall_ms() / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_token_two_secrets_and_expiry() {
        let c = UserClaims { sub: Some("u1".into()), user_id: None, participant_type: 0, hidden: false, metadata: None, permission: None, iat: 10, exp: 100 };
        let t = sign(&c, "acct").expect("sign");
        assert_eq!(verify_user(&t, &["other", "acct"], 50).map(|c| c.sub), Ok(Some("u1".into())));
        assert_eq!(verify_user(&t, &["acct"], 100), Err(TokenError::Expired));
        assert_eq!(verify_user(&t, &["nope"], 50), Err(TokenError::Invalid));
    }

    #[test]
    fn ops_token() {
        let c = OpsClaims { iss: "k".into(), ops: true, sub: None, iat: 1, exp: 99 };
        let t = sign(&c, "s").expect("sign");
        assert_eq!(peek_issuer(&t).as_deref(), Some("k"));
        assert!(verify_ops(&t, "s", 10).is_ok());
        assert_eq!(verify_ops(&t, "x", 10), Err(TokenError::Invalid));
    }
}
