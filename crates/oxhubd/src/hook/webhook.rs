// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-WEBHOOK · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-OPS-DROPS · 정책:hook · model: claude-opus-5-5
//! webhook 운반 — 계정마다 한 줄(차례 — 한 묶음이 끝나야 다음 묶음): 계정 큐에서 묶음을 꺼내 Standard Webhooks 1.0.0 대칭 `v1` 꼴로 `POST`(머리 `webhook-id` = 묶음 id(사실 id 꼴 · 다시 보내기에 같은 값) · `webhook-timestamp` = 보내는 시각 초(시도마다 새로) · `webhook-signature` = `v1,` + base64(HMAC-SHA256(키, `id.timestamp.몸`)) · 키 = `whsec_` 뒤를 base64 로 푼 바이트) · ★2xx 만 성공 · 그 밖(3xx 도 — 따라가지 않는다) · 시한 · 연결 실패는 실패 → 지수 백오프 · 재시도 상한을 넘으면 그 묶음을 버리고 사실 수와 실었던 `dropped` 를 되돌려 센다 · 410 · 429 · `retry-after` 갈래 없음 · https 서버 인증서는 시스템 신뢰 저장소로.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use http_body_util::Full;
use hyper::Uri;
use hyper_util::rt::TokioIo;
use sha2::Sha256;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use super::Hook;
use super::zgate::send::FactsBundle;

pub const IDLE_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn target(url: &str) -> Result<Target, &'static str> {
    let u: Uri = url.parse().map_err(|_| "url unreadable")?;
    let https = match u.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => return Err("http · https only"),
    };
    let host = u.host().filter(|h| !h.is_empty()).ok_or("no host")?.trim_start_matches('[').trim_end_matches(']').to_string();
    let port = u.port_u16().unwrap_or(if https { 443 } else { 80 });
    let path = u.path_and_query().map(|p| p.as_str().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| "/".into());
    Ok(Target { https, host, port, path })
}

pub fn host_header(t: &Target) -> String {
    let host = if t.host.contains(':') { format!("[{}]", t.host) } else { t.host.clone() };
    if (t.https && t.port == 443) || (!t.https && t.port == 80) { host } else { format!("{host}:{}", t.port) }
}

pub fn sign(key: &[u8], id: &str, ts: u64, body: &[u8]) -> String {
    let Ok(mut m) = <Hmac<Sha256> as Mac>::new_from_slice(key) else { return String::new() };
    m.update(id.as_bytes());
    m.update(b".");
    m.update(ts.to_string().as_bytes());
    m.update(b".");
    m.update(body);
    format!("v1,{}", base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes()))
}

pub fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Status(u16),
    Timeout,
    Failed(String),
}

impl Sent {
    pub fn ok(&self) -> bool {
        matches!(self, Sent::Status(s) if success(*s))
    }

    pub fn reason(&self) -> String {
        match self {
            Sent::Status(s) => format!("status {s}"),
            Sent::Timeout => "timeout".into(),
            Sent::Failed(e) => e.clone(),
        }
    }
}

pub fn retry_after(failures: u32, backoff: &[u32], retry_max: u8) -> Option<Duration> {
    if failures > u32::from(retry_max) || failures == 0 {
        return None;
    }
    let i = usize::try_from(failures - 1).unwrap_or(usize::MAX);
    backoff.get(i).or(backoff.last()).map(|ms| Duration::from_millis(u64::from(*ms)))
}

#[derive(Debug, Clone)]
pub struct Knobs {
    pub backoff_ms: Vec<u32>,
    pub retry_max: u8,
    pub timeout: Duration,
    pub bundle_max_bytes: usize,
}

impl Knobs {
    pub fn of(hook: &common::policy::Hook, bundle_max_bytes: usize) -> Knobs {
        Knobs {
            backoff_ms: hook.retry_backoff_ms.clone(),
            retry_max: hook.retry_max,
            timeout: Duration::from_millis(u64::from(hook.webhook_timeout_ms)),
            bundle_max_bytes,
        }
    }
}

pub struct Webhook {
    pub account: String,
    pub target: Target,
    pub key: Vec<u8>,
    pub url: String,
}

