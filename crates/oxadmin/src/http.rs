// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §1 · §2 · §6 · model: claude-opus-5

//! C 평면에 거는 손 — ★**실패를 삼키지 않는다.**
//!
//! ★★**「못 물어봤다」와 「서버가 그렇게 답했다」를 가른다**(운영 §2). 둘을 같게 다루면
//! 망이 끊긴 것을 *"값이 비었다"* 로 읽고 운영자가 엉뚱한 곳을 고친다.

use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// 한 번 부른 결과. ★**상태와 본문을 같이** 든다 — 실패 본문이 §2 형식이라 그것이 사유다.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub body: serde_json::Value,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// 실패 본문이 말하는 코드 — ★**없으면 `None`**(지어내지 않는다).
    pub fn code(&self) -> Option<u64> {
        self.body.get("code").and_then(|v| v.as_u64())
    }
}

/// 운영 토큰을 ★**그 자리에서 서명한다**(운영 §1 — 서버에 발급을 조르지 않는다).
///
/// ★**발급 경로가 없다**(정§16-1-1) — 운영자는 자기 `api_secret` 을 쥔 사람이고,
/// 그 사실이 곧 자격이다. hub 는 검증만 한다.
pub fn sign_ops(api_key: &str, api_secret: &str, now: u64, ttl: u64) -> Result<String, String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    #[derive(serde::Serialize)]
    struct Claims<'a> {
        iss: &'a str,
        ops: bool,
        iat: u64,
        exp: u64,
    }
    encode(
        &Header::new(Algorithm::HS256),
        &Claims { iss: api_key, ops: true, iat: now, exp: now + ttl },
        &EncodingKey::from_secret(api_secret.as_bytes()),
    )
    .map_err(|e| format!("운영 토큰 서명 실패: {e}"))
}

/// 서명의 수명 — ★**짧게 둔다.** 도구가 매 실행 새로 서명하므로 길 이유가 없다.
pub const TOKEN_TTL_S: u64 = 60;

pub struct Conn {
    base: String,
    bearer: Option<String>,
    client: Client<hyper_util::client::legacy::connect::HttpConnector, String>,
}

impl Conn {
    pub fn new(base: String, bearer: Option<String>) -> Self {
        Self {
            base,
            bearer,
            client: Client::builder(TokioExecutor::new()).build_http(),
        }
    }

    pub async fn get(&self, path: &str) -> Result<Reply, String> {
        self.call("GET", path).await
    }

    pub async fn post(&self, path: &str) -> Result<Reply, String> {
        self.call("POST", path).await
    }

    async fn call(&self, method: &str, path: &str) -> Result<Reply, String> {
        use http_body_util::BodyExt;
        let url = format!("{}{path}", self.base);
        let mut req = hyper::Request::builder().method(method).uri(&url);
        if let Some(t) = &self.bearer {
            req = req.header(hyper::header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = req
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(String::new())
            .map_err(|e| format!("요청 조립 실패 {url}: {e}"))?;
        // ★**못 닿은 것은 「없다」가 아니다** — 주소를 같이 적어 옆 hub 를 본 경우를 드러낸다.
        let res = self.client.request(req).await.map_err(|e| format!("{url} 에 못 닿는다: {e}"))?;
        let status = res.status().as_u16();
        let raw = res
            .into_body()
            .collect()
            .await
            .map_err(|e| format!("{url} 본문을 못 읽었다: {e}"))?
            .to_bytes();
        // ★**본문이 JSON 이 아니면 그 사실을 낸다** — 빈 객체로 덮으면 사유가 사라진다.
        let body = if raw.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&raw)
                .map_err(|e| format!("{url} 이 JSON 이 아닌 것을 냈다({status}): {e}"))?
        };
        Ok(Reply { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 서명은_ops_를_켜고_수명을_둔다() {
        // ★`ops` 가 없으면 hub 가 `2006` 으로 막는다(운영 §1) — 도구가 낼 수 있는 유일한 자격이다.
        let t = sign_ops("ox_k_demo", "ox_s_demo", 1000, TOKEN_TTL_S).expect("sign");
        let mid = t.split('.').nth(1).expect("payload");
        let pad = "=".repeat((4 - mid.len() % 4) % 4);
        let raw = base64_url(&format!("{mid}{pad}"));
        let v: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(v["iss"], "ox_k_demo");
        assert_eq!(v["ops"], true);
        assert_eq!(v["exp"].as_u64(), Some(1000 + TOKEN_TTL_S));
    }

    fn base64_url(s: &str) -> Vec<u8> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut bits = 0u32;
        let mut n = 0;
        let mut out = Vec::new();
        for c in s.bytes().filter(|c| *c != b'=') {
            let Some(i) = T.iter().position(|t| *t == c) else { continue };
            bits = (bits << 6) | i as u32;
            n += 6;
            if n >= 8 {
                n -= 8;
                out.push((bits >> n) as u8);
            }
        }
        out
    }

    #[test]
    fn 실패_본문의_코드를_읽는다() {
        // ★없는 것을 `0` 으로 채우지 않는다 — 「코드 0」이라는 사유는 없다.
        let r = Reply { status: 404, body: serde_json::json!({ "code": 3001 }) };
        assert!(!r.ok() && r.code() == Some(3001));
        let r = Reply { status: 500, body: serde_json::Value::Null };
        assert_eq!(r.code(), None);
    }
}
