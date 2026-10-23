// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-TOKEN · 연:SES.http.TOKEN · 연:RULE-SES-TOKEN-CLAIMS · 연:RULE-SES-TOKEN-ACCOUNT-ALLOW · 연:RULE-SES-TOKEN-NO-FLOOR-POLICY · model: claude-opus-5-5
//! 토큰 — 앱 사용자 토큰 발급(계정 비밀로 HS256 서명 · 발급 계정은 싣지 않는다) · 검증(계정 비밀들과 배포 공통 비밀) · 운영 토큰 검증.

use common::system::SystemFile;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use oxsig::err::ErrCode;
use oxsig::room::Permission;
use oxsig::ses::{OpsClaims, TokenReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub iat: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    #[serde(default)]
    pub participant_type: u8,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<Permission>,
}

impl UserClaims {
    pub fn user(&self) -> Option<&str> {
        self.sub.as_deref().or(self.user_id.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<Value>,
    pub permission: Permission,
}

pub fn issue(sys: &SystemFile, req: &TokenReq, ttl_secs: u32, metadata_max: u32, now_s: u64) -> Result<(String, u32), ErrCode> {
    let acct = sys.account(&req.api_key).ok_or(ErrCode::InvalidApiKey)?;
    if acct.secret != req.api_secret {
        return Err(ErrCode::InvalidApiKey);
    }
    let pt = req.participant_type.unwrap_or(0);
    if pt > 2 {
        return Err(ErrCode::InvalidPayload);
    }
    if !acct.participant_types.contains(&pt) {
        return Err(ErrCode::ClaimNotAllowed);
    }
    let hidden = req.hidden.unwrap_or(false);
    if hidden && !acct.hidden_allowed {
        return Err(ErrCode::ClaimNotAllowed);
    }
    if let Some(m) = &req.metadata {
        let n = serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0) as u32;
        if metadata_max != 0 && n > metadata_max {
            return Err(ErrCode::InvalidPayload);
        }
    }
    if req.user_id.is_empty() {
        return Err(ErrCode::MissingField);
    }
    let claims = UserClaims {
        sub: Some(req.user_id.clone()),
        user_id: None,
        iat: now_s,
        exp: Some(now_s + ttl_secs as u64),
        participant_type: pt,
        hidden,
        metadata: req.metadata.clone(),
        permission: req.permission.filter(|p| !p.is_default()),
    };
    let t = encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(acct.secret.as_bytes())).map_err(|_| ErrCode::InternalError)?;
    Ok((t, ttl_secs))
}

fn secrets(sys: &SystemFile) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = sys.hub.auth.api_keys.iter().map(|a| a.secret.as_bytes().to_vec()).collect();
    if let Some(s) = &sys.hub.auth.jwt_secret {
        v.push(s.as_bytes().to_vec());
    }
    v
}

fn validation() -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.required_spec_claims.clear();
    v.validate_exp = true;
    v.leeway = 0;
    v
}

pub fn verify(sys: &SystemFile, token: &str) -> Result<Verified, ErrCode> {
    let mut expired = false;
    for s in secrets(sys) {
        match decode::<UserClaims>(token, &DecodingKey::from_secret(&s), &validation()) {
            Ok(d) => {
                let c = d.claims;
                let Some(user) = c.user().map(str::to_string) else { return Err(ErrCode::TokenInvalid) };
                return Ok(Verified {
                    user_id: user,
                    participant_type: c.participant_type,
                    hidden: c.hidden,
                    metadata: c.metadata,
                    permission: c.permission.unwrap_or_default(),
                });
            }
            Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => expired = true,
            Err(_) => {}
        }
    }
    Err(if expired { ErrCode::TokenExpired } else { ErrCode::TokenInvalid })
}

