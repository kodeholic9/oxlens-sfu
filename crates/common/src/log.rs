// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-LOGGER · 정:RULE-OPS-OBSERVE · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 보내는 쪽 로깅 — `tracing` Layer 하나가 logfmt 줄(운영 규격서 §7-3)을 짓고 유한 채널 → 전담 스레드 → DGRAM 소켓 · 실패 즉시 spool · 소켓 미설정이면 stdout.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use crate::clock;

const KEY_ORDER: [&str; 10] = ["session", "room", "user", "epoch", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq", "arev"];

pub type SinkFn = Box<dyn Fn(&str) + Send + Sync>;

pub struct LogConf {
    pub node: String,
    pub proc_name: String,
    pub socket: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub level: String,
    pub line_max: usize,
    pub queue_max: usize,
    pub spool_max: u64,
    pub sink: Option<SinkFn>,
}

#[derive(Default)]
pub struct Counts {
    pub sent: AtomicU64,
    pub queue_dropped: AtomicU64,
    pub spooled: AtomicU64,
    pub spool_dropped: AtomicU64,
    pub eagain: AtomicU64,
    pub emsgsize: AtomicU64,
    pub refused: AtomicU64,
    pub truncated: AtomicU64,
}

static COUNTS: OnceLock<Arc<Counts>> = OnceLock::new();
static LEVEL: AtomicU8 = AtomicU8::new(3);
static BASE_LEVEL: AtomicU8 = AtomicU8::new(3);
static OVERRIDE_UNTIL: AtomicU64 = AtomicU64::new(0);

pub fn counts() -> Arc<Counts> {
    COUNTS.get_or_init(|| Arc::new(Counts::default())).clone()
}

pub fn level_rank(s: &str) -> Option<u8> {
    match s {
        "trace" => Some(5),
        "debug" => Some(4),
        "info" => Some(3),
        "warn" => Some(2),
        "error" => Some(1),
        _ => None,
    }
}

pub fn level_name(r: u8) -> &'static str {
    match r {
        5 => "trace",
        4 => "debug",
        3 => "info",
        2 => "warn",
        _ => "error",
    }
}

fn rank_of(l: &Level) -> u8 {
    match *l {
        Level::TRACE => 5,
        Level::DEBUG => 4,
        Level::INFO => 3,
        Level::WARN => 2,
        Level::ERROR => 1,
    }
}

pub fn current_level() -> &'static str {
    level_name(effective_level())
}

