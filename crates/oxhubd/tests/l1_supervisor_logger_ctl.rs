// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-OPS-SUPERVISOR · 정:RULE-OPS-LOGGER · 운영:RULE-CTL-SUPERVISOR · model: claude-opus-5-5
//! 1층 감독자 — 로거 유닛(hub 안 · 정지 장치 없음)에 온 `load` · `stop` · `kill` 은 거절로 답하고 상태를 바꾸지 않는다. 이루지 않은 `to` 를 답하지 않는다(context 202611/20261124b 수정 갈래 B I11).

use std::time::Duration;

use common::system::{Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

#[tokio::test]
async fn the_logger_unit_refuses_stop_kill_and_load_and_keeps_running() {
    let (tx, mut fatal) = tokio::sync::mpsc::unbounded_channel();
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
    s.mark_state(Role::Log, UnitState::Running, "bound");
    for which in ["stop", "kill", "load"] {
        let got = tokio::time::timeout(Duration::from_secs(2), s.request("log", which)).await;
        assert!(matches!(&got, Ok(Err(_))), "S-sym-RULE-OPS-SUPERVISOR-6 {which} on the logger unit is refused — it has no stop device, so it does not answer a to it never reached: {got:?}");
        assert_eq!(s.unit("log").map(|u| u.state()), Some(UnitState::Running), "S-sym-RULE-OPS-SUPERVISOR-6 the refused {which} leaves the logger Running");
    }
    assert!(fatal.try_recv().is_err(), "S-sym-RULE-OPS-SUPERVISOR-6 a logger command does not end the hub — only the bus unit's stop · kill does");
}
