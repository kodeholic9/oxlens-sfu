// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SES-TOKEN-ISSUE · 정:CAND-SES-TOKEN-CLAIM-NAMES · 정:RULE-OPS-TOKEN · 연:SES.http.TOKEN · 연:RULE-SES-TOKEN-CLAIMS · 연:RULE-SES-TOKEN-ACCOUNT-ALLOW · 연:RULE-SES-TOKEN-NO-FLOOR-POLICY · 연:DEV-SES-TOKEN-PERMISSION-REQ · 운영:RULE-AUTH-THREE · model: claude-opus-5-5
//! 토큰 — A 평면 HS256(계정 `api_secret` 으로 서명 · 검증 비밀 = 계정 비밀 전부 + `jwt_secret`) · 운영 토큰(`iss` 로 계정을 찾아 검증 · 발급 경로 없음).

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use oxsig::code::Code;
use oxsig::msg::TokenReq;
use oxsig::types::Permission;
use serde::{Deserialize, Serialize};

use common::system::ApiKey;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claims {
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

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
    pub iat: u64,
    pub exp: u64,
}

pub fn issue(accounts: &[ApiKey], req: &TokenReq, ttl_secs: u32, metadata_max: u32, now_secs: u64) -> Result<(String, u32), Code> {
    let acct = accounts.iter().find(|a| a.key == req.api_key && a.secret == req.api_secret).ok_or(Code::INVALID_API_KEY)?;
    let pt = req.participant_type.unwrap_or(0);
    if pt > 2 {
        return Err(Code::INVALID_PAYLOAD);
    }
    if !acct.participant_types.contains(&pt) {
        return Err(Code::CLAIM_NOT_ALLOWED);
    }
    let hidden = req.hidden.unwrap_or(false);
    if hidden && !acct.hidden_allowed {
        return Err(Code::CLAIM_NOT_ALLOWED);
    }
    if let Some(m) = &req.metadata
        && metadata_max != 0
        && serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0) > metadata_max as usize
    {
        return Err(Code::INVALID_PAYLOAD);
    }
    if req.user_id.is_empty() {
        return Err(Code::MISSING_FIELD);
    }
    let claims = Claims {
        sub: Some(req.user_id.clone()),
        user_id: None,
        participant_type: pt,
        hidden,
        metadata: req.metadata.clone(),
        permission: Some(req.permission.unwrap_or_default()),
        iat: now_secs,
        exp: now_secs + u64::from(ttl_secs),
    };
    let tok = encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(acct.secret.as_bytes())).map_err(|_| Code::INTERNAL_ERROR)?;
    Ok((tok, ttl_secs))
}

pub fn verify(token: &str, secrets: &[&str]) -> Result<Identity, Code> {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    v.validate_aud = false;
    let mut expired = false;
    for s in secrets {
        match decode::<Claims>(token, &DecodingKey::from_secret(s.as_bytes()), &v) {
            Ok(d) => {
                let c = d.claims;
                let user_id = c.sub.or(c.user_id).filter(|u| !u.is_empty()).ok_or(Code::TOKEN_INVALID)?;
                return Ok(Identity { user_id, participant_type: c.participant_type, hidden: c.hidden, metadata: c.metadata, permission: c.permission.unwrap_or_default() });
            }
            Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => expired = true,
            Err(_) => {}
        }
    }
    Err(if expired { Code::TOKEN_EXPIRED } else { Code::TOKEN_INVALID })
}

pub fn secrets<'a>(accounts: &'a [ApiKey], jwt_secret: Option<&'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = accounts.iter().map(|a| a.secret.as_str()).collect();
    if let Some(s) = jwt_secret {
        v.push(s);
    }
    v
}

pub fn verify_ops(token: &str, accounts: &[ApiKey]) -> Result<String, Code> {
    let mut peek = Validation::new(Algorithm::HS256);
    peek.insecure_disable_signature_validation();
    peek.validate_exp = false;
    peek.required_spec_claims.clear();
    let iss = decode::<OpsClaims>(token, &DecodingKey::from_secret(b""), &peek).map_err(|_| Code::TOKEN_INVALID)?.claims.iss;
    let acct = accounts.iter().find(|a| a.key == iss).ok_or(Code::INVALID_API_KEY)?;
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    let c = match decode::<OpsClaims>(token, &DecodingKey::from_secret(acct.secret.as_bytes()), &v) {
        Ok(d) => d.claims,
        Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => return Err(Code::TOKEN_EXPIRED),
        Err(_) => return Err(Code::TOKEN_INVALID),
    };
    if !c.ops || !acct.ops_allowed {
        return Err(Code::NOT_AUTHORIZED);
    }
    Ok(acct.key.clone())
}