impl Webhook {
    pub fn of(w: &common::system::Webhook) -> Result<Webhook, String> {
        let key = common::system::webhook_secret_key(&w.secret).map_err(|e| format!("hook.webhooks key {:?} — secret {e}", w.key))?;
        let target = target(&w.url).map_err(|e| format!("hook.webhooks key {:?} — url {e}", w.key))?;
        Ok(Webhook { account: w.key.clone(), target, key, url: w.url.clone() })
    }
}

pub fn tls() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for c in loaded.certs {
        let _ = roots.add(c);
    }
    if !loaded.errors.is_empty() {
        tracing::warn!(kind = "diag", spec = "SV-RULE-HOOK-WEBHOOK", errors = loaded.errors.len(), "system trust store — some roots were not read");
    }
    TlsConnector::from(Arc::new(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()))
}

pub async fn post(t: &Target, tls: &TlsConnector, headers: &[(&str, String)], body: Bytes, timeout: Duration) -> Sent {
    match tokio::time::timeout(timeout, post_once(t, tls, headers, body)).await {
        Ok(Ok(s)) => Sent::Status(s),
        Ok(Err(e)) => Sent::Failed(e),
        Err(_) => Sent::Timeout,
    }
}

async fn post_once(t: &Target, tls: &TlsConnector, headers: &[(&str, String)], body: Bytes) -> Result<u16, String> {
    let tcp = TcpStream::connect((t.host.as_str(), t.port)).await.map_err(|e| format!("connect: {e}"))?;
    let mut req = hyper::Request::post(t.path.as_str())
        .header(hyper::header::HOST, host_header(t))
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .header(hyper::header::CONTENT_LENGTH, body.len());
    for (k, v) in headers {
        req = req.header(*k, v.as_str());
    }
    let req = req.body(Full::new(body)).map_err(|e| format!("request: {e}"))?;
    if t.https {
        let name = ServerName::try_from(t.host.clone()).map_err(|e| format!("server name: {e}"))?;
        let s = tls.connect(name, tcp).await.map_err(|e| format!("tls: {e}"))?;
        send(TokioIo::new(s), req).await
    } else {
        send(TokioIo::new(tcp), req).await
    }
}

async fn send<I>(io: I, req: hyper::Request<Full<Bytes>>) -> Result<u16, String>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut s, conn) = hyper::client::conn::http1::handshake(io).await.map_err(|e| format!("handshake: {e}"))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let res = s.send_request(req).await.map_err(|e| format!("send: {e}"))?;
    Ok(res.status().as_u16())
}

pub fn headers(id: &str, ts: u64, sig: String) -> Vec<(&'static str, String)> {
    vec![("webhook-id", id.to_string()), ("webhook-timestamp", ts.to_string()), ("webhook-signature", sig)]
}

pub async fn deliver(hook: &Hook, w: &Webhook, k: &Knobs, tls: &TlsConnector, b: FactsBundle) -> bool {
    let id = hook.mint(common::now_ms());
    let body = Bytes::from(serde_json::to_vec(&b).unwrap_or_default());
    let mut failures = 0u32;
    loop {
        let ts = common::now_ms() / 1000;
        let sent = post(&w.target, tls, &headers(&id, ts, sign(&w.key, &id, ts, &body)), body.clone(), k.timeout).await;
        if sent.ok() {
            return true;
        }
        failures += 1;
        match retry_after(failures, &k.backoff_ms, k.retry_max) {
            Some(d) => {
                tracing::debug!(kind = "diag", spec = "SV-RULE-HOOK-WEBHOOK", key = %w.account, webhook_id = %id, tries = failures, why = %sent.reason(), "webhook failed — retry");
                tokio::time::sleep(d).await;
            }
            None => {
                tracing::warn!(kind = "diag", spec = "SV-RULE-HOOK-WEBHOOK", key = %w.account, webhook_id = %id, tries = failures, facts = b.facts.len(), why = %sent.reason(), "webhook given up — retry limit");
                hook.lost(&w.account, &b);
                return false;
            }
        }
    }
}

pub async fn run(hook: Arc<Hook>, w: Webhook, k: Knobs, tls: TlsConnector) {
    loop {
        let wake = hook.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        match hook.take(&w.account, k.bundle_max_bytes) {
            Some(b) => {
                let _ = deliver(&hook, &w, &k, &tls, b).await;
            }
            None => {
                let _ = tokio::time::timeout(IDLE_TICK, wake).await;
            }
        }
    }
}
