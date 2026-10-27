// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-PRINCIPLE · 운영:RULE-LOG-SENDER · 운영:RULE-LOG-SPEC · 운영:RULE-LOG-TRANSITION · 정:RULE-OPS-LOGGER · 정:RULE-OPS-SUPERVISOR · model: claude-opus-5-5
//! 로그 — 한 곳에 모은다 · 보내는 쪽은 기다리지 않는다 · 규격에 이름이 있으면 `ev`. `ev!` · `stat!` 은 수준과 상관없이 늘 나간다.

pub mod layer;
pub mod level;
pub mod line;
pub mod sender;

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_subscriber::layer::SubscriberExt;

use crate::policy::{Logging, LoggingLevel};

pub use line::fingerprint;

pub const SPEC_UNIT_STATE: &str = "SV-STATE-UNIT-STATE";
pub const SPEC_PEER_STATE: &str = "SV-STATE-PEER-STATE";
pub const SPEC_PUBLISH_STATE: &str = "SV-STATE-PUBLISH-STATE";
pub const SPEC_SUBSCRIBE_STATE: &str = "SV-STATE-SUBSCRIBE-STATE";

static HEAD: OnceLock<line::Head> = OnceLock::new();
static LINE_MAX: OnceLock<usize> = OnceLock::new();
static TX: OnceLock<sender::Tx> = OnceLock::new();

#[macro_export]
macro_rules! ev {
    ($($t:tt)*) => { ::tracing::info!(target: "ox::ev", $($t)*) };
}

#[macro_export]
macro_rules! stat {
    ($($t:tt)*) => { ::tracing::info!(target: "ox::stat", $($t)*) };
}

#[derive(Debug)]
pub struct Init {
    pub node: String,
    pub proc_: &'static str,
    pub epoch: Option<String>,
    pub logging: Logging,
    pub sink: sender::Sink,
}

pub fn sink_for(proc_: &str, node: &str, log_dir: Option<&str>, log_socket: Option<&str>, logging: &Logging) -> sender::Sink {
    match log_socket {
        None => sender::Sink::Stdout,
        Some(sock) => {
            let dir = log_dir.map(PathBuf::from).or_else(|| PathBuf::from(sock).parent().map(PathBuf::from)).unwrap_or_else(std::env::temp_dir);
            sender::Sink::Socket { path: PathBuf::from(sock), spool: dir.join(format!("{node}.{proc_}.spool.log")), spool_max: logging.spool_max_bytes }
        }
    }
}

pub fn hub_sink(node: &str, log_dir: Option<&str>, log_socket: Option<&str>, logging: &Logging) -> (sender::Sink, Option<std::sync::Arc<sender::Gate>>) {
    match log_socket {
        None => (sender::Sink::Stdout, None),
        Some(sock) => {
            let dir = log_dir.map(PathBuf::from).or_else(|| PathBuf::from(sock).parent().map(PathBuf::from)).unwrap_or_else(std::env::temp_dir);
            let gate = sender::Gate::new();
            (sender::Sink::Hub { gate: gate.clone(), spool: dir.join(format!("{node}.hub.spool.log")), spool_max: logging.spool_max_bytes }, Some(gate))
        }
    }
}

pub fn init(i: Init) -> LoggingLevel {
    let lvl = level::boot(i.logging.level);
    let head = line::Head { node: i.node, proc_: i.proc_.to_string(), epoch: i.epoch };
    let _ = HEAD.set(head.clone());
    let _ = LINE_MAX.set(i.logging.line_max_bytes as usize);
    let tx = sender::start(i.sink, i.logging.queue_max_lines as usize);
    let _ = TX.set(tx.clone());
    let layer = layer::OxLayer { head, tx, line_max: i.logging.line_max_bytes as usize };
    let sub = tracing_subscriber::registry().with(layer);
    let _ = tracing::subscriber::set_global_default(sub);
    lvl
}

pub fn recovery_line(spooled: u64, dropped: u64) -> String {
    let head = HEAD.get().cloned().unwrap_or_default();
    let fields = vec![("spooled".to_string(), spooled.to_string()), ("dropped".to_string(), dropped.to_string())];
    let ts = crate::time::iso_now();
    line::render(&head, &line::Record { ts: &ts, mono: crate::time::mono_us(), kind: line::Kind::Stat, fields: &fields, msg: None }, LINE_MAX.get().copied().unwrap_or(1024))
}

pub fn render_local(kind: line::Kind, fields: &[(String, String)], msg: Option<&str>) -> String {
    let head = HEAD.get().cloned().unwrap_or_default();
    let ts = crate::time::iso_now();
    line::render(&head, &line::Record { ts: &ts, mono: crate::time::mono_us(), kind, fields, msg }, LINE_MAX.get().copied().unwrap_or(1024))
}

pub fn unit_transition(unit: &str, from: &str, to: &str, cause: &str) {
    crate::ev!(spec = SPEC_UNIT_STATE, unit = unit, area = "OPS", key = unit, from = from, to = to, cause = cause);
}

pub fn banner_lines(head: &line::Head, ts: &str, mono: u64, build: &str, epoch: &str, banner: &crate::banner::Banner, line_max: usize) -> Result<Vec<String>, String> {
    let line_of = |part: usize, parts: usize, msg: &str| {
        let mut fields = vec![("spec".to_string(), SPEC_UNIT_STATE.to_string())];
        if part == 1 {
            fields.push(("epoch".to_string(), epoch.to_string()));
            fields.push(("build".to_string(), build.to_string()));
        }
        fields.push(("part".to_string(), part.to_string()));
        fields.push(("parts".to_string(), parts.to_string()));
        line::render(head, &line::Record { ts, mono, kind: line::Kind::Ev, fields: &fields, msg: Some(msg) }, usize::MAX)
    };
    banner.split(line_max, &line_of).map_err(|key| format!("기동 배너 칸 `{key}` 하나가 줄 상한 logging.line_max_bytes={line_max} 을 홀로 넘는다(운영:RULE-BOOT-ARGS — 기동 실패)"))
}

pub fn boot_lines(build: &str, epoch: &str, banner: &crate::banner::Banner) -> Result<(), String> {
    let head = HEAD.get().cloned().unwrap_or_default();
    let ts = crate::time::iso_now();
    let lines = banner_lines(&head, &ts, crate::time::mono_us(), build, epoch, banner, LINE_MAX.get().copied().unwrap_or(1024))?;
    if let Some(tx) = TX.get() {
        for l in lines {
            tx.send(l);
        }
    }
    Ok(())
}
