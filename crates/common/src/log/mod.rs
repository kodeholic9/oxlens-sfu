// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:RULE-OPS-OBSERVE · 정:RULE-CONC-SIX · 운영 규격서 §7 · model: claude-opus-5-5
//! 로깅 — `tracing` Layer 하나가 줄을 짓고(§7-3) 보내는 쪽(§7-2)이 나른다. 수준은 정책값 · 기동 환경변수 · 운영 일시 덮기(§4-4).

pub mod line;
pub mod sender;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record as SpanRecord};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::policy::{Logging, LoggingLevel};
use line::Record;
use sender::{Sender, SenderConfig};

pub const LEVEL_ENV: &str = "OX_LOG_LEVEL";
const OWN_TARGETS: &[&str] = &["oxhubd", "oxsfud", "oxlogd", "oxadmin", "common", "oxsig"];

static LEVEL: AtomicU8 = AtomicU8::new(2);
static OVERRIDE_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicU8 = AtomicU8::new(2);

fn rank(l: LoggingLevel) -> u8 {
    match l {
        LoggingLevel::Error => 0,
        LoggingLevel::Warn => 1,
        LoggingLevel::Info => 2,
        LoggingLevel::Debug => 3,
        LoggingLevel::Trace => 4,
    }
}

fn unrank(r: u8) -> LoggingLevel {
    match r {
        0 => LoggingLevel::Error,
        1 => LoggingLevel::Warn,
        2 => LoggingLevel::Info,
        3 => LoggingLevel::Debug,
        _ => LoggingLevel::Trace,
    }
}

fn level_rank(l: &Level) -> u8 {
    match *l {
        Level::ERROR => 0,
        Level::WARN => 1,
        Level::INFO => 2,
        Level::DEBUG => 3,
        Level::TRACE => 4,
    }
}

pub fn level() -> LoggingLevel {
    let until = OVERRIDE_UNTIL_MS.load(Ordering::Relaxed);
    if until != 0 && crate::time::wall_ms() >= until {
        OVERRIDE_UNTIL_MS.store(0, Ordering::Relaxed);
        LEVEL.store(BASE.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    unrank(LEVEL.load(Ordering::Relaxed))
}

pub fn base_level() -> LoggingLevel {
    unrank(BASE.load(Ordering::Relaxed))
}

pub fn override_level(to: LoggingLevel, ttl_secs: Option<u64>) -> (LoggingLevel, Option<u64>) {
    let from = level();
    LEVEL.store(rank(to), Ordering::Relaxed);
    let until = ttl_secs.map(|t| crate::time::wall_ms() + t * 1000);
    OVERRIDE_UNTIL_MS.store(until.unwrap_or(0), Ordering::Relaxed);
    (from, until)
}

#[derive(Debug, Clone)]
pub struct LogInit {
    pub node: String,
    pub proc_: &'static str,
    pub log_dir: Option<PathBuf>,
    pub log_socket: Option<PathBuf>,
    pub policy: Logging,
}

#[derive(Clone)]
pub struct LogHandle {
    pub sender: Sender,
}

struct OxLayer {
    node: String,
    proc_: &'static str,
    sender: Sender,
    line_max: usize,
}

struct SpanFields(Record);

struct Visitor<'a>(&'a mut Record);

impl Visit for Visitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.set(field.name(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.set(field.name(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.set(field.name(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.set(field.name(), if value { "1".into() } else { "0".into() });
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.set(field.name(), format!("{value}"));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.set(field.name(), format!("{value:?}"));
    }
}

fn own(target: &str) -> bool {
    OWN_TARGETS.iter().any(|t| target == *t || target.starts_with(&format!("{t}::")))
}

impl<S> Layer<S> for OxLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn register_callsite(&self, _m: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(&self, m: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        if m.is_span() {
            return true;
        }
        let lim = if own(m.target()) { rank(level()) } else { rank(LoggingLevel::Warn) };
        level_rank(m.level()) <= lim
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut r = Record::default();
        attrs.record(&mut Visitor(&mut r));
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(r));
        }
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            if let Some(f) = ext.get_mut::<SpanFields>() {
                values.record(&mut Visitor(&mut f.0));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut r = Record::default();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(f) = span.extensions().get::<SpanFields>() {
                    for (k, v) in &f.0.fields {
                        r.set(k, v.clone());
                    }
                }
            }
        }
        event.record(&mut Visitor(&mut r));
        let out = line::render(&line::now_ts(), crate::time::mono_us(), &self.node, self.proc_, &r, self.line_max);
        self.sender.push(out.into_bytes());
    }
}

pub fn init(cfg: LogInit) -> LogHandle {
    let mut base = cfg.policy.level;
    if let Some(l) = std::env::var(LEVEL_ENV).ok().and_then(|v| LoggingLevel::parse(&v)) {
        base = l;
    }
    BASE.store(rank(base), Ordering::Relaxed);
    LEVEL.store(rank(base), Ordering::Relaxed);
    let spool = cfg.log_dir.as_ref().map(|d| d.join(format!("{}.{}.spool.log", cfg.node, cfg.proc_)));
    let node = cfg.node.clone();
    let proc_ = cfg.proc_;
    let line_max = cfg.policy.line_max_bytes as usize;
    let stat = Arc::new(move |spooled: u64, dropped: u64| {
        let mut r = Record::default();
        r.set("kind", "stat".into());
        r.set("spooled", spooled.to_string());
        r.set("dropped", dropped.to_string());
        line::render(&line::now_ts(), crate::time::mono_us(), &node, proc_, &r, line_max)
    });
    let sender = Sender::start(
        SenderConfig { socket: cfg.log_socket.clone(), spool, queue_max: cfg.policy.queue_max_lines as usize, spool_max: cfg.policy.spool_max_bytes },
        stat,
    );
    let layer = OxLayer { node: cfg.node, proc_: cfg.proc_, sender: sender.clone(), line_max };
    let _ = tracing_subscriber::registry().with(layer).try_init();
    LogHandle { sender }
}
