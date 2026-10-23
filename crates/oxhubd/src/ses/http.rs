// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SES-TOKEN-ISSUE · 정:IMPL-ROOM-RESYNC · 연:SES.http.TOKEN · 연:RULE-TRN-ENDPOINTS · 연:RULE-TRN-HTTP-AUTH-TWO · 연:RULE-TRN-WS-NO-UPGRADE-AUTH · 연:DEV-TRN-WS-ORIGIN · model: claude-opus-5-5
//! SES 의 HTTP 입구 — `POST {base}/auth/token` · `{base}/ws` 업그레이드(무인증 · `Origin` 보조 방어) · HTTP 두 갈래 인증(Bearer · 세션 헤더).

use std::sync::Arc;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use oxsig::err::{ErrCode, Failure};
use oxsig::ses::{TokenReq, TokenRes};
use serde_json::Value;

use super::shell::Ses;
use super::token;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub user_id: String,
    pub session_id: Option<String>,
}

pub fn status_of(code: ErrCode) -> StatusCode {
    match code {
        ErrCode::InvalidApiKey | ErrCode::TokenInvalid | ErrCode::TokenExpired | ErrCode::SessionNotFound | ErrCode::NotAuthorized => StatusCode::UNAUTHORIZED,
        ErrCode::RoomNotFound | ErrCode::NotInRoom => StatusCode::NOT_FOUND,
        ErrCode::PreconditionFailed => StatusCode::CONFLICT,
        ErrCode::SfuUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrCode::SfuError => StatusCode::BAD_GATEWAY,
        ErrCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        ErrCode::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    }
}

pub fn fail_response(f: Failure) -> Response {
    let st = f.err_code().map(status_of).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (st, Json(f)).into_response()
}

pub fn identify(ses: &Ses, h: &HeaderMap) -> Result<Ident, ErrCode> {
    if let Some(sid) = h.get("x-oxlens-session").and_then(|v| v.to_str().ok()) {
        return match ses.identify_header(sid) {
            Some((user, sid)) => Ok(Ident { user_id: user, session_id: Some(sid) }),
            None => Err(ErrCode::SessionNotFound),
        };
    }
    let bearer = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).ok_or(ErrCode::TokenInvalid)?;
    token::verify(&ses.sys, bearer.trim()).map(|v| Ident { user_id: v.user_id, session_id: None })
}

async fn token_issue(State(ses): State<Arc<Ses>>, body: axum::body::Bytes) -> Response {
    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return fail_response(Failure::new(ErrCode::InvalidPayload).msg(e.to_string())),
    };
    for k in ["api_key", "api_secret", "user_id"] {
        if v.get(k).is_none() {
            return fail_response(Failure::new(ErrCode::MissingField).msg(format!("{k} 없음")));
        }
    }
    let req: TokenReq = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => return fail_response(Failure::new(ErrCode::InvalidPayload).msg(e.to_string())),
    };
    let now_s = common::clock::wall_ms() / 1000;
    match token::issue(&ses.sys, &req, ses.policy.hub.token_ttl_secs, ses.policy.hub.metadata_max_bytes, now_s) {
        Ok((t, ttl)) => {
            tracing::info!(kind = "diag", user = %req.user_id, participant_type = req.participant_type.unwrap_or(0), "token issued");
            (StatusCode::OK, Json(TokenRes { token: t, expires_in: ttl })).into_response()
        }
        Err(c) => fail_response(Failure::new(c)),
    }
}

async fn ws_upgrade(State(ses): State<Arc<Ses>>, headers: HeaderMap, up: WebSocketUpgrade) -> Response {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let host = headers.get("host").and_then(|v| v.to_str().ok());
    if !ses.origin_ok(origin, host) {
        tracing::warn!(kind = "diag", origin = origin.unwrap_or(""), "ws origin rejected");
        return StatusCode::FORBIDDEN.into_response();
    }
    up.max_message_size(oxsig::trn::WS_MAX_FRAME + 1024).on_upgrade(move |ws| ses.run_socket(ws))
}

pub fn router(ses: Arc<Ses>) -> Router {
    Router::new().route("/auth/token", post(token_issue)).route("/ws", get(ws_upgrade)).with_state(ses)
}