#[cfg(test)]
mod tests {
    use super::*;

    pub fn sign_ops(api_key: &str, api_secret: &str, now_secs: u64, ttl_secs: u64) -> Option<String> {
        let c = OpsClaims { iss: api_key.to_string(), ops: true, iat: now_secs, exp: now_secs + ttl_secs };
        encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(api_secret.as_bytes())).ok()
    }

    fn acct(key: &str, secret: &str, ops: bool, hidden: bool) -> ApiKey {
        ApiKey { key: key.into(), secret: secret.into(), name: None, participant_types: vec![0, 1], hidden_allowed: hidden, ops_allowed: ops, hook_allowed: false }
    }

    fn req(user: &str) -> TokenReq {
        TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: user.into(), participant_type: None, hidden: None, metadata: None, permission: None }
    }

    fn now() -> u64 {
        common::time::wall_secs()
    }

    #[test]
    fn issue_then_verify_with_default_permission() {
        let a = vec![acct("k", "s", false, false)];
        let (t, ttl) = issue(&a, &req("u1"), 3600, 2048, now()).expect("issue");
        assert_eq!(ttl, 3600);
        let id = verify(&t, &secrets(&a, Some("other"))).expect("verify");
        assert_eq!(id.user_id, "u1");
        assert_eq!(id.permission, Permission::default());
        assert!(id.permission.floor_request);
    }

    #[test]
    fn claims_outside_account_are_refused() {
        let a = vec![acct("k", "s", false, false)];
        let mut r = req("u");
        r.hidden = Some(true);
        assert_eq!(issue(&a, &r, 60, 2048, now()), Err(Code::CLAIM_NOT_ALLOWED));
        let mut r = req("u");
        r.participant_type = Some(2);
        assert_eq!(issue(&a, &r, 60, 2048, now()), Err(Code::CLAIM_NOT_ALLOWED));
        let mut r = req("u");
        r.participant_type = Some(9);
        assert_eq!(issue(&a, &r, 60, 2048, now()), Err(Code::INVALID_PAYLOAD));
        let mut r = req("u");
        r.api_secret = "x".into();
        assert_eq!(issue(&a, &r, 60, 2048, now()), Err(Code::INVALID_API_KEY));
        let mut r = req("u");
        r.metadata = Some(serde_json::json!({"n": "x".repeat(300)}));
        assert_eq!(issue(&a, &r, 60, 256, now()), Err(Code::INVALID_PAYLOAD));
    }

    #[test]
    fn expired_and_forged() {
        let a = vec![acct("k", "s", false, false)];
        let (t, _) = issue(&a, &req("u"), 60, 2048, now() - 10_000).expect("issue");
        assert_eq!(verify(&t, &secrets(&a, None)), Err(Code::TOKEN_EXPIRED));
        assert_eq!(verify("x.y.z", &secrets(&a, None)), Err(Code::TOKEN_INVALID));
        let (t, _) = issue(&a, &req("u"), 60, 2048, now()).expect("issue");
        assert_eq!(verify(&t, &["wrong"]), Err(Code::TOKEN_INVALID));
    }

    #[test]
    fn ops_token() {
        let a = vec![acct("k", "s", true, false), acct("n", "t", false, false)];
        let t = sign_ops("k", "s", now(), 60).expect("sign");
        assert_eq!(verify_ops(&t, &a), Ok("k".into()));
        let t = sign_ops("n", "t", now(), 60).expect("sign");
        assert_eq!(verify_ops(&t, &a), Err(Code::NOT_AUTHORIZED));
        let t = sign_ops("zz", "t", now(), 60).expect("sign");
        assert_eq!(verify_ops(&t, &a), Err(Code::INVALID_API_KEY));
        let t = sign_ops("k", "bad", now(), 60).expect("sign");
        assert_eq!(verify_ops(&t, &a), Err(Code::TOKEN_INVALID));
        let t = sign_ops("k", "s", now() - 1000, 60).expect("sign");
        assert_eq!(verify_ops(&t, &a), Err(Code::TOKEN_EXPIRED));
    }
}
