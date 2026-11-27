// author: kodeholic (powered by Claude)
//! 토큰 발급 — 앱(서버 프로그램)이 hub 에서 API 키로 직접 받는다(연:SES.http.TOKEN · SDK:RULE-PLAT-SERVER 토큰).
//! ★참가자 종류(`participant_type` — 1 녹음 · 2 봇)와 투명(`hidden`)은 앱이 고른다 — SDK 는 정하거나 묶지 않는다(연:RULE-SES-TOKEN-AXES-SEPARATE).
//! 요청 꼴은 생성 wire 의 `TokenReq` 그대로다(손으로 다시 적지 않는다).

use serde_json::Value;

use crate::error::Error;
use crate::generated::ws_wire::TokenReq;
use crate::http;

/// `POST {base}/auth/token` — 받은 토큰 문자열(불투명 — SDK 는 열어 보지 않는다)
pub async fn issue_token(base: &str, req: &TokenReq) -> Result<String, Error> {
    issue_token_with_root(base, req, None).await
}

/// 같은 것 — `https://` hub 가 사설 CA 면 그 뿌리 PEM 을 준다(`Config::tls_root_pem` 과 같은 값 · context 202611/20261108d E2)
pub async fn issue_token_with_root(base: &str, req: &TokenReq, root_pem: Option<&[u8]>) -> Result<String, Error> {
    let body = serde_json::to_value(req).map_err(|e| Error::transport(e.to_string()))?;
    let tls = match root_pem {
        Some(p) => Some(crate::tls::client_config(Some(p))?),
        None => None,
    };
    let r = http::call(base, tls.as_ref(), "POST", "/auth/token", &[], Some(&body)).await?;
    if r.status >= 400 {
        return Err(Error::from_failure(&r.body));
    }
    r.body.get("token").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::transport(format!("토큰 응답에 token 이 없다: {}", r.body)))
}