fn effective_level() -> u8 {
    let until = OVERRIDE_UNTIL.load(Ordering::Relaxed);
    if until != 0 && clock::wall_ms() >= until {
        OVERRIDE_UNTIL.store(0, Ordering::Relaxed);
        LEVEL.store(BASE_LEVEL.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    LEVEL.load(Ordering::Relaxed)
}

pub fn override_level(level: &str, ttl_secs: Option<u64>) -> Option<(String, String, u64)> {
    let r = level_rank(level)?;
    let from = current_level().to_string();
    LEVEL.store(r, Ordering::Relaxed);
    let until = ttl_secs.map(|s| clock::wall_ms() + s * 1000).unwrap_or(0);
    OVERRIDE_UNTIL.store(until, Ordering::Relaxed);
    Some((from, level.to_string(), until))
}

struct Fields {
    keys: Vec<(String, String)>,
    msg: Option<String>,
}

impl Visit for Fields {
    fn record_str(&mut self, f: &Field, v: &str) {
        if f.name() == "message" {
            self.msg = Some(v.to_string());
        } else {
            self.keys.push((f.name().to_string(), v.to_string()));
        }
    }

    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        let s = format!("{v:?}");
        if f.name() == "message" {
            self.msg = Some(s);
        } else {
            self.keys.push((f.name().to_string(), s.trim_matches('"').to_string()));
        }
    }

    fn record_u64(&mut self, f: &Field, v: u64) {
        self.keys.push((f.name().to_string(), v.to_string()));
    }

    fn record_i64(&mut self, f: &Field, v: i64) {
        self.keys.push((f.name().to_string(), v.to_string()));
    }

    fn record_bool(&mut self, f: &Field, v: bool) {
        self.keys.push((f.name().to_string(), if v { "1".to_string() } else { "0".to_string() }));
    }
}

fn quote(v: &str) -> String {
    if v.is_empty() || v.contains([' ', '"', '=', '\n', '\t']) {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
    } else {
        v.to_string()
    }
}

pub fn line(node: &str, proc_name: &str, keys: &[(String, String)], msg: Option<&str>, line_max: usize) -> (String, bool) {
    let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z");
    let mut s = String::with_capacity(256);
    let _ = write!(s, "ts={ts} mono={} node={} proc={}", clock::mono_us(), quote(node), quote(proc_name));
    let get = |k: &str| keys.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let kind = get("kind").unwrap_or("diag");
    let _ = write!(s, " kind={}", quote(kind));
    if let (true, Some(spec)) = (kind == "ev", get("spec")) {
        let _ = write!(s, " spec={}", quote(spec));
    }
    for k in KEY_ORDER {
        if let Some(v) = get(k) {
            let _ = write!(s, " {k}={}", quote(v));
        }
    }
    for (k, v) in keys {
        if k == "kind" || k == "spec" || KEY_ORDER.contains(&k.as_str()) {
            continue;
        }
        let _ = write!(s, " {k}={}", quote(v));
    }
    let mut truncated = false;
    if let Some(m) = msg {
        let room = line_max.saturating_sub(s.len() + 20);
        let mut m = m.to_string();
        if m.len() > room {
            let mut cut = room;
            while cut > 0 && !m.is_char_boundary(cut) {
                cut -= 1;
            }
            m.truncate(cut);
            truncated = true;
        }
        let _ = write!(s, " msg={}", quote(&m));
        if truncated {
            s.push_str(" truncated=1");
        }
    }
    (s, truncated)
}

struct OxLayer {
    node: String,
    proc_name: String,
    tx: SyncSender<String>,
    line_max: usize,
    counts: Arc<Counts>,
}

impl<S: Subscriber> Layer<S> for OxLayer {
    fn enabled(&self, meta: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        rank_of(meta.level()) <= effective_level()
    }

    fn on_event(&self, ev: &Event<'_>, _ctx: Context<'_, S>) {
        if rank_of(ev.metadata().level()) > effective_level() {
            return;
        }
        let mut f = Fields { keys: Vec::new(), msg: None };
        ev.record(&mut f);
        let (l, truncated) = line(&self.node, &self.proc_name, &f.keys, f.msg.as_deref(), self.line_max);
        if truncated {
            self.counts.truncated.fetch_add(1, Ordering::Relaxed);
        }
        match self.tx.try_send(l) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counts.queue_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

struct Spool {
    path: Option<PathBuf>,
    file: Option<std::fs::File>,
    size: u64,
    max: u64,
}

impl Spool {
    fn write(&mut self, l: &str, counts: &Counts) {
        let Some(path) = self.path.clone() else {
            counts.spool_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.size + l.len() as u64 + 1 > self.max {
            counts.spool_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.file.is_none() {
            self.file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        }
        match self.file.as_mut().map(|f| writeln!(f, "{l}")) {
            Some(Ok(())) => {
                self.size += l.len() as u64 + 1;
                counts.spooled.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                counts.spool_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn sender(rx: Receiver<String>, conf: SenderConf, counts: Arc<Counts>) {
    let sock = UnixDatagram::unbound().ok();
    if let Some(s) = &sock {
        let _ = s.set_nonblocking(true);
    }
    let mut spool = Spool {
        path: conf.log_dir.as_ref().map(|d| d.join(format!("{}.{}.spool.log", conf.node, conf.proc_name))),
        file: None,
        size: 0,
        max: conf.spool_max,
    };
    let mut failing = false;
    let mut since_spooled = 0u64;
    let mut since_dropped = 0u64;
    while let Ok(l) = rx.recv() {
        if let Some(f) = &conf.sink {
            f(&l);
            continue;
        }
        let Some(path) = &conf.socket else {
            let out = std::io::stdout();
            let mut lock = out.lock();
            let _ = writeln!(lock, "{l}");
            continue;
        };
        let res = sock.as_ref().map(|s| s.send_to(l.as_bytes(), path));
        match res {
            Some(Ok(_)) => {
                counts.sent.fetch_add(1, Ordering::Relaxed);
                if failing {
                    failing = false;
                    let spooled = counts.spooled.load(Ordering::Relaxed) - since_spooled;
                    let dropped = counts.spool_dropped.load(Ordering::Relaxed) + counts.queue_dropped.load(Ordering::Relaxed) - since_dropped;
                    let keys = vec![("kind".to_string(), "stat".to_string()), ("spooled".to_string(), spooled.to_string()), ("dropped".to_string(), dropped.to_string())];
                    let (r, _) = line(&conf.node, &conf.proc_name, &keys, None, conf.line_max);
                    if let Some(s) = &sock {
                        let _ = s.send_to(r.as_bytes(), path);
                    }
                }
            }
            Some(Err(e)) => {
                match e.raw_os_error() {
                    Some(35) | Some(11) => counts.eagain.fetch_add(1, Ordering::Relaxed),
                    Some(40) | Some(90) => counts.emsgsize.fetch_add(1, Ordering::Relaxed),
                    _ => counts.refused.fetch_add(1, Ordering::Relaxed),
                };
                if !failing {
                    failing = true;
                    since_spooled = counts.spooled.load(Ordering::Relaxed);
                    since_dropped = counts.spool_dropped.load(Ordering::Relaxed) + counts.queue_dropped.load(Ordering::Relaxed);
                }
                spool.write(&l, &counts);
            }
            None => spool.write(&l, &counts),
        }
    }
}

struct SenderConf {
    node: String,
    proc_name: String,
    socket: Option<PathBuf>,
    log_dir: Option<PathBuf>,
    line_max: usize,
    spool_max: u64,
    sink: Option<SinkFn>,
}

static INSTALLED: Mutex<bool> = Mutex::new(false);

pub fn init(conf: LogConf) {
    let mut done = match INSTALLED.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if *done {
        return;
    }
    let base = std::env::var("OX_LOG").ok().and_then(|l| level_rank(&l)).or_else(|| level_rank(&conf.level)).unwrap_or(3);
    BASE_LEVEL.store(base, Ordering::Relaxed);
    LEVEL.store(base, Ordering::Relaxed);
    let (tx, rx) = sync_channel::<String>(conf.queue_max.max(1));
    let counts = counts();
    let sc = SenderConf {
        node: conf.node.clone(),
        proc_name: conf.proc_name.clone(),
        socket: conf.socket,
        log_dir: conf.log_dir,
        line_max: conf.line_max,
        spool_max: conf.spool_max,
        sink: conf.sink,
    };
    let c2 = counts.clone();
    let _ = std::thread::Builder::new().name("ox-log".to_string()).spawn(move || sender(rx, sc, c2));
    let layer = OxLayer { node: conf.node, proc_name: conf.proc_name, tx, line_max: conf.line_max, counts };
    let sub = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(sub);
    done.clone_from(&true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logfmt_order_and_quoting() {
        let keys = vec![
            ("room".to_string(), "r1".to_string()),
            ("kind".to_string(), "ev".to_string()),
            ("spec".to_string(), "CP-OP-ROOM-JOIN".to_string()),
            ("session".to_string(), "s-1".to_string()),
            ("extra".to_string(), "a b".to_string()),
        ];
        let (l, t) = line("n1", "sfu", &keys, Some("joined"), 1024);
        assert!(!t);
        let at = |k: &str| l.find(k).unwrap_or(usize::MAX);
        assert!(at(" kind=ev") < at(" spec=CP-OP-ROOM-JOIN"));
        assert!(at(" spec=") < at(" session=s-1"));
        assert!(at(" session=") < at(" room=r1"));
        assert!(at(" room=") < at(" extra=\"a b\""));
        assert!(l.ends_with(" msg=joined"));
        assert!(l.contains(" mono="));
    }

    #[test]
    fn long_message_is_cut_and_marked() {
        let (l, t) = line("n", "hub", &[], Some(&"x".repeat(5000)), 1024);
        assert!(t);
        assert!(l.len() <= 1024);
        assert!(l.ends_with(" truncated=1"));
    }
}
