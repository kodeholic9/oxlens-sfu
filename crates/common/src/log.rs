// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 운영 규격서 §7(로깅) · 정:RULE-CONC-SIX 1 · model: claude-opus-5-5
//! 로그 — logfmt 한 줄 = 한 datagram. 유한 채널 · 전담 스레드 · 소켓 실패 즉시 spool · 보내는 쪽은 기다리지 않는다.

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

use crate::policy::{Logging, LoggingLevel};

const KEY_ORDER: &[&str] = &["session", "room", "user", "epoch", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq"];

#[derive(Debug, Default)]
pub struct Counters {
    pub dropped: AtomicU64,
    pub spooled: AtomicU64,
    pub eagain: AtomicU64,
    pub emsgsize: AtomicU64,
    pub absent: AtomicU64,
}

pub struct LogConfig<'a> {
    pub node: &'a str,
    pub proc_name: &'a str,
    pub logging: &'a Logging,
    pub log_dir: Option<&'a str>,
    pub log_socket: Option<&'a str>,
}

type Handle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

struct Shared {
    handle: Handle,
    base: String,
    restore_at: Mutex<Option<std::time::Instant>>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static COUNTERS: OnceLock<Arc<Counters>> = OnceLock::new();

pub fn counters() -> Option<Arc<Counters>> {
    COUNTERS.get().cloned()
}

fn level_name(l: LoggingLevel) -> &'static str {
    match l {
        LoggingLevel::Trace => "trace",
        LoggingLevel::Debug => "debug",
        LoggingLevel::Info => "info",
        LoggingLevel::Warn => "warn",
        LoggingLevel::Error => "error",
    }
}

pub fn init(cfg: LogConfig<'_>) {
    let base = std::env::var("OX_LOG").unwrap_or_else(|_| level_name(cfg.logging.level).to_string());
    let filter = EnvFilter::try_new(&base).unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, handle) = reload::Layer::new(filter);
    let counters = Arc::new(Counters::default());
    let _ = COUNTERS.set(counters.clone());
    let (tx, rx) = sync_channel::<String>(cfg.logging.queue_max_lines as usize);
    let line_max = cfg.logging.line_max_bytes as usize;
    let layer = LineLayer { node: cfg.node.to_string(), proc_name: cfg.proc_name.to_string(), tx, line_max, counters: counters.clone() };
    let sink = Sink {
        socket: cfg.log_socket.map(str::to_string),
        spool_path: cfg
            .log_dir
            .map(|d| std::path::Path::new(d).join(format!("{}.{}.spool.log", cfg.node, cfg.proc_name))),
        spool_max: cfg.logging.spool_max_bytes,
        counters,
    };
    std::thread::Builder::new().name("ox-log".into()).spawn(move || sink.run(rx)).ok();
    let _ = tracing_subscriber::registry().with(filter).with(layer).try_init();
    let _ = SHARED.set(Shared { handle, base, restore_at: Mutex::new(None) });
}

pub fn set_level(level: &str, ttl_secs: Option<u64>) -> Result<(String, String), String> {
    let sh = SHARED.get().ok_or("logger not initialised")?;
    let f = EnvFilter::try_new(level).map_err(|e| e.to_string())?;
    let from = sh.handle.with_current(|cur| cur.to_string()).map_err(|e| e.to_string())?;
    sh.handle.reload(f).map_err(|e| e.to_string())?;
    if let Ok(mut g) = sh.restore_at.lock() {
        g.replace(std::time::Instant::now() + std::time::Duration::from_secs(ttl_secs.unwrap_or(u64::MAX / 4)));
    }
    Ok((from, level.to_string()))
}

pub fn restore_if_due() {
    let Some(sh) = SHARED.get() else { return };
    let due = match sh.restore_at.lock() {
        Ok(g) => g.is_some_and(|t| std::time::Instant::now() >= t),
        Err(_) => false,
    };
    if due {
        if let Ok(f) = EnvFilter::try_new(&sh.base) {
            let _ = sh.handle.reload(f);
        }
        if let Ok(mut g) = sh.restore_at.lock() {
            g.take();
        }
    }
}

struct LineLayer {
    node: String,
    proc_name: String,
    tx: SyncSender<String>,
    line_max: usize,
    counters: Arc<Counters>,
}

#[derive(Default)]
struct Fields {
    msg: String,
    kind: Option<String>,
    spec: Option<String>,
    keyed: Vec<(String, String)>,
    other: Vec<(String, String)>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field.name(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field.name(), value.to_string());
    }
}

