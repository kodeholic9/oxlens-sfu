// author: kodeholic (powered by Claude)
//! 운영 HTTP 중 클라 규격 안의 것 — 연§5-2 `POST /auth/token`(정§3-4) · `/healthz`.
//! HTTP 실패 body 는 전부 연§4-5 `Failure`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use common::auth;
use common::config::{HubAuth, PolicyConfig, SystemConfig};
use oxsig::{FailCode, Failure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub api_key: String,
    pub api_secret: String,
    pub user_id: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub floor_priority: u8,
    #[serde(default)]
    pub metadata: Option<Value>,
}

fn default_role() -> String {
    auth::ROLE_USER.to_owned()
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct TokenResponse {
    pub token: String,
    pub expires_in: u64,
}

/// 정§3-4 판정 — `2004`(401) · `2005`(400) · `5003`(500).
pub fn issue_token(auth_cfg: &HubAuth, ttl_secs: u64, req: &TokenRequest, now: i64) -> Result<TokenResponse, (StatusCode, Failure)> {
    let Some(account) = auth_cfg.api_keys.iter().find(|k| k.key == req.api_key && k.secret == req.api_secret) else {
        return Err((StatusCode::UNAUTHORIZED, Failure::new(FailCode::InvalidApiKey)));
    };
    if !auth::is_known_role(&req.role) || !account.roles.iter().any(|r| r == &req.role) {
        return Err((StatusCode::BAD_REQUEST, Failure::new(FailCode::InvalidRole).message(format!("role '{}' not allowed", req.role))));
    }
    auth::issue(&auth_cfg.jwt_secret, &req.user_id, &req.role, req.floor_priority, req.metadata.clone(), ttl_secs, now)
        .map(|i| TokenResponse { token: i.token, expires_in: i.expires_in })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Failure::new(FailCode::InternalError).message(e.to_string())))
}

pub struct RestState {
    pub system: SystemConfig,
    pub policy: PolicyConfig,
}

pub async fn token(State(st): State<Arc<RestState>>, Json(req): Json<TokenRequest>) -> axum::response::Response {
    match issue_token(&st.system.hub.auth, u64::from(st.policy.hub.token_ttl_secs), &req, auth::now_unix()) {
        Ok(res) => (StatusCode::OK, Json(res)).into_response(),
        Err((status, failure)) => (status, Json(failure)).into_response(),
    }
}

pub async fn healthz() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::config::ApiKey;

    fn cfg() -> HubAuth {
        HubAuth { jwt_secret: "s".into(), api_keys: vec![ApiKey { key: "k".into(), secret: "p".into(), name: String::new(), roles: vec!["user".into()] }] }
    }
    fn req(key: &str, role: &str) -> TokenRequest {
        TokenRequest { api_key: key.into(), api_secret: "p".into(), user_id: "u1".into(), role: role.into(), floor_priority: 9, metadata: None }
    }

    #[test]
    fn token_judgement() {
        let now = auth::now_unix();
        let ok = issue_token(&cfg(), 3600, &req("k", "user"), now).unwrap();
        assert_eq!(ok.expires_in, 3600);
        assert_eq!(auth::verify("s", &ok.token).unwrap().floor_priority, 9);
        let (st, f) = issue_token(&cfg(), 3600, &req("x", "user"), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::UNAUTHORIZED, 2004));
        let (st, f) = issue_token(&cfg(), 3600, &req("k", "admin"), now).unwrap_err();
        assert_eq!((st, f.code), (StatusCode::BAD_REQUEST, 2005));
    }
}
