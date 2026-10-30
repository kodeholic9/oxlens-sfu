// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-CONTEXT · 운영:RULE-LOG-LEVEL · 정:RULE-OPS-LOGGER · model: claude-opus-5-5
//! `tracing` Layer 하나 — 사건 칸 + span 문맥(`session` · `room` · `user` · `epoch`)으로 줄을 짓는다 · 수준은 `diag` 만 거른다 · 남의 크레이트는 `warn` 이상(zenoh 는 `info`).

use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::line::{Kind, Line, key_index};
use super::sender::Sender;
use crate::policy::LoggingLevel;

pub struct OxLayer {
    pub sender: Arc<Sender>,
}

#[derive(Default, Clone)]
struct SpanKeys(Vec<(String, String)>);

struct Collect {
    line: Line,
    kind_set: bool,
}

impl Collect {
    fn put(&mut self, name: &str, v: String) {
        match name {
            "message" => self.line.msg = if v.is_empty() { None } else { Some(v) },
            "kind" => {
                if let Some(k) = Kind::parse(&v) {
                    self.line.kind = k;
                    self.kind_set = true;
                }
            }
            "spec" => self.line.spec = Some(v),
            "rec" => self.line.raw = Some(v),
            n if key_index(n).is_some() => {
                let i = key_index(n).unwrap_or(0);
                self.line.keys[i] = Some(v);
            }
            n => self.line.recs.push((n.to_string(), v)),
        }
    }
}

impl Visit for Collect {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.put(field.name(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field.name(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field.name(), value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field.name(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field.name(), value.to_string());
    }
}

struct SpanVisit<'a>(&'a mut Vec<(String, String)>);

impl Visit for SpanVisit<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_string(), format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_string(), value.to_string()));
    }
}

fn own_target(t: &str) -> bool {
    t.starts_with("oxhubd") || t.starts_with("oxsfud") || t.starts_with("common") || t.starts_with("oxsig") || t.starts_with("oxadmin") || t == "ox"
}

fn level_rank(l: &Level) -> u8 {
    match *l {
        Level::TRACE => 0,
        Level::DEBUG => 1,
        Level::INFO => 2,
        Level::WARN => 3,
        Level::ERROR => 4,
    }
}

impl<S> tracing_subscriber::Layer<S> for OxLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let mut v = Vec::new();
        attrs.record(&mut SpanVisit(&mut v));
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanKeys(v));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let target = meta.target();
        let rank = level_rank(meta.level());
        if !own_target(target) {
            let floor = if target.starts_with("zenoh") { 2 } else { 3 };
            if rank < floor {
                return;
            }
        }
        let mut c = Collect { line: Line::new(Kind::Diag), kind_set: false };
        event.record(&mut c);
        if !c.kind_set || c.line.kind == Kind::Diag {
            c.line.kind = Kind::Diag;
            let cur = self.sender.level(crate::now_ms());
            if rank < cur.rank() && !(cur == LoggingLevel::Trace) {
                return;
            }
            if !own_target(target) {
                c.line.recs.insert(0, ("target".to_string(), target.to_string()));
            }
        }
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(k) = span.extensions().get::<SpanKeys>() {
                    for (n, v) in &k.0 {
                        c.line.set_key_if_absent(n, v);
                    }
                }
            }
        }
        self.sender.emit(c.line);
    }
}
