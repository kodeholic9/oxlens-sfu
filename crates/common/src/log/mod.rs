// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:RULE-LOG-PRINCIPLE 운영:RULE-LOG-SENDER 운영:RULE-LOG-LEVEL 운영:RULE-LOG-KIND 운영:RULE-LOG-CONTEXT 운영:RULE-LOG-SPEC 정:RULE-OPS-LOGGER 정:CAND-OPS-LOG-ENV 정:CAND-OPS-FOREIGN-LOG 연:DEV-TRN-SESSION-SECRET · model: claude-opus-5-5
//! 보내는 쪽 — `tracing` Layer 하나가 줄을 짓고 · 유한 채널(가득이면 드롭 + 계수) · 전담 스레드가 Unix DGRAM 으로 · 실패 즉시 spool · 소켓 미설정이면 stdout.

pub mod line;

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

use crate::policy::LoggingLevel;
pub use line::{Kind, Record};

pub const TARGET_EV: &str = "ox::ev";
pub const TARGET_STAT: &str = "ox::stat";
const OUR_CRATES: &[&str] = &["oxsig", "common", "oxhubd", "oxsfud", "oxlogd", "oxadmin"];

#[macro_export]
macro_rules! ev {
    ($($t:tt)*) => { ::tracing::info!(target: "ox::ev", $($t)*) };
}

#[macro_export]
macro_rules! stat {
    ($($t:tt)*) => { ::tracing::info!(target: "ox::stat", $($t)*) };
}

