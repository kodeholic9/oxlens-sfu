// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-OPS-LOGGER · 정:RULE-OPS-SUPERVISOR · model: claude-opus-5-5
//! 1층 소스 가드 · 감독자 — 로거 유닛은 소켓을 묶고 파일을 연 뒤에야 `Running` 이다: `logger::start` 가 bind · 파일 열기를 마치고 돌아온 뒤에 `Running` 으로 적고, sfud 는 그 뒤에 뜬다(bind 전에 sfud 가 보낸 줄이 spool 로 샐 창 0) · 로거가 `Running` 이 되기 전에는 준비 판정이 거짓이다(`/healthz/ready` 503)(context 202611/20261124b 서버 규칙 갈래).

use std::time::Duration;

use common::system::{Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

fn src(rel: &str) -> Vec<String> {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).expect("source").lines().map(|l| l.trim().to_string()).collect()
}

fn at(lines: &[String], needle: &str) -> usize {
    let hits: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains(needle)).map(|(i, _)| i).collect();
    assert_eq!(hits.len(), 1, "stand check — {needle} once ({hits:?})");
    hits[0]
}

#[test]
fn the_logger_binds_and_opens_before_it_is_running_and_before_sfud_starts() {
    let lg = src("src/ops/logger.rs");
    let s = lg.iter().position(|l| l.starts_with("pub fn start(")).expect("stand check — logger::start");
    let body = &lg[s..];
    let end = body.iter().position(|l| l == "Ok(())").expect("stand check — start returns Ok");
    let bind = body.iter().position(|l| l.contains("UnixDatagram::bind(")).expect("stand check — bind");
    let open = body.iter().position(|l| l.contains("FileWriter::open(")).expect("stand check — file open");
    let spawn = body.iter().position(|l| l.contains("thread::Builder::new()")).expect("stand check — worker threads");
    assert!(bind < spawn && open < spawn && spawn < end, "S-sym-RULE-OPS-LOGGER-2 logger::start binds the socket and opens the file synchronously before it returns (bind {bind} · open {open} · threads {spawn} · Ok {end})");
    let hub = src("src/main.rs");
    let (starting, start, running, sfu) = (
        at(&hub, "sup.mark_state(Role::Log, UnitState::Starting"),
        at(&hub, "ops::logger::start("),
        hub.iter().position(|l| l.contains("mark_state(Role::Log, UnitState::Running")).expect("stand check — the logger is marked Running"),
        at(&hub, "sup.start_sfu("),
    );
    assert!(hub[running].contains("\"bound\"") && starting < start && start < running && running < sfu, "S-sym-RULE-OPS-LOGGER-2 the hub marks the logger Running only after start returned, and starts sfud after that ({starting} · {start} · {running} · {sfu})");
}

#[test]
fn ready_is_false_while_the_logger_is_starting() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SupCfg {
        node: "n1".into(),
        hub_inst: "i1".into(),
        build_label: None,
        backoff_ms: vec![100],
        intensity_max: 5,
        intensity_window: Duration::from_secs(60),
        hello_timeout: Duration::from_secs(1),
        extra_args: Vec::new(),
        sfu_args: Vec::new(),
        rec: None,
    };
    let log = Unit { id: "log".into(), kind: UnitKind::Inproc, order: 0, role: "log".into(), addr: None, cmd: Vec::new(), enabled: true };
    let s = Supervisor::new(&[log], cfg, tx);
    s.mark_state(Role::Bus, UnitState::Running, "l1");
    s.mark_state(Role::Log, UnitState::Starting, "start");
    assert_eq!(s.down_units(), vec!["log".to_string()], "S-sym-RULE-OPS-LOGGER-2 before the logger is bound the node is not ready (503)");
    s.mark_state(Role::Log, UnitState::Running, "bound");
    assert!(s.down_units().is_empty(), "S-sym-RULE-OPS-LOGGER-2 ready once the logger is bound (positive control)");
}
