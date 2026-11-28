// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 정:IMPL-FLR-TALK-REPORT · 정:RULE-OPS-DROPS · 정:RULE-BUS-KEYSPACE · 정:IMPL-SES-TOKEN-ISSUE · 연:RULE-CHAT-REC-FILE · 연:ROOM.http.CREATE_ROOM · model: claude-opus-5-5
//! 녹음 유닛 라이브러리 — 얇은 실행 파일이 `unit::run(인자, 등록부)` 를 부른다 · 결과물 소비자 등록부(`consumer`) · 방 작업(`job`) · 파일 자리(`files`) · 로컬 게이트웨이 HTTP(`--hub-base` · TLS 면 `--hub-ca` 인증서 하나로 검증 · 방 상세 · 방 세션 토큰) · 닫은 녹취의 `rec` 알림 한 줄(일시 실패는 백오프로 다시 · 상한을 넘기면 버리고 센다) · 운영 표면(Handle `Call` 은 `/admin/drops` · 감독자의 토큰 넘김 `POST /unit/token` · 지원 종류 `GET /unit/record_kinds` = 등록된 이름 전부) · 방 작업 되살림 값(`Restart` — rec 유닛 인자 `--backoff-ms` · `--intensity-max` · `--intensity-window-secs`).

pub mod bus;
pub mod consumer;
pub mod files;
pub mod job;
pub mod sink;
pub mod unit;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use common::talk::{Outcome, Report};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub use consumer::{RecConsumer, RecoverCtx, Registry, RoomCtx};
pub use files::{DAY, Layout, PART_DIR, sweep};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restart {
    pub backoff_ms: Vec<u64>,
    pub intensity_max: u32,
    pub intensity_window: Duration,
}

impl Restart {
    pub fn parse(backoff: Option<&str>, max: Option<&str>, window: Option<&str>) -> Result<Restart, String> {
        let d = common::system::Supervisor::default();
        let backoff_ms = match backoff {
            Some(s) => s.split(',').map(|x| x.trim().parse::<u64>().map_err(|_| format!("--backoff-ms {s:?}"))).collect::<Result<Vec<_>, _>>()?,
            None => d.backoff_ms,
        };
        if backoff_ms.is_empty() {
            return Err("--backoff-ms empty".to_string());
        }
        let intensity_max = max.map(|s| s.parse().map_err(|_| format!("--intensity-max {s:?}"))).transpose()?.unwrap_or(d.intensity_max);
        let secs = window.map(|s| s.parse().map_err(|_| format!("--intensity-window-secs {s:?}"))).transpose()?.unwrap_or(d.intensity_window_secs);
        Ok(Restart { backoff_ms, intensity_max, intensity_window: Duration::from_secs(secs) })
    }
}

pub async fn report_line<A, F>(mut rx: mpsc::UnboundedReceiver<Report>, retry_for: Duration, mut ask: A, drops: &AtomicU64)
where
    A: FnMut(Vec<u8>) -> F,
    F: std::future::Future<Output = Option<Vec<u8>>>,
{
    while let Some(r) = rx.recv().await {
        match common::talk::deliver(&r, retry_for, &mut ask).await {
            Outcome::Done => {}
            Outcome::Refused(c) => {
                tracing::info!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", talk = %r.talk_id(), code = c.num(), "rec not linked — no space or chat off");
            }
            Outcome::Dropped => {
                drops.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", talk = %r.talk_id(), "talk_drop — rec report dropped after retry limit");
            }
        }
    }
}

#[derive(Clone)]
pub struct Hub {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub tls: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
}

pub fn hub_of(base: &str, ca_pem: Option<&[u8]>) -> Option<Hub> {
    let (rest, secure) = match base.strip_prefix("https://") {
        Some(r) => (r, true),
        None => (base.strip_prefix("http://")?, false),
    };
    let tls = match (secure, ca_pem) {
        (false, _) => None,
        (true, Some(pem)) => Some(oxpeer::tls::pinned_config(pem).ok()?),
        (true, None) => return None,
    };
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
        None => (rest, String::new()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.trim_start_matches('[').trim_end_matches(']').to_string(), p.parse().ok()?),
        None => (hostport.to_string(), if secure { 443 } else { 80 }),
    };
    let path = if path == "/" { String::new() } else { path };
    Some(Hub { host, port, path, tls })
}

pub async fn get_json(hub: &Hub, path: &str, token: &str) -> Option<(u16, Value)> {
    request_json(hub, "GET", path, token, None).await
}

pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn request_json(hub: &Hub, method: &str, path: &str, token: &str, body: Option<&Value>) -> Option<(u16, Value)> {
    tokio::time::timeout(HTTP_TIMEOUT, request_once(hub, method, path, token, body)).await.ok().flatten()
}

