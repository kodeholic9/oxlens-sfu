// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-LOG-PRINCIPLE · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-TRANSITION · 운영:LOG.log.LINE · 정:CAND-OPS-LOG-ENV · model: claude-opus-5-5
//! 로그 — 줄 꼴 · 보내는 쪽 · Layer · 배너 · 전이 줄 · 세션 지문.

pub mod banner;
pub mod layer;
pub mod line;
pub mod sender;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub use line::{Kind, Line};
pub use sender::{Counters, Sender, SenderConfig, Sink};

use crate::policy::{LoggingLevel, Policy};

static GLOBAL: OnceLock<Arc<Sender>> = OnceLock::new();

pub struct Setup {
    pub node: String,
    pub proc_name: &'static str,
    pub epoch: Option<String>,
    pub sink: Sink,
    pub log_dir: Option<PathBuf>,
}

pub fn level_from_env(policy: &Policy) -> LoggingLevel {
    std::env::var("OX_LOG_LEVEL").ok().and_then(|v| LoggingLevel::parse(v.trim())).unwrap_or(policy.logging.level)
}

pub fn init(setup: Setup, policy: &Policy) -> Arc<Sender> {
    let sender = Sender::start(SenderConfig {
        node: setup.node,
        proc_name: setup.proc_name,
        epoch: setup.epoch,
        sink: setup.sink,
        log_dir: setup.log_dir,
        line_max: policy.logging.line_max_bytes as usize,
        queue_max: policy.logging.queue_max_lines as usize,
        spool_max: policy.logging.spool_max_bytes,
        level: level_from_env(policy),
    });
    let _ = GLOBAL.set(sender.clone());
    use tracing_subscriber::layer::SubscriberExt;
    let sub = tracing_subscriber::registry().with(layer::OxLayer { sender: sender.clone() });
    let _ = tracing::subscriber::set_global_default(sub);
    sender
}

pub fn global() -> Option<&'static Arc<Sender>> {
    GLOBAL.get()
}

pub fn emit(line: Line) {
    if let Some(s) = GLOBAL.get() {
        s.emit(line);
    }
}

pub fn fingerprint(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(session_id.as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

pub struct Transition<'a> {
    pub area: &'a str,
    pub key: &'a str,
    pub spec: Option<&'a str>,
    pub from: &'a str,
    pub to: Option<&'a str>,
    pub cause: &'a str,
    pub inputs: &'a [(&'a str, String)],
}

impl Transition<'_> {
    pub fn line(&self) -> Line {
        let mut l = Line::new(Kind::Ev);
        l.spec = self.spec.map(|s| s.to_string());
        l = l.rec("area", self.area).rec("key", self.key).rec("from", self.from);
        if let Some(t) = self.to {
            l = l.rec("to", t);
        }
        l = l.rec("cause", self.cause);
        if self.spec.is_none() {
            l = l.rec("impossible", 1);
        }
        for (k, v) in self.inputs {
            l = l.rec(&format!("in.{k}"), v);
        }
        l
    }
}

pub fn bool_fields_state(fields: &[(&str, bool)]) -> String {
    let on: Vec<&str> = fields.iter().filter(|(_, b)| *b).map(|(n, _)| *n).collect();
    if on.is_empty() { "-".to_string() } else { on.join("+") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_8_hex() {
        let f = fingerprint("abc");
        assert_eq!(f.len(), 8);
        assert_eq!(f, "ba7816bf");
    }

    #[test]
    fn transition_impossible_has_no_spec_to() {
        let t = Transition { area: "FLR", key: "r1", spec: None, from: "IDLE", to: None, cause: "RELEASE", inputs: &[] };
        let s = t.line().render(&line::Head { node: "n".into(), proc_name: "sfu" }, "T", 0, 1024);
        assert!(s.contains("impossible=1"));
        assert!(!s.contains("spec="));
        assert!(!s.contains(" to="));
        assert_eq!(bool_fields_state(&[("bound", true), ("resumed", false), ("live", true)]), "bound+live");
    }
}
