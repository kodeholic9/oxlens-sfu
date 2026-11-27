// author: kodeholic (powered by Claude)
//! 작은 HTTP/1.1 클라 — 토큰 발급(연:SES.http.TOKEN)과 방 재동기(연:ROOM.http.GET_ROOM)만 쓴다.
//! 요청마다 연결 하나(`Connection: close`) · `http://` 또는 `https://`(context 202611/20261108d E2 — TLS 는 `tls` 모듈).

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls::ClientConfig;

use crate::error::Error;

/// 앱이 준 `base`(연:RULE-TRN-BASE-GIVEN)를 가른 것
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Base {
    pub secure: bool,
    /// host:port
    pub auth: String,
    /// 경로 앞머리(끝 `/` 없음)
    pub prefix: String,
}

impl Base {
    /// WS 주소 — `ws://` · `wss://` + 경로 + `/ws`
    pub(crate) fn ws_url(&self) -> String {
        format!("{}://{}{}/ws", if self.secure { "wss" } else { "ws" }, self.auth, self.prefix)
    }
}

/// `http://host:port/경로` · `https://host:port/경로` — 포트가 없으면 80 · 443
pub(crate) fn split_base(base: &str) -> Result<Base, Error> {
    let (secure, rest) = if let Some(r) = base.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = base.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(Error::transport(format!("base 는 http:// 또는 https:// 로 시작한다: {base}")));
    };
    let (auth, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if auth.is_empty() {
        return Err(Error::transport(format!("base 에 host 가 없다: {base}")));
    }
    let has_port = auth.rsplit_once(':').is_some_and(|(_, p)| p.chars().all(|c| c.is_ascii_digit())) && !auth.ends_with(']');
    let auth = if has_port { auth.to_string() } else { format!("{auth}:{}", if secure { 443 } else { 80 }) };
    Ok(Base { secure, auth, prefix: path.trim_end_matches('/').to_string() })
}

pub(crate) struct Reply {
    pub status: u16,
    pub body: Value,
}

/// 요청 하나 — `tls` 는 `https://` 일 때 쓸 설정(없으면 webpki 뿌리로 짓는다)
pub(crate) async fn call(base: &str, tls: Option<&Arc<ClientConfig>>, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Result<Reply, Error> {
    let b = split_base(base)?;
    let (auth, prefix) = (b.auth.clone(), b.prefix.clone());
    let full = format!("{prefix}{path}");
    let own;
    let tls = match (b.secure, tls) {
        (false, _) => None,
        (true, Some(t)) => Some(t),
        (true, None) => {
            own = crate::tls::client_config(None)?;
            Some(&own)
        }
    };
    let mut s = crate::tls::dial(&auth, tls).await?;
    let payload = body.map(|b| serde_json::to_vec(b).unwrap_or_default()).unwrap_or_default();
    let mut req = format!("{method} {full} HTTP/1.1\r\nHost: {auth}\r\nConnection: close\r\nContent-Length: {}\r\n", payload.len());
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.map_err(|e| Error::transport(e.to_string()))?;
    s.write_all(&payload).await.map_err(|e| Error::transport(e.to_string()))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.map_err(|e| Error::transport(e.to_string()))?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::transport(format!("{method} {full}: 머리 끝이 없다")))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut rest = raw[split + 4..].to_vec();
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| Error::transport(format!("{method} {full}: 상태 줄 {head}")))?;
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        rest = dechunk(&rest);
    }
    let body = if rest.is_empty() { Value::Null } else { serde_json::from_slice(&rest).unwrap_or(Value::String(String::from_utf8_lossy(&rest).to_string())) };
    Ok(Reply { status, body })
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16).unwrap_or(0);
        if n == 0 || b.len() < nl + 2 + n {
            break;
        }
        out.extend_from_slice(&b[nl + 2..nl + 2 + n]);
        b = &b[(nl + 4 + n).min(b.len())..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_split() {
        let b = split_base("http://127.0.0.1:19745/media").unwrap();
        assert_eq!((b.secure, b.auth.as_str(), b.prefix.as_str()), (false, "127.0.0.1:19745", "/media"));
        assert_eq!(b.ws_url(), "ws://127.0.0.1:19745/media/ws");
        let b = split_base("https://hub/").unwrap();
        assert_eq!((b.secure, b.auth.as_str(), b.prefix.as_str()), (true, "hub:443", ""));
        assert_eq!(b.ws_url(), "wss://hub:443/ws");
        assert!(split_base("ftp://hub").is_err());
    }

    #[test]
    fn chunked() {
        assert_eq!(dechunk(b"4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n"), b"{\"a\":1}".to_vec());
    }
}
