// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SES-TOKEN-ISSUE 정:RULE-OPS-TOKEN 정:CAND-SES-TOKEN-CLAIM-NAMES 연:RULE-SES-TOKEN-CLAIMS 연:RULE-SES-TOKEN-ACCOUNT-ALLOW 연:DEV-SES-TOKEN-PERMISSION-REQ 운영:RULE-AUTH-THREE · model: claude-opus-5-5
//! HS256 토큰 — A 평면 사용자 토큰(`sub` · 종류 · 투명 · 권한 · 메타) 서명 · 검증 · 운영 토큰(`iss` · `ops`) 검증.

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use oxsig::Code;
use oxsig::msg::Permission;
use serde::{Deserialize, Serialize};

use crate::system::System;

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
    pub permission: Option<Permission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
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
    pub iat: u64,
    pub exp: u64,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn sign<T: Serialize>(claims: &T, secret: &str) -> Result<String, Code> {
    encode(&Header::new(Algorithm::HS256), claims, &EncodingKey::from_secret(secret.as_bytes())).map_err(|_| Code::InternalError)
}

fn validation() -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    v.required_spec_claims.clear();
    v.validate_exp = true;
    v
}

fn map_err(e: &jsonwebtoken::errors::Error) -> Code {
    match e.kind() {
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => Code::TokenExpired,
        _ => Code::TokenInvalid,
    }
}

pub fn verify_user(token: &str, sys: &System) -> Result<UserClaims, Code> {
    let mut secrets: Vec<&str> = sys.hub.auth.api_keys.iter().map(|a| a.secret.as_str()).collect();
    if let Some(j) = sys.hub.auth.jwt_secret.as_deref() {
        secrets.push(j);
    }
    let mut expired = false;
    for s in secrets {
        match decode::<UserClaims>(token, &DecodingKey::from_secret(s.as_bytes()), &validation()) {
            Ok(d) => {
                if d.claims.user().is_none() {
                    return Err(Code::TokenInvalid);
                }
                return Ok(d.claims);
            }
            Err(e) if map_err(&e) == Code::TokenExpired => expired = true,
            Err(_) => {}
        }
    }
    Err(if expired { Code::TokenExpired } else { Code::TokenInvalid })
}

pub fn verify_ops(token: &str, sys: &System) -> Result<OpsClaims, Code> {
    let iss = jsonwebtoken::dangerous_insecure_decode_iss(token).ok_or(Code::TokenInvalid)?;
    let acct = sys.account(&iss).ok_or(Code::InvalidApiKey)?;
    let d = decode::<OpsClaims>(token, &DecodingKey::from_secret(acct.secret.as_bytes()), &validation()).map_err(|e| map_err(&e))?;
    if !d.claims.ops || !acct.ops_allowed {
        return Err(Code::NotAuthorized);
    }
    Ok(d.claims)
}

mod jsonwebtoken {
    pub use ::jsonwebtoken::*;

    pub fn dangerous_insecure_decode_iss(token: &str) -> Option<String> {
        let mut v = ::jsonwebtoken::Validation::new(::jsonwebtoken::Algorithm::HS256);
        v.insecure_disable_signature_validation();
        v.validate_exp = false;
        v.required_spec_claims.clear();
        let d = ::jsonwebtoken::decode::<serde_json::Value>(token, &::jsonwebtoken::DecodingKey::from_secret(b""), &v).ok()?;
        d.claims.get("iss").and_then(|x| x.as_str()).map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> System {
        System::parse(
            r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "j"
[[hub.auth.api_keys]]
key = "k1"
secret = "s1"
ops_allowed = true
[[hub.auth.api_keys]]
key = "k2"
secret = "s2"
[zenoh]
mode = "router"
namespace = "ox"
"#,
        )
        .expect("sys")
    }

    fn user(exp: u64) -> UserClaims {
        UserClaims { sub: Some("u1".into()), user_id: None, participant_type: 0, hidden: false, permission: None, metadata: None, iat: now_secs(), exp }
    }

    #[test]
    fn user_token_any_account_or_shared_secret() {
        let s = sys();
        let t = sign(&user(now_secs() + 60), "s2").expect("sign");
        assert_eq!(verify_user(&t, &s).expect("ok").user(), Some("u1"));
        let t = sign(&user(now_secs() + 60), "j").expect("sign");
        assert!(verify_user(&t, &s).is_ok());
        let t = sign(&user(now_secs() + 60), "nope").expect("sign");
        assert_eq!(verify_user(&t, &s), Err(Code::TokenInvalid));
        let t = sign(&user(now_secs().saturating_sub(10)), "s1").expect("sign");
        assert_eq!(verify_user(&t, &s), Err(Code::TokenExpired));
    }

    #[test]
    fn ops_token_rules() {
        let s = sys();
        let mk = |iss: &str, ops: bool, secret: &str| sign(&OpsClaims { iss: iss.into(), ops, iat: now_secs(), exp: now_secs() + 60 }, secret).expect("sign");
        assert!(verify_ops(&mk("k1", true, "s1"), &s).is_ok());
        assert_eq!(verify_ops(&mk("zz", true, "s1"), &s), Err(Code::InvalidApiKey));
        assert_eq!(verify_ops(&mk("k1", true, "bad"), &s), Err(Code::TokenInvalid));
        assert_eq!(verify_ops(&mk("k1", false, "s1"), &s), Err(Code::NotAuthorized));
        assert_eq!(verify_ops(&mk("k2", true, "s2"), &s), Err(Code::NotAuthorized));
    }
}
