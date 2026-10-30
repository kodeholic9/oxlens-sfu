// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-TOOL-CONTRACT · 운영:RULE-AUTH-THREE · 운영:RULE-FMT-REPLY · 정:CAND-OPS-TOOL-HUB-FORM · model: claude-opus-5-5
//! hub 하나에 붙는 HTTP/1.1 — `host:port` · URL 둘 다 받는다(스킴이 없으면 `http://`) · 원격이면 그 자리에서 운영 토큰을 서명한다.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

pub struct Hub {
    pub url: String,
    pub source: &'static str,
    host: String,
    port: u16,
    token: Option<String>,
}

pub struct Reply {
    pub status: u16,
    pub body: Value,
}

#[derive(serde::Serialize)]
struct OpsClaims<'a> {
    iss: &'a str,
    ops: bool,
    iat: u64,
    exp: u64,
}

pub const DEFAULT_HUB: &str = "http://127.0.0.1:1974";
pub const TOKEN_TTL_SECS: u64 = 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn sign(api_key: &str, api_secret: &str) -> Result<String, String> {
    let iat = now_secs();
    let c = OpsClaims { iss: api_key, ops: true, iat, exp: iat + TOKEN_TTL_SECS };
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &c, &jsonwebtoken::EncodingKey::from_secret(api_secret.as_bytes())).map_err(|e| e.to_string())
}

impl Hub {
    pub fn resolve(flag: Option<&str>, env: Option<String>) -> Result<Hub, String> {
        let (raw, source) = match (flag, env) {
            (Some(f), _) => (f.to_string(), "--hub"),
            (None, Some(e)) if !e.is_empty() => (e, "OXADMIN_URL"),
            _ => (DEFAULT_HUB.to_string(), "기본값"),
        };
        let url = if raw.contains("://") { raw } else { format!("http://{raw}") };
        let rest = url.strip_prefix("http://").ok_or_else(|| format!("https 는 이 도구가 다루지 않는다: {url}"))?;
        let hostport = rest.split('/').next().unwrap_or(rest);
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| format!("포트가 아니다: {p}"))?),
            None => (hostport.to_string(), 80),
        };
        Ok(Hub { url: url.trim_end_matches('/').to_string(), source, host, port, token: None })
    }

    pub fn with_token(mut self, t: Option<String>) -> Hub {
        self.token = t;
        self
    }

    pub fn loopback(&self) -> bool {
        matches!(self.host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
    }

    pub fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Reply, String> {
        let addr = (self.host.trim_matches(|c| c == '[' || c == ']'), self.port)
            .to_socket_addrs()
            .map_err(|e| format!("{}: {e}", self.url))?
            .next()
            .ok_or_else(|| format!("{}: 주소 없음", self.url))?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("{}: {e}", self.url))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept: application/json\r\n", self.host, self.port);
        if let Some(t) = &self.token {
            req.push_str(&format!("Authorization: Bearer {t}\r\n"));
        }
        if body.is_some() || method == "POST" {
            req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len()));
        }
        req.push_str("\r\n");
        req.push_str(&payload);
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        parse(&raw)
    }
}

fn parse(raw: &[u8]) -> Result<Reply, String> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("응답 머리가 끝나지 않았다")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let status = head.split_whitespace().nth(1).and_then(|x| x.parse::<u16>().ok()).ok_or("상태 줄이 없다")?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    if chunked {
        body = dechunk(&body);
    }
    let v = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap_or(Value::String(String::from_utf8_lossy(&body).to_string())) };
    Ok(Reply { status, body: v })
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(end) = b[i..].windows(2).position(|w| w == b"\r\n") else { break };
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[i..i + end]).trim(), 16).unwrap_or(0);
        i += end + 2;
        if size == 0 || i + size > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size + 2;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_forms() {
        let h = Hub::resolve(Some("127.0.0.1:1975"), None).unwrap();
        assert_eq!((h.url.as_str(), h.source, h.port), ("http://127.0.0.1:1975", "--hub", 1975));
        let h = Hub::resolve(None, Some("http://10.0.0.2:1974/".into())).unwrap();
        assert_eq!((h.url.as_str(), h.source), ("http://10.0.0.2:1974", "OXADMIN_URL"));
        let h = Hub::resolve(None, None).unwrap();
        assert_eq!(h.source, "기본값");
        assert!(h.loopback());
    }

    #[test]
    fn chunked_body() {
        let r = parse(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n").unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, serde_json::json!({ "a": 1 }));
    }
}
