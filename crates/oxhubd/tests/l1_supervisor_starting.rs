// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! 1층 감독자 — 유닛은 `Starting` 을 거쳐서만 `Running` 이 된다: `Running` 을 적는 자리마다 같은 흐름에서 `Starting` 이 먼저다(감독 고리 · 프로세스 안 유닛 · 게이트웨이 기동) · 준비 신호는 게이트웨이가 띄운 유닛에 hello 로 묻는 것뿐이라 밖에서 넣는 길이 없다 · `Running` 이 되기 전 창(`Inactive` · `Starting`)에는 준비 판정이 거짓이다(`/healthz/ready` 503)(context 202611/20261124b 서버 규칙 갈래).

use std::time::Duration;

use common::system::{Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

fn src(rel: &str) -> Vec<String> {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).expect("source").lines().map(|l| l.trim().to_string()).collect()
}

fn body<'a>(lines: &'a [String], head: &str) -> &'a [String] {
    let s = lines.iter().position(|l| l.contains(head)).unwrap_or_else(|| panic!("stand check — {head}"));
    let e = lines[s + 1..].iter().position(|l| l.starts_with("fn ") || l.starts_with("pub fn ") || l.starts_with("async fn ") || l.starts_with("pub async fn ")).map(|i| s + 1 + i).unwrap_or(lines.len());
    &lines[s..e]
}

fn first(lines: &[String], needle: &str) -> Option<usize> {
    lines.iter().position(|l| l.contains(needle))
}

#[test]
fn running_is_only_written_after_starting() {
    let sup = src("src/ops/supervisor.rs");
    let mr = body(&sup, "pub fn mark_running(");
    assert!(first(mr, "set(UnitState::Starting").zip(first(mr, "set(UnitState::Running")).is_some_and(|(a, b)| a < b), "S-sym-RULE-OPS-SUPERVISOR-2① in-process units go Starting then Running");
    let ru = body(&sup, "async fn run_unit(");
    let starting = first(ru, "u.set(UnitState::Starting, \"start\")").expect("stand check — run_unit starts its loop with Starting");
    let running: Vec<usize> = ru.iter().enumerate().filter(|(_, l)| l.contains("set(UnitState::Running") || l.contains("became_running(")).map(|(i, _)| i).collect();
    assert!(!running.is_empty() && running.iter().all(|&i| i > starting), "S-sym-RULE-OPS-SUPERVISOR-2① the supervised loop writes Running (or calls became_running) only after Starting: {running:?} vs {starting}");
    let callers: Vec<&String> = sup.iter().filter(|l| l.contains("became_running(") && !l.contains("fn became_running")).collect();
    assert_eq!(callers.len(), 1, "S-sym-RULE-OPS-SUPERVISOR-2① became_running is reached only from the supervised loop: {callers:?}");
    let hub = src("src/main.rs");
    for role in ["Role::Bus", "Role::Log"] {
        let s = first(&hub, &format!("mark_state({role}, UnitState::Starting"));
        let r = first(&hub, &format!("mark_state({role}, UnitState::Running"));
        assert!(s.zip(r).is_some_and(|(s, r)| s < r), "S-sym-RULE-OPS-SUPERVISOR-2① {role} is marked Starting before Running at boot ({s:?} · {r:?})");
    }
    let pushes: Vec<&String> = sup.iter().chain(hub.iter()).filter(|l| l.contains("UnitState::Running") && l.contains("route(")).collect();
    assert!(pushes.is_empty(), "S-sym-RULE-OPS-SUPERVISOR-2① no route pushes a unit to Running from outside: {pushes:?}");
}

#[test]
fn ready_is_false_until_the_unit_is_running() {
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
    let unit = Unit { id: "n1".into(), kind: UnitKind::Process, order: 3, role: "sfu".into(), addr: None, cmd: vec!["true".into()], enabled: true };
    let s = Supervisor::new(&[unit], cfg, tx);
    s.mark_state(Role::Bus, UnitState::Running, "l1");
    assert_eq!(s.unit("n1").map(|u| u.state()), Some(UnitState::Inactive), "stand check — a unit never started is Inactive");
    assert_eq!(s.down_units(), vec!["n1".to_string()], "S-sym-RULE-OPS-SUPERVISOR-2② an Inactive media server keeps the node out of ready (503)");
    s.mark_state(Role::Sfu, UnitState::Starting, "l1");
    assert_eq!(s.down_units(), vec!["n1".to_string()], "S-sym-RULE-OPS-SUPERVISOR-2② the Starting window is still not ready (503)");
    s.mark_state(Role::Sfu, UnitState::Running, "l1");
    assert!(s.down_units().is_empty(), "S-sym-RULE-OPS-SUPERVISOR-2② ready only once the unit is Running (positive control)");
}
