// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-TOKEN · 연:SES.http.TOKEN · 연:RULE-SES-TOKEN-ACCOUNT-ALLOW · 연:RULE-SES-TOKEN-CLAIMS · 정:CAND-SES-TOKEN-CLAIM-NAMES · 연:RULE-SES-TOKEN-AXES-SEPARATE · 연:RULE-SES-TOKEN-NO-FLOOR-POLICY · 연:RULE-SES-TOKEN-BY-BACKEND · model: claude-opus-5-5
//! 토큰 — 발급(계정 비밀로 HS256 · 계정 허용 검사 · `metadata` 상한) · A 평면 검증(계정 비밀 전부 + `jwt_secret`) · 운영 토큰 검증(`iss` 계정 · `ops`).

use common::envelope::Claims;
use common::system::ApiKey;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode, errors::ErrorKind};
use oxsig::msg::{Permission, TokenReq, TokenRes};
use oxsig::{Code, Failure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Accounts {
    pub keys: Vec<ApiKey>,
    pub jwt_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsClaims {
    pub iss: String,
    #[serde(default)]
    pub ops: bool,
    #[serde(default)]
    pub iat: u64,
    pub exp: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub claims: Claims,
}

fn validation() -> Validation {
    let mut v = Validation::new(Algorithm::HS256);
    v.leeway = 0;
    v.validate_aud = false;
    v.set_required_spec_claims(&["exp"]);
    v
}

fn secs_now() -> u64 {
    common::now_ms() / 1000
}

impl Accounts {
    pub fn find(&self, key: &str) -> Option<&ApiKey> {
        self.keys.iter().find(|k| k.key == key)
    }

    pub fn issue(&self, req: &TokenReq, ttl_secs: u32, metadata_max: u32) -> Result<TokenRes, Failure> {
        let Some(acc) = self.find(&req.api_key).filter(|a| a.secret == req.api_secret) else {
            return Err(Code::InvalidApiKey.failure());
        };
        let pt = req.participant_type.unwrap_or(0);
        if pt > 2 {
            return Err(Code::InvalidPayload.failure().with_message("unknown participant_type"));
        }
        if !acc.participant_types.contains(&pt) {
            return Err(Code::ClaimNotAllowed.failure().with_message("participant_type not allowed"));
        }
        let hidden = req.hidden.unwrap_or(false);
        if hidden && !acc.hidden_allowed {
            return Err(Code::ClaimNotAllowed.failure().with_message("hidden not allowed"));
        }
        if let Some(m) = &req.metadata {
            let n = serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0);
            if metadata_max > 0 && n as u32 > metadata_max {
                return Err(Code::InvalidPayload.failure().with_message("metadata too large"));
            }
        }
        let now = secs_now();
        let c = UserClaims {
            sub: Some(req.user_id.clone()),
            user_id: None,
            participant_type: pt,
            hidden,
            metadata: req.metadata.clone(),
            permission: req.permission,
            iat: now,
            exp: now + ttl_secs as u64,
        };
        let token = encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(acc.secret.as_bytes()))
            .map_err(|_| Code::InternalError.failure())?;
        Ok(TokenRes { token, expires_in: ttl_secs })
    }

    fn secrets(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.keys.iter().map(|k| k.secret.as_str()).collect();
        if let Some(s) = &self.jwt_secret {
            v.push(s.as_str());
        }
        v
    }

    pub fn verify_user(&self, token: &str) -> Result<Identity, Code> {
        let mut expired = false;
        for s in self.secrets() {
            match decode::<UserClaims>(token, &DecodingKey::from_secret(s.as_bytes()), &validation()) {
                Ok(d) => {
                    let c = d.claims;
                    let Some(user) = c.sub.clone().or(c.user_id.clone()).filter(|u| !u.is_empty()) else {
                        return Err(Code::TokenInvalid);
                    };
                    return Ok(Identity {
                        user_id: user,
                        claims: Claims {
                            participant_type: c.participant_type,
                            hidden: c.hidden,
                            metadata: c.metadata,
                            permission: c.permission,
                        },
                    });
                }
                Err(e) if matches!(e.kind(), ErrorKind::ExpiredSignature) => expired = true,
                Err(_) => {}
            }
        }
        Err(if expired { Code::TokenExpired } else { Code::TokenInvalid })
    }

    pub fn verify_ops(&self, token: &str) -> Result<String, Code> {
        let mut peek = Validation::new(Algorithm::HS256);
        peek.insecure_disable_signature_validation();
        peek.validate_exp = false;
        peek.set_required_spec_claims::<&str>(&[]);
        let iss = decode::<OpsClaims>(token, &DecodingKey::from_secret(b"x"), &peek).map_err(|_| Code::TokenInvalid)?.claims.iss;
        let acc = self.find(&iss).ok_or(Code::InvalidApiKey)?;
        let d = decode::<OpsClaims>(token, &DecodingKey::from_secret(acc.secret.as_bytes()), &validation()).map_err(|e| {
            if matches!(e.kind(), ErrorKind::ExpiredSignature) { Code::TokenExpired } else { Code::TokenInvalid }
        })?;
        if !d.claims.ops || !acc.ops_allowed {
            return Err(Code::NotAuthorized);
        }
        Ok(iss)
    }

    pub fn sign_ops(&self, iss: &str, ttl_secs: u64) -> Option<String> {
        let acc = self.find(iss)?;
        let now = secs_now();
        let c = OpsClaims { iss: iss.to_string(), ops: true, iat: now, exp: now + ttl_secs };
        encode(&Header::new(Algorithm::HS256), &c, &EncodingKey::from_secret(acc.secret.as_bytes())).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts() -> Accounts {
        Accounts {
            keys: vec![ApiKey {
                key: "k".into(),
                secret: "s".into(),
                name: None,
                participant_types: vec![0, 1],
                hidden_allowed: false,
                ops_allowed: true,
                hook_allowed: false,
            }],
            jwt_secret: None,
        }
    }

    fn req(pt: Option<u8>, hidden: Option<bool>) -> TokenReq {
        TokenReq {
            api_key: "k".into(),
            api_secret: "s".into(),
            user_id: "u1".into(),
            participant_type: pt,
            hidden,
            metadata: None,
            permission: None,
        }
    }

    #[test]
    fn issue_and_verify() {
        let a = accounts();
        let t = a.issue(&req(Some(1), None), 60, 2048).unwrap();
        let id = a.verify_user(&t.token).unwrap();
        assert_eq!(id.user_id, "u1");
        assert_eq!(id.claims.participant_type, 1);
        assert_eq!(a.issue(&req(Some(2), None), 60, 2048).unwrap_err().code, 2005);
        assert_eq!(a.issue(&req(None, Some(true)), 60, 2048).unwrap_err().code, 2005);
        assert_eq!(a.verify_user("garbage"), Err(Code::TokenInvalid));
    }

    #[test]
    fn ops_token() {
        let a = accounts();
        let t = a.sign_ops("k", 60).unwrap();
        assert_eq!(a.verify_ops(&t), Ok("k".to_string()));
        let user = a.issue(&req(None, None), 60, 2048).unwrap().token;
        assert!(a.verify_ops(&user).is_err());
    }
}
