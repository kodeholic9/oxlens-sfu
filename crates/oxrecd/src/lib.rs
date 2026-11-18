// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 정:IMPL-FLR-TALK-REPORT · 정:RULE-OPS-DROPS · 정:RULE-BUS-KEYSPACE · 연:RULE-CHAT-REC-FILE · 연:ROOM.http.CREATE_ROOM · model: claude-opus-5-5
//! 녹음 유닛 — 파일 자리(`<root>/.rec-part/` 에 다 쓰고 `<root>/rec/{space_id}/` 로 이름 바꿔 닫는다 — 같은 저장소라 바꾸기는 한 번에) · 닫은 녹취의 `rec` 알림 · 하루 한 번 보관 기간을 넘긴 녹취 지우기 · 알림은 한 줄로(일시 실패는 백오프로 다시 · 상한을 넘기면 버리고 센다) · 로컬 게이트웨이 방 상세의 `record`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use common::talk::{Outcome, Report};
use oxpeer::record::OggFile;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub const PART_DIR: &str = ".rec-part";
pub const DAY: Duration = Duration::from_secs(86_400);
pub const OGG_MIME: &str = "audio/ogg";

pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Layout {
        Layout { root: root.into() }
    }

    pub fn staging(&self) -> PathBuf {
        self.root.join(PART_DIR)
    }

    pub fn rec_dir(&self, space: &str) -> PathBuf {
        self.root.join("rec").join(space)
    }

    pub fn clear_staging(&self) -> std::io::Result<usize> {
        let dir = self.staging();
        std::fs::create_dir_all(&dir)?;
        let mut n = 0;
        for f in std::fs::read_dir(&dir)?.flatten() {
            if f.file_type().map(|t| t.is_file()).unwrap_or(false) {
                std::fs::remove_file(f.path())?;
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn settle(&self, space: &str, f: &OggFile) -> std::io::Result<Option<Report>> {
        let (Some(talk), true) = (f.talk.as_deref(), common::keyspace::valid_piece(space)) else {
            std::fs::remove_file(&f.path)?;
            return Ok(None);
        };
        let who = f.user_id.as_deref().filter(|u| common::keyspace::valid_piece(u)).unwrap_or("none");
        let dir = self.rec_dir(space);
        std::fs::create_dir_all(&dir)?;
        let mut name = format!("{space}-{talk}-{who}.ogg");
        if dir.join(&name).exists() {
            name = format!("{space}-{talk}-{who}-{:04}.ogg", f.segment);
        }
        let to = dir.join(&name);
        std::fs::rename(&f.path, &to)?;
        let size = std::fs::metadata(&to)?.len();
        Ok(f.talk.clone().map(|talk_id| Report::Rec { talk_id, path: format!("rec/{space}/{name}"), size, mime: OGG_MIME.into(), duration_ms: f.samples / 48 }))
    }
}

pub fn sweep(root: &Path, keep_days: u32, now: SystemTime) -> std::io::Result<usize> {
    let cut = now.checked_sub(DAY * keep_days).unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(spaces) = std::fs::read_dir(root.join("rec")) else { return Ok(0) };
    let mut n = 0;
    for sp in spaces.flatten() {
        if !sp.file_type()?.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(sp.path())?.flatten() {
            let m = f.metadata()?;
            if m.is_file() && m.modified()? < cut {
                std::fs::remove_file(f.path())?;
                n += 1;
            }
        }
    }
    Ok(n)
}

pub fn recorded(body: &Value) -> bool {
    body.get("record").and_then(Value::as_bool).unwrap_or(false)
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

pub async fn ask_bus(mesh: oxmesh::Mesh, key: String, payload: Vec<u8>) -> Option<Vec<u8>> {
    let mut r = mesh.get(&key, payload, common::talk::ASK_TIMEOUT).ok()?;
    r.recv().await.and_then(|x| x.result.ok())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hub {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn hub_of(base: &str) -> Option<Hub> {
    let rest = base.strip_prefix("http://")?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
        None => (rest, String::new()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (hostport.to_string(), 80),
    };
    let path = if path == "/" { String::new() } else { path };
    Some(Hub { host, port, path })
}

pub async fn get_json(hub: &Hub, path: &str, token: &str) -> Option<(u16, Value)> {
    let mut s = tokio::net::TcpStream::connect((hub.host.as_str(), hub.port)).await.ok()?;
    let req = format!("GET {}{} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", hub.path, path, hub.host, token);
    s.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.ok()?;
    parse_reply(&buf)
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
