// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-SES-TOKEN-ISSUE 정:RULE-OPS-TOKEN 연:SES.http.TOKEN 연:RULE-SES-TOKEN-CLAIMS 연:RULE-SES-TOKEN-ACCOUNT-ALLOW · model: claude-opus-5-5
//! 토큰 — 앱 사용자 토큰(계정 비밀로 서명 · 발급 계정은 싣지 않는다)과 운영 토큰(`{iss, ops, iat, exp}` — 서버가 발급하지 않는다).

use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, errors::ErrorKind};
use oxsig::Code;
use oxsig::wire::Permission;
use serde::{Deserialize, Serialize};

use common::system::System;

pub const ALL_ALLOWED: Permission = Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true };

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
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

#[derive(Debug, Clone, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
}

fn validation() -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    v.required_spec_claims.clear();
    v.validate_exp = true;
    v
}

pub fn issue(secret: &str, claims: &Claims) -> Result<String, Code> {
    jsonwebtoken::encode(&Header::new(Algorithm::HS256), claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|_| Code::INTERNAL_ERROR)
}

pub fn verify_user(system: &System, token: &str) -> Result<Claims, Code> {
    let mut secrets: Vec<&str> = system.hub.auth.api_keys.iter().map(|a| a.secret.as_str()).collect();
    if let Some(s) = system.hub.auth.jwt_secret.as_deref() {
        secrets.push(s);
    }
    let mut expired = false;
    for s in secrets {
        match jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(s.as_bytes()), &validation()) {
            Ok(d) => return Ok(d.claims),
            Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => expired = true,
            Err(_) => {}
        }
    }
    Err(if expired { Code::TOKEN_EXPIRED } else { Code::TOKEN_INVALID })
}

pub fn verify_ops(system: &System, token: &str) -> Result<(), Code> {
    let mut peek = validation();
    peek.insecure_disable_signature_validation();
    peek.validate_exp = false;
    let unverified =
        jsonwebtoken::decode::<OpsClaims>(token, &DecodingKey::from_secret(b""), &peek).map_err(|_| Code::TOKEN_INVALID)?;
    let acct = system.account(&unverified.claims.iss).ok_or(Code::INVALID_API_KEY)?;
    let claims = match jsonwebtoken::decode::<OpsClaims>(token, &DecodingKey::from_secret(acct.secret.as_bytes()), &validation()) {
        Ok(d) => d.claims,
        Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => return Err(Code::TOKEN_EXPIRED),
        Err(_) => return Err(Code::TOKEN_INVALID),
    };
    if !claims.ops || !acct.ops_allowed {
        return Err(Code::NOT_AUTHORIZED);
    }
    Ok(())
}

pub fn now_secs() -> u64 {
    common::ident::now_ms() / 1000
}
