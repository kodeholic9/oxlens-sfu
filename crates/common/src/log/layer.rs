// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-CONTEXT · 운영:RULE-LOG-LEVEL · 운영:RULE-LOG-KEYS · 정:RULE-OPS-LOGGER · 정:CAND-OPS-FOREIGN-LOG · model: claude-opus-5-5
//! `tracing` Layer 하나 — span 칸(session · room · user · epoch)을 줄에 붙이고 줄을 지어 보내는 쪽 채널에 넣는다. 남의 크레이트는 `warn` 이상 · zenoh 는 `info` 까지 · hub 안 zenoh 유닛이 낸 줄은 상관 키 `unit=zenoh`.

use std::fmt::Write as _;

use tracing::field::{Field, Visit};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::line::{Head, Kind, Record, render};
use super::sender::Tx;

pub const EV_TARGET: &str = "ox::ev";
pub const STAT_TARGET: &str = "ox::stat";
const OURS: &[&str] = &["ox::", "oxhubd", "oxsfud", "oxadmin", "common", "oxsig"];
pub const HUB_ZENOH_UNIT: &str = "zenoh";

pub fn ours(target: &str) -> bool {
    OURS.iter().any(|p| target.starts_with(p))
}

fn foreign_allows(m: &Metadata<'_>) -> bool {
    let cap = if m.target().starts_with("zenoh") { &Level::INFO } else { &Level::WARN };
    m.level() <= cap
}

#[derive(Default)]
struct Fields(Vec<(String, String)>, Option<String>);

impl Visit for Fields {
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{v:?}");
        self.put(f, s);
    }

    fn record_str(&mut self, f: &Field, v: &str) {
        self.put(f, v.to_string());
    }

    fn record_i64(&mut self, f: &Field, v: i64) {
        self.put(f, v.to_string());
    }

    fn record_u64(&mut self, f: &Field, v: u64) {
        self.put(f, v.to_string());
    }

    fn record_bool(&mut self, f: &Field, v: bool) {
        self.put(f, if v { "1".to_string() } else { "0".to_string() });
    }

    fn record_f64(&mut self, f: &Field, v: f64) {
        self.put(f, v.to_string());
    }
}

impl Fields {
    fn put(&mut self, f: &Field, v: String) {
        if f.name() == "message" {
            self.1 = Some(v);
        } else {
            self.0.push((f.name().to_string(), v));
        }
    }
}

struct SpanFields(Vec<(String, String)>);

pub struct OxLayer {
    pub head: Head,
    pub tx: Tx,
    pub line_max: usize,
}

impl<S> tracing_subscriber::Layer<S> for OxLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn register_callsite(&self, m: &'static Metadata<'static>) -> Interest {
        if ours(m.target()) {
            Interest::sometimes()
        } else if foreign_allows(m) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, m: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        if !ours(m.target()) {
            return foreign_allows(m);
        }
        if m.is_span() || m.target() == EV_TARGET || m.target() == STAT_TARGET {
            return true;
        }
        super::level::allows(m.level(), crate::time::mono_ms())
    }

    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let mut f = Fields::default();
        attrs.record(&mut f);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(f.0));
        }
    }

    fn on_record(&self, id: &tracing::span::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        let mut f = Fields::default();
        values.record(&mut f);
        if let Some(span) = ctx.span(id) {
            let mut ext = span.extensions_mut();
            if let Some(sf) = ext.get_mut::<SpanFields>() {
                sf.0.extend(f.0);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let m = event.metadata();
        let kind = match m.target() {
            EV_TARGET => Kind::Ev,
            STAT_TARGET => Kind::Stat,
            _ => Kind::Diag,
        };
        let mut fields: Vec<(String, String)> = Vec::new();
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                if let Some(sf) = span.extensions().get::<SpanFields>() {
                    fields.extend(sf.0.iter().cloned());
                }
            }
        }
        let mut f = Fields::default();
        event.record(&mut f);
        fields.extend(f.0);
        if self.head.proc_ == "hub" && m.target().starts_with("zenoh") && !fields.iter().any(|(k, _)| k == "unit") {
            fields.push(("unit".to_string(), HUB_ZENOH_UNIT.to_string()));
        }
        let ts = crate::time::iso_now();
        let line = render(&self.head, &Record { ts: &ts, mono: crate::time::mono_us(), kind, fields: &fields, msg: f.1.as_deref() }, self.line_max);
        self.tx.send(line);
    }
}
