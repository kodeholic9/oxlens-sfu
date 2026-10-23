// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-TOKEN · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! 운영 도구의 HTTP — hub 하나에 붙는다(평문 HTTP/1.1) · 원격이면 그 자리에서 운영 토큰을 서명해 싣는다(서버에 발급을 조르지 않는다).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use oxsig::ses::OpsClaims;
use serde_json::Value;

pub struct Hub {
    pub host: String,
    pub key: Option<(String, String)>,
}

pub struct Resp {
    pub status: u16,
    pub body: Value,
}

impl Hub {
    pub fn new(url: &str, key: Option<(String, String)>) -> Result<Hub, String> {
        let rest = url.strip_prefix("http://").ok_or_else(|| format!("hub 주소는 http:// 로 시작한다: {url}"))?;
        let host = rest.split('/').next().unwrap_or(rest).to_string();
        Ok(Hub { host, key })
    }

    fn token(&self) -> Option<String> {
        let (k, s) = self.key.as_ref()?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let c = OpsClaims { iss: k.clone(), ops: true, iat: now, exp: now + 60 };
        jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256), &c, &jsonwebtoken::EncodingKey::from_secret(s.as_bytes())).ok()
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Resp, String> {
        let mut s = TcpStream::connect(&self.host).map_err(|e| format!("{}: {e}", self.host))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let auth = self.token().map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            self.host,
            payload.len()
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, rest) = text.split_once("\r\n\r\n").ok_or("응답 머리 없음")?;
        let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
        let body = if chunked { dechunk(rest) } else { rest.to_string() };
        Ok(Resp { status, body: serde_json::from_str(&body).unwrap_or(Value::String(body)) })
    }
}

fn dechunk(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((len, tail)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(len.trim(), 16).unwrap_or(0);
        if n == 0 || tail.len() < n {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].trim_start_matches("\r\n");
    }
    out
}