pub fn fingerprint(session_id: &str) -> String {
    let d = Sha256::digest(session_id.as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn start() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

pub fn mono_us() -> u64 {
    start().elapsed().as_micros() as u64
}

fn level_num(l: LoggingLevel) -> u8 {
    match l {
        LoggingLevel::Error => 1,
        LoggingLevel::Warn => 2,
        LoggingLevel::Info => 3,
        LoggingLevel::Debug => 4,
        LoggingLevel::Trace => 5,
    }
}

fn num_level(n: u8) -> LoggingLevel {
    match n {
        1 => LoggingLevel::Error,
        2 => LoggingLevel::Warn,
        3 => LoggingLevel::Info,
        4 => LoggingLevel::Debug,
        _ => LoggingLevel::Trace,
    }
}

fn tracing_num(l: &Level) -> u8 {
    match *l {
        Level::ERROR => 1,
        Level::WARN => 2,
        Level::INFO => 3,
        Level::DEBUG => 4,
        Level::TRACE => 5,
    }
}

pub fn level_name(l: LoggingLevel) -> &'static str {
    match l {
        LoggingLevel::Error => "error",
        LoggingLevel::Warn => "warn",
        LoggingLevel::Info => "info",
        LoggingLevel::Debug => "debug",
        LoggingLevel::Trace => "trace",
    }
}

pub fn parse_level(s: &str) -> Option<LoggingLevel> {
    match s {
        "error" => Some(LoggingLevel::Error),
        "warn" => Some(LoggingLevel::Warn),
        "info" => Some(LoggingLevel::Info),
        "debug" => Some(LoggingLevel::Debug),
        "trace" => Some(LoggingLevel::Trace),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct Counters {
    pub queue_full: AtomicU64,
    pub spooled: AtomicU64,
    pub spool_full: AtomicU64,
    pub eagain: AtomicU64,
    pub emsgsize: AtomicU64,
    pub absent: AtomicU64,
}

struct Shared {
    node: String,
    proc_: String,
    epoch: OnceLock<String>,
    base: AtomicU8,
    over: AtomicU8,
    until: AtomicU64,
    line_max: usize,
    tx: SyncSender<Vec<u8>>,
    counters: Counters,
}

impl Shared {
    fn level(&self) -> u8 {
        let o = self.over.load(Ordering::Relaxed);
        if o != 0 {
            let until = self.until.load(Ordering::Relaxed);
            if until == 0 || now_ms() < until {
                return o;
            }
            self.over.store(0, Ordering::Relaxed);
        }
        self.base.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct LogHandle {
    shared: Arc<Shared>,
}

pub struct LevelChange {
    pub from: LoggingLevel,
    pub to: LoggingLevel,
    pub until: Option<u64>,
}

impl LogHandle {
    pub fn set_epoch(&self, epoch: &str) {
        let _ = self.shared.epoch.set(epoch.to_string());
    }

    pub fn level(&self) -> LoggingLevel {
        num_level(self.shared.level())
    }

    pub fn override_level(&self, to: LoggingLevel, ttl_secs: Option<u32>) -> LevelChange {
        let from = self.level();
        let until = ttl_secs.map(|t| now_ms() + u64::from(t) * 1000);
        self.shared.until.store(until.unwrap_or(0), Ordering::Relaxed);
        self.shared.over.store(level_num(to), Ordering::Relaxed);
        LevelChange { from, to, until }
    }

    pub fn counters(&self) -> &Counters {
        &self.shared.counters
    }
}

fn push(s: &Shared, line: Vec<u8>) {
    match s.tx.try_send(line) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            s.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub enum Sink {
    Stdout,
    Socket { path: PathBuf, spool: PathBuf, spool_max: u64 },
    Queue(SyncSender<Vec<u8>>),
}

pub struct LogOpts {
    pub node: String,
    pub proc_: &'static str,
    pub level: LoggingLevel,
    pub line_max: usize,
    pub queue_max: usize,
    pub sink: Sink,
}

pub fn env_level(policy: LoggingLevel) -> LoggingLevel {
    std::env::var("OX_LOG_LEVEL").ok().and_then(|v| parse_level(v.trim())).unwrap_or(policy)
}

pub fn spool_path(log_dir: &str, node: &str, proc_: &str) -> PathBuf {
    PathBuf::from(log_dir).join(format!("{node}.{proc_}.spool.log"))
}

pub fn init(opts: LogOpts) -> LogHandle {
    let _ = start();
    let (tx, rx) = match opts.sink {
        Sink::Queue(ref q) => (q.clone(), None),
        _ => {
            let (tx, rx) = sync_channel::<Vec<u8>>(opts.queue_max.max(1));
            (tx, Some(rx))
        }
    };
    let shared = Arc::new(Shared {
        node: opts.node.clone(),
        proc_: opts.proc_.to_string(),
        epoch: OnceLock::new(),
        base: AtomicU8::new(level_num(opts.level)),
        over: AtomicU8::new(0),
        until: AtomicU64::new(0),
        line_max: opts.line_max,
        tx,
        counters: Counters::default(),
    });
    if let Some(rx) = rx {
        let s2 = shared.clone();
        let sink = opts.sink;
        let _ = std::thread::Builder::new().name("ox-log".into()).spawn(move || drain(rx, sink, s2));
    }
    let layer = OxLayer { shared: shared.clone() };
    let _ = tracing_subscriber::registry().with(layer).try_init();
    LogHandle { shared }
}

fn drain(rx: Receiver<Vec<u8>>, sink: Sink, s: Arc<Shared>) {
    match sink {
        Sink::Stdout | Sink::Queue(_) => {
            let out = std::io::stdout();
            while let Ok(mut l) = rx.recv() {
                l.push(b'\n');
                let mut lock = out.lock();
                let _ = lock.write_all(&l);
                let _ = lock.flush();
            }
        }
        Sink::Socket { path, spool, spool_max } => socket_loop(rx, path, spool, spool_max, &s),
    }
}

fn socket_loop(rx: Receiver<Vec<u8>>, path: PathBuf, spool: PathBuf, spool_max: u64, s: &Shared) {
    let sock = UnixDatagram::unbound().ok();
    if let Some(so) = &sock {
        let _ = so.set_nonblocking(true);
    }
    let mut spool_f: Option<File> = None;
    let mut spool_bytes: u64 = std::fs::metadata(&spool).map(|m| m.len()).unwrap_or(0);
    let mut failing = false;
    let mut since_spooled: u64 = 0;
    let drops = || s.counters.queue_full.load(Ordering::Relaxed) + s.counters.spool_full.load(Ordering::Relaxed);
    let mut drop_base = drops();
    while let Ok(l) = rx.recv() {
        let sent = match &sock {
            Some(so) => match so.send_to(&l, &path) {
                Ok(_) => true,
                Err(e) => {
                    let c = match e.raw_os_error() {
                        Some(35) | Some(11) => &s.counters.eagain,
                        Some(40) | Some(90) => &s.counters.emsgsize,
                        _ => &s.counters.absent,
                    };
                    c.fetch_add(1, Ordering::Relaxed);
                    false
                }
            },
            None => false,
        };
        if sent {
            if failing {
                failing = false;
                let dropped = drops().saturating_sub(drop_base);
                let mut r = Record::default();
                stamp(&mut r, s, Kind::Stat);
                r.set("spooled", since_spooled.to_string());
                r.set("dropped", dropped.to_string());
                r.msg = "log sink recovered".into();
                if let Some(so) = &sock {
                    let _ = so.send_to(line::render(&r, s.line_max).as_bytes(), &path);
                }
                since_spooled = 0;
                drop_base += dropped;
            }
            continue;
        }
        failing = true;
        if spool_f.is_none() {
            spool_f = OpenOptions::new().create(true).append(true).open(&spool).ok();
        }
        let n = l.len() as u64 + 1;
        let wrote = match spool_f.as_mut() {
            Some(f) if spool_bytes + n <= spool_max => f.write_all(&l).and_then(|_| f.write_all(b"\n")).is_ok(),
            _ => false,
        };
        if wrote {
            spool_bytes += n;
            since_spooled += 1;
            s.counters.spooled.fetch_add(1, Ordering::Relaxed);
        } else {
            s.counters.spool_full.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn stamp(r: &mut Record, s: &Shared, kind: Kind) {
    let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string();
    r.set("ts", ts);
    r.set("mono", mono_us().to_string());
    r.set("node", s.node.clone());
    r.set("proc", s.proc_.clone());
    r.set("kind", kind.as_str());
    if s.proc_ == "sfu"
        && r.get("epoch").is_none()
        && let Some(e) = s.epoch.get()
    {
        r.set("epoch", e.clone());
    }
}

struct OxLayer {
    shared: Arc<Shared>,
}

fn ours(target: &str) -> bool {
    OUR_CRATES.iter().any(|c| target == *c || target.strip_prefix(c).is_some_and(|rest| rest.starts_with("::")))
}

impl<S: Subscriber> Layer<S> for OxLayer {
    fn enabled(&self, meta: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        let t = meta.target();
        if t == TARGET_EV || t == TARGET_STAT {
            return true;
        }
        let lv = tracing_num(meta.level());
        if ours(t) {
            return lv <= self.shared.level();
        }
        if t.starts_with("zenoh") {
            return lv <= 3;
        }
        lv <= 2
    }

    fn register_callsite(&self, _meta: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let t = event.metadata().target();
        let kind = if t == TARGET_EV {
            Kind::Ev
        } else if t == TARGET_STAT {
            Kind::Stat
        } else {
            Kind::Diag
        };
        let mut r = Record::default();
        let mut v = Collect { r: &mut r };
        event.record(&mut v);
        if kind != Kind::Ev {
            r.fields.retain(|(k, _)| k != "spec");
        }
        if kind == Kind::Diag && !ours(t) {
            r.msg = format!("{t}: {}", r.msg);
        }
        stamp(&mut r, &self.shared, kind);
        push(&self.shared, line::render(&r, self.shared.line_max).into_bytes());
    }
}

struct Collect<'a> {
    r: &'a mut Record,
}

impl Visit for Collect<'_> {
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.put(f.name(), format!("{v:?}"));
    }

    fn record_str(&mut self, f: &Field, v: &str) {
        self.put(f.name(), v.to_string());
    }

    fn record_u64(&mut self, f: &Field, v: u64) {
        self.put(f.name(), v.to_string());
    }

    fn record_i64(&mut self, f: &Field, v: i64) {
        self.put(f.name(), v.to_string());
    }

    fn record_bool(&mut self, f: &Field, v: bool) {
        self.put(f.name(), if v { "1".into() } else { "0".into() });
    }

    fn record_f64(&mut self, f: &Field, v: f64) {
        self.put(f.name(), format!("{v}"));
    }
}

impl Collect<'_> {
    fn put(&mut self, name: &str, v: String) {
        if name == "message" {
            self.r.msg = v;
        } else if name == "in_args" {
            for p in v.split(',').filter(|p| !p.is_empty()) {
                if let Some((k, val)) = p.split_once('=') {
                    self.r.ins.push((k.to_string(), val.to_string()));
                }
            }
        } else {
            self.r.set(name, v);
        }
    }
}

pub struct Step<'a> {
    pub area: &'a str,
    pub key: &'a str,
    pub from: &'a str,
    pub to: Option<&'a str>,
    pub cause: &'a str,
    pub label: Option<&'a str>,
    pub ins: &'a [(&'a str, String)],
    pub room: Option<&'a str>,
    pub user: Option<&'a str>,
    pub arev: Option<u64>,
}

pub fn transition(s: &Step<'_>) {
    let ins: String = s.ins.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",");
    let room = s.room.unwrap_or("");
    let user = s.user.unwrap_or("");
    let arev = s.arev.map(|a| a.to_string()).unwrap_or_default();
    match s.label {
        Some(l) => crate::ev!(
            spec = l,
            area = s.area,
            key = s.key,
            from = s.from,
            to = s.to.unwrap_or(""),
            cause = s.cause,
            room = room,
            user = user,
            arev = arev.as_str(),
            in_args = ins.as_str()
        ),
        None => crate::ev!(
            area = s.area,
            key = s.key,
            from = s.from,
            cause = s.cause,
            impossible = "1",
            room = room,
            user = user,
            arev = arev.as_str(),
            in_args = ins.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_sha256_prefix8() {
        assert_eq!(fingerprint("abc"), "ba7816bf");
    }

    #[test]
    fn levels_parse() {
        assert_eq!(parse_level("debug"), Some(LoggingLevel::Debug));
        assert_eq!(parse_level("loud"), None);
        assert_eq!(level_name(LoggingLevel::Warn), "warn");
    }

    #[test]
    fn our_targets() {
        assert!(ours("oxsfud::fwd"));
        assert!(ours("oxhubd"));
        assert!(!ours("oxsfudx"));
        assert!(!ours("zenoh::net"));
    }
}