pub fn verify_ops(sys: &SystemFile, token: &str) -> Result<String, ErrCode> {
    let iss = jsonwebtoken::decode_header(token).ok().and(insecure_iss(token)).ok_or(ErrCode::TokenInvalid)?;
    let acct = sys.account(&iss).ok_or(ErrCode::InvalidApiKey)?;
    let d = decode::<OpsClaims>(token, &DecodingKey::from_secret(acct.secret.as_bytes()), &validation()).map_err(|e| {
        if matches!(e.kind(), ErrorKind::ExpiredSignature) {
            ErrCode::TokenExpired
        } else {
            ErrCode::TokenInvalid
        }
    })?;
    if !d.claims.ops || !acct.ops_allowed {
        return Err(ErrCode::NotAuthorized);
    }
    Ok(iss)
}

fn insecure_iss(token: &str) -> Option<String> {
    let mid = token.split('.').nth(1)?;
    let raw = b64url(mid)?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    v.get("iss").and_then(Value::as_str).map(str::to_string)
}

fn b64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
pub fn sign_ops(api_key: &str, secret: &str, now_s: u64, ttl: u64) -> Option<String> {
    let c = OpsClaims { iss: api_key.to_string(), ops: true, iat: now_s, exp: now_s + ttl };
    encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(secret.as_bytes())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> SystemFile {
        SystemFile::parse(
            r#"
[hub]
listen = "127.0.0.1:1974"
[hub.auth]
jwt_secret = "j"
[[hub.auth.api_keys]]
key = "k1"
secret = "s1"
participant_types = [0, 1]
hidden_allowed = false
ops_allowed = true
[zenoh]
namespace = "ox"
"#,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    }

    fn req(pt: Option<u8>, hidden: Option<bool>) -> TokenReq {
        TokenReq { api_key: "k1".into(), api_secret: "s1".into(), user_id: "u1".into(), participant_type: pt, hidden, metadata: None, permission: None }
    }

    #[test]
    fn issue_checks_account_allowance() {
        let s = sys();
        let now = common::clock::wall_ms() / 1000;
        let (t, ttl) = issue(&s, &req(Some(1), None), 3600, 2048, now).unwrap_or_default();
        assert_eq!(ttl, 3600);
        let v = verify(&s, &t).map(|v| (v.user_id, v.participant_type));
        assert_eq!(v, Ok(("u1".to_string(), 1)));
        assert_eq!(issue(&s, &req(Some(2), None), 3600, 2048, now), Err(ErrCode::ClaimNotAllowed));
        assert_eq!(issue(&s, &req(None, Some(true)), 3600, 2048, now), Err(ErrCode::ClaimNotAllowed));
        assert_eq!(issue(&s, &req(Some(9), None), 3600, 2048, now), Err(ErrCode::InvalidPayload));
        let mut bad = req(None, None);
        bad.api_secret = "x".into();
        assert_eq!(issue(&s, &bad, 3600, 2048, now), Err(ErrCode::InvalidApiKey));
    }

    #[test]
    fn expired_and_forged_differ() {
        let s = sys();
        let (t, _) = issue(&s, &req(None, None), 60, 2048, 1_000).unwrap_or_default();
        assert_eq!(verify(&s, &t), Err(ErrCode::TokenExpired));
        assert_eq!(verify(&s, "a.b.c"), Err(ErrCode::TokenInvalid));
    }

    #[test]
    fn ops_token_is_self_signed() {
        let s = sys();
        let now = common::clock::wall_ms() / 1000;
        let t = sign_ops("k1", "s1", now, 60).unwrap_or_default();
        assert_eq!(verify_ops(&s, &t), Ok("k1".to_string()));
        let t2 = sign_ops("nope", "s1", now, 60).unwrap_or_default();
        assert_eq!(verify_ops(&s, &t2), Err(ErrCode::InvalidApiKey));
        let (user, _) = issue(&s, &req(None, None), 60, 2048, now).unwrap_or_default();
        assert!(verify_ops(&s, &user).is_err());
    }
}
