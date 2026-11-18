// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 정:IMPL-FLR-TALK-REPORT · 정:RULE-OPS-DROPS · 정:RULE-BUS-KEYSPACE · 연:RULE-CHAT-REC-FILE · 연:ROOM.http.CREATE_ROOM · model: claude-opus-5-5
//! 녹음 유닛 — 발언마다 파일 하나(영상 트랙이 든 발언은 `.mkv` — 음성도 그 안 · 아니면 `.ogg` · 알림도 하나) · 파일 자리(`<root>/.rec-part/` 에 다 쓰고 `<root>/rec/{space_id}/` 로 이름 바꿔 닫는다 — 같은 저장소라 바꾸기는 한 번에) · 닫은 녹취의 `rec` 알림 · 하루 한 번 보관 기간을 넘긴 녹취 지우기 · 알림은 한 줄로(일시 실패는 백오프로 다시 · 상한을 넘기면 버리고 센다) · 로컬 게이트웨이(`--hub-base` · TLS 면 `--hub-ca` 인증서 하나로 검증) 방 상세의 `record` · 운영 표면(Handle `Call` 은 `/admin/drops` 만).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use common::talk::{Outcome, Report};
use oxpeer::record::{MkvFile, OggFile};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub const PART_DIR: &str = ".rec-part";
pub const DAY: Duration = Duration::from_secs(86_400);
pub const OGG_MIME: &str = "audio/ogg";
pub const MKV_MIME: &str = "video/x-matroska";

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
        let c = Closed { path: &f.path, talk: f.talk.as_deref(), user: f.user_id.as_deref(), ext: "ogg", mime: OGG_MIME, duration_ms: f.samples / 48, tag: f.segment };
        self.settle_file(space, &c)
    }

    pub fn settle_mkv(&self, f: &MkvFile) -> std::io::Result<Option<Report>> {
        let c = Closed { path: &f.path, talk: f.talk.as_deref(), user: f.user_id.as_deref(), ext: "mkv", mime: MKV_MIME, duration_ms: f.duration_ms, tag: f.index };
        self.settle_file(&f.room, &c)
    }

    pub fn settle_file(&self, space: &str, f: &Closed) -> std::io::Result<Option<Report>> {
        let (Some(talk), true) = (f.talk, common::keyspace::valid_piece(space)) else {
            std::fs::remove_file(f.path)?;
            return Ok(None);
        };
        let who = f.user.filter(|u| common::keyspace::valid_piece(u)).unwrap_or("none");
        let dir = self.rec_dir(space);
        std::fs::create_dir_all(&dir)?;
        let mut name = format!("{space}-{talk}-{who}.{}", f.ext);
        if dir.join(&name).exists() {
            name = format!("{space}-{talk}-{who}-{:04}.{}", f.tag, f.ext);
        }
        let to = dir.join(&name);
        std::fs::rename(f.path, &to)?;
        let size = std::fs::metadata(&to)?.len();
        Ok(Some(Report::Rec { talk_id: talk.to_string(), path: format!("rec/{space}/{name}"), size, mime: f.mime.into(), duration_ms: f.duration_ms }))
    }
}

pub struct Closed<'a> {
    pub path: &'a Path,
    pub talk: Option<&'a str>,
    pub user: Option<&'a str>,
    pub ext: &'a str,
    pub mime: &'a str,
    pub duration_ms: u64,
    pub tag: u64,
}

pub struct Talks {
    pub layout: Layout,
    ogg: oxpeer::record::OggSink,
    mkv: oxpeer::record::MkvSink,
    pick: OneFilePerTalk,
    rooms: std::collections::HashMap<u64, String>,
    ogg_seen: usize,
    mkv_seen: usize,
}

impl Talks {
    pub fn new(layout: Layout) -> std::io::Result<Talks> {
        let ogg = oxpeer::record::OggSink::new(layout.staging())?;
        let mkv = oxpeer::record::MkvSink::new(layout.staging())?;
        Ok(Talks { layout, ogg, mkv, pick: OneFilePerTalk::default(), rooms: Default::default(), ogg_seen: 0, mkv_seen: 0 })
    }

    pub fn feed(&mut self, o: &oxpeer::record::Output) -> Vec<Report> {
        use oxpeer::record::{Output, SegmentKind};
        let mut closed = false;
        if let Output::Segment(s) = o {
            match s.event {
                SegmentKind::Opened => {
                    self.rooms.insert(s.id, s.room.clone());
                    self.pick.saw(s);
                }
                SegmentKind::Closed => closed = true,
            }
        }
        for r in [self.ogg.write(o), self.mkv.write(o)] {
            if let Err(e) = r {
                tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", "rec write: {e}");
            }
        }
        if closed { self.settle() } else { Vec::new() }
    }

    pub fn keyframe_requests(&mut self) -> Vec<String> {
        self.mkv.take_keyframe_requests()
    }

    fn settle(&mut self) -> Vec<Report> {
        let mut out = Vec::new();
        let mkv: Vec<MkvFile> = self.mkv.files().iter().skip(self.mkv_seen).cloned().collect();
        self.mkv_seen += mkv.len();
        for f in mkv {
            let r = if self.pick.keep_mkv(&f) { self.layout.settle_mkv(&f) } else { std::fs::remove_file(&f.path).map(|_| None) };
            Self::push(&mut out, r, &f.path);
        }
        let ogg: Vec<OggFile> = self.ogg.files().iter().skip(self.ogg_seen).cloned().collect();
        self.ogg_seen += ogg.len();
        for f in ogg {
            let space = self.rooms.remove(&f.segment).unwrap_or_default();
            let r = if self.pick.keep_ogg(&f) { self.layout.settle(&space, &f) } else { std::fs::remove_file(&f.path).map(|_| None) };
            Self::push(&mut out, r, &f.path);
        }
        out
    }

    fn push(out: &mut Vec<Report>, r: std::io::Result<Option<Report>>, path: &Path) {
        match r {
            Ok(Some(x)) => out.push(x),
            Ok(None) => {}
            Err(e) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", file = %path.display(), "rec settle: {e}"),
        }
    }
}

#[derive(Default)]
pub struct OneFilePerTalk {
    video: std::collections::HashSet<String>,
}

impl OneFilePerTalk {
    pub fn saw(&mut self, s: &oxpeer::record::Segment) {
        if s.owner.is_none()
            && s.kind == "video"
            && let Some(t) = &s.talk
        {
            self.video.insert(t.clone());
        }
    }

    pub fn keep_ogg(&self, f: &OggFile) -> bool {
        f.talk.as_ref().is_some_and(|t| !self.video.contains(t))
    }

    pub fn keep_mkv(&mut self, f: &MkvFile) -> bool {
        let video = f.owner.is_none() && f.tracks.iter().any(|t| t.kind == "video");
        if let Some(t) = &f.talk
            && video
        {
            self.video.remove(t);
            return true;
        }
        false
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
    let tcp = tokio::net::TcpStream::connect((hub.host.as_str(), hub.port)).await.ok()?;
    let req = format!("GET {}{} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n", hub.path, path, hub.host, token);
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
        let log = common::log::global().map(|l| (l.counters.queue_full.load(Ordering::Relaxed), l.counters.spool_full.load(Ordering::Relaxed)));
        let reply = admin_answer(&env, self.drops.load(Ordering::Relaxed), log);
        Ok(tonic::Response::new(common::envelope::handle::CallRes { reply: reply.encode() }))
    }
}

pub fn admin_answer(env: &common::envelope::Envelope, talk_drop: u64, log: Option<(u64, u64)>) -> common::envelope::Reply {
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
