// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SES-TOKEN-ISSUE · 정:RULE-OPS-TOKEN · 연:SES.http.TOKEN · 연:RULE-SES-TOKEN-CLAIMS · 연:RULE-SES-TOKEN-ACCOUNT-ALLOW · 연:RULE-SES-TOKEN-NO-FLOOR-POLICY · 연:DEV-SES-TOKEN-PERMISSION-REQ · 연:RULE-SES-TOKEN-AXES-SEPARATE · model: claude-opus-5-5
//! `POST /auth/token` — 계정 목록으로 자격을 보고(종류 · 투명 허용) 그 계정 비밀로 서명한다. 발급 경로는 이것 하나다(운영 토큰은 발급하지 않는다).

use common::system::ApiKey;
use common::token::{sign, UserClaims};
use oxsig::{Code, Failure, TokenReq, TokenRes};

pub const KNOWN_TYPES: &[u8] = &[0, 1, 2];

pub fn issue(accounts: &[ApiKey], req: &TokenReq, ttl_secs: u32, metadata_max: u32, now_s: u64) -> Result<TokenRes, Failure> {
    let acct = accounts.iter().find(|a| a.key == req.api_key && a.secret == req.api_secret).ok_or_else(|| Code::InvalidApiKey.failure())?;
    let _ = acct.name.as_deref();
    if req.user_id.is_empty() {
        return Err(Code::MissingField.failure_with("user_id", None));
    }
    let pt = req.participant_type.unwrap_or(0);
    if !KNOWN_TYPES.contains(&pt) {
        return Err(Code::InvalidPayload.failure_with(format!("participant_type {pt}"), None));
    }
    if !acct.participant_types.contains(&pt) {
        return Err(Code::ClaimNotAllowed.failure_with(format!("participant_type {pt}"), None));
    }
    let hidden = req.hidden.unwrap_or(false);
    if hidden && !acct.hidden_allowed {
        return Err(Code::ClaimNotAllowed.failure_with("hidden", None));
    }
    if let Some(m) = &req.metadata {
        let len = serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0);
        if metadata_max > 0 && len > metadata_max as usize {
            return Err(Code::InvalidPayload.failure_with(format!("metadata {len} bytes > {metadata_max}"), None));
        }
    }
    let claims = UserClaims {
        sub: Some(req.user_id.clone()),
        user_id: None,
        participant_type: pt,
        hidden,
        metadata: req.metadata.clone(),
        permission: req.permission,
        iat: now_s,
        exp: now_s + u64::from(ttl_secs),
    };
    let token = sign(&claims, &acct.secret).ok_or_else(|| Code::InternalError.failure())?;
    Ok(TokenRes { token, expires_in: ttl_secs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct() -> ApiKey {
        ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0], hidden_allowed: false, ops_allowed: false, hook_allowed: false }
    }

    fn req() -> TokenReq {
        TokenReq { api_key: "k".into(), api_secret: "s".into(), user_id: "u1".into(), participant_type: None, hidden: None, metadata: None, permission: None }
    }

    #[test]
    fn account_allow_and_limits() {
        assert!(issue(&[acct()], &req(), 60, 2048, 1).is_ok());
        let bad = TokenReq { api_secret: "x".into(), ..req() };
        assert_eq!(issue(&[acct()], &bad, 60, 2048, 1).map_err(|f| f.code), Err(2004), "SV-CODE-2001-2005 계정 비밀 불일치");
        let rec = TokenReq { participant_type: Some(1), ..req() };
        assert_eq!(issue(&[acct()], &rec, 60, 2048, 1).map_err(|f| f.code), Err(2005), "SV-CODE-2001-2005 허용 밖 종류");
        let hid = TokenReq { hidden: Some(true), ..req() };
        assert_eq!(issue(&[acct()], &hid, 60, 2048, 1).map_err(|f| f.code), Err(2005), "SV-CODE-2001-2005 투명 미허용");
        let unk = TokenReq { participant_type: Some(9), ..req() };
        assert_eq!(issue(&[acct()], &unk, 60, 2048, 1).map_err(|f| f.code), Err(1002));
        let big = TokenReq { metadata: Some(serde_json::Value::String("x".repeat(300))), ..req() };
        assert_eq!(issue(&[acct()], &big, 60, 256, 1).map_err(|f| f.code), Err(1002));
    }
}