impl Fields {
    fn put(&mut self, name: &str, v: String) {
        match name {
            "message" => self.msg = v,
            "kind" => self.kind = Some(v),
            "spec" => self.spec = Some(v),
            n if KEY_ORDER.contains(&n) => self.keyed.push((n.to_string(), v)),
            n => self.other.push((n.to_string(), v)),
        }
    }
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '"' || c == '=') {
        format!("\"{}\"", v.replace('"', "'"))
    } else {
        v.to_string()
    }
}

impl<S: Subscriber> Layer<S> for LineLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut f = Fields::default();
        event.record(&mut f);
        let kind = f.kind.clone().unwrap_or_else(|| "diag".to_string());
        let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z");
        let mut line = String::with_capacity(160);
        let _ = write!(line, "ts={ts} node={} proc={} kind={kind}", self.node, self.proc_name);
        if let (true, Some(s)) = (kind == "ev", &f.spec) {
            let _ = write!(line, " spec={s}");
        }
        for k in KEY_ORDER {
            if let Some((_, v)) = f.keyed.iter().find(|(n, _)| n == k) {
                let _ = write!(line, " {k}={}", quote(v));
            }
        }
        if *event.metadata().level() <= Level::WARN {
            let _ = write!(line, " level={}", event.metadata().level().as_str().to_lowercase());
        }
        for (k, v) in &f.other {
            let _ = write!(line, " {k}={}", quote(v));
        }
        let msg_part = format!(" msg={}", quote(&f.msg));
        if line.len() + msg_part.len() > self.line_max {
            let room = self.line_max.saturating_sub(line.len() + " truncated=1 msg=\"\"".len());
            let cut: String = f.msg.chars().take(room).collect();
            let _ = write!(line, " truncated=1 msg={}", quote(&cut));
        } else {
            line.push_str(&msg_part);
        }
        match self.tx.try_send(line) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct Sink {
    socket: Option<String>,
    spool_path: Option<std::path::PathBuf>,
    spool_max: u64,
    counters: Arc<Counters>,
}

impl Sink {
    fn run(self, rx: Receiver<String>) {
        #[cfg(unix)]
        let sock = std::os::unix::net::UnixDatagram::unbound().ok();
        let mut spool: Option<std::fs::File> = None;
        let mut spool_bytes: u64 = 0;
        let mut failing = false;
        let mut stdout = std::io::stdout();
        while let Ok(line) = rx.recv() {
            let Some(path) = self.socket.as_deref() else {
                let _ = writeln!(stdout, "{line}");
                continue;
            };
            #[cfg(unix)]
            let sent = match sock.as_ref() {
                Some(s) => match s.send_to(line.as_bytes(), path) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                },
                None => Err(std::io::Error::other("no socket")),
            };
            #[cfg(not(unix))]
            let sent: Result<(), std::io::Error> = Err(std::io::Error::other("no unix socket"));
            match sent {
                Ok(()) => {
                    if failing {
                        failing = false;
                        let note = format!(
                            "kind=stat spooled={} dropped={}",
                            self.counters.spooled.load(Ordering::Relaxed),
                            self.counters.dropped.load(Ordering::Relaxed)
                        );
                        #[cfg(unix)]
                        if let Some(s) = sock.as_ref() {
                            let _ = s.send_to(note.as_bytes(), path);
                        }
                    }
                }
                Err(e) => {
                    failing = true;
                    match e.raw_os_error() {
                        Some(35) | Some(11) => self.counters.eagain.fetch_add(1, Ordering::Relaxed),
                        Some(40) | Some(90) => self.counters.emsgsize.fetch_add(1, Ordering::Relaxed),
                        _ => self.counters.absent.fetch_add(1, Ordering::Relaxed),
                    };
                    let Some(sp) = self.spool_path.as_ref() else {
                        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    if spool.is_none() {
                        spool = std::fs::OpenOptions::new().create(true).append(true).open(sp).ok();
                    }
                    let n = line.len() as u64 + 1;
                    let wrote = match spool.as_mut() {
                        Some(f) if spool_bytes + n <= self.spool_max => writeln!(f, "{line}").is_ok(),
                        _ => false,
                    };
                    if wrote {
                        spool_bytes += n;
                        self.counters.spooled.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("ab"), "ab");
        assert_eq!(quote(""), "\"\"");
    }
}