async fn request_once(hub: &Hub, method: &str, path: &str, token: &str, body: Option<&Value>) -> Option<(u16, Value)> {
    let tcp = tokio::net::TcpStream::connect((hub.host.as_str(), hub.port)).await.ok()?;
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let typed = if body.is_some() { format!("Content-Type: application/json\r\nContent-Length: {}\r\n", payload.len()) } else { String::new() };
    let req = format!("{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\n{typed}Connection: close\r\n\r\n{payload}", hub.path, hub.host);
    let mut buf = Vec::new();
    match &hub.tls {
        None => {
            let mut s = tcp;
            s.write_all(req.as_bytes()).await.ok()?;
            s.read_to_end(&mut buf).await.ok()?;
        }
        Some(cfg) => {
            let name = tokio_rustls::rustls::pki_types::ServerName::try_from(hub.host.clone()).ok()?;
            let mut s = tokio_rustls::TlsConnector::from(cfg.clone()).connect(name, tcp).await.ok()?;
            s.write_all(req.as_bytes()).await.ok()?;
            let _ = s.read_to_end(&mut buf).await;
        }
    }
    parse_reply(&buf)
}

pub struct Svc {
    pub epoch: String,
    pub build: String,
    pub drops: std::sync::Arc<AtomicU64>,
    pub token: tokio::sync::watch::Sender<String>,
    pub kinds: Vec<String>,
}

pub fn handed_token(env: &common::envelope::Envelope) -> Option<String> {
    let post = env.method.as_deref() == Some("POST");
    match (&env.kind, post, env.path.as_deref().map(|p| p.trim_matches('/'))) {
        (common::envelope::CallKind::Admin, true, Some(common::system::REC_TOKEN_PATH)) => env.body.get("token").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).map(str::to_string),
        _ => None,
    }
}

#[tonic::async_trait]
impl common::envelope::handle::handle_server::Handle for Svc {
    async fn hello(&self, req: tonic::Request<common::envelope::handle::HelloReq>) -> Result<tonic::Response<common::envelope::handle::HelloRes>, tonic::Status> {
        let r = req.into_inner();
        tracing::info!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "hello from hub {} ({})", r.node, r.hub_inst);
        Ok(tonic::Response::new(common::envelope::handle::HelloRes { epoch: self.epoch.clone(), build: self.build.clone() }))
    }

    async fn call(&self, req: tonic::Request<common::envelope::handle::CallReq>) -> Result<tonic::Response<common::envelope::handle::CallRes>, tonic::Status> {
        let env = common::envelope::Envelope::decode(&req.into_inner().envelope).map_err(|e| tonic::Status::invalid_argument(e.to_string()))?;
        if let Some(t) = handed_token(&env) {
            self.token.send_replace(t);
            tracing::info!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec unit token renewed");
            return Ok(tonic::Response::new(common::envelope::handle::CallRes { reply: common::envelope::Reply::ok(serde_json::json!({})).encode() }));
        }
        let log = common::log::global().map(|l| (l.counters.queue_full.load(Ordering::Relaxed), l.counters.spool_full.load(Ordering::Relaxed)));
        let reply = admin_answer(&env, self.drops.load(Ordering::Relaxed), log, &self.kinds);
        Ok(tonic::Response::new(common::envelope::handle::CallRes { reply: reply.encode() }))
    }
}

pub fn admin_answer(env: &common::envelope::Envelope, talk_drop: u64, log: Option<(u64, u64)>, kinds: &[String]) -> common::envelope::Reply {
    use common::envelope::{CallKind, Reply};
    let get = env.method.as_deref().unwrap_or("GET") == "GET";
    match (&env.kind, get, env.path.as_deref().map(|p| p.trim_matches('/'))) {
        (CallKind::Admin, true, Some("admin/drops")) => {
            let mut o = serde_json::json!({ "talk_drop": talk_drop });
            if let Some((q, s)) = log {
                o["log_queue_full"] = serde_json::json!(q);
                o["log_spool_full"] = serde_json::json!(s);
            }
            Reply::ok(o)
        }
        (CallKind::Admin, true, Some(common::system::REC_KINDS_PATH)) => Reply::ok(serde_json::json!({ "kinds": kinds })),
        _ => Reply::fail(oxsig::Code::UnknownOp.failure().with_message(format!("{:?} {}", env.kind, env.path.as_deref().unwrap_or("")))),
    }
}

pub fn parse_reply(buf: &[u8]) -> Option<(u16, Value)> {
    let at = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..at]).ok()?;
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let body = &buf[at + 4..];
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { unchunk(body)? } else { body.to_vec() };
    Some((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

fn unchunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let at = b.windows(2).position(|w| w == b"\r\n")?;
        let n = usize::from_str_radix(std::str::from_utf8(&b[..at]).ok()?.split(';').next()?.trim(), 16).ok()?;
        b = &b[at + 2..];
        if n == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..n)?);
        b = b.get(n + 2..)?;
    }
}
