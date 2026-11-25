// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-OPS-SUPERVISOR · model: claude-opus-5-5
//! 1층 감독자 — `enabled=false` 유닛은 `Disabled` 로 서고 그 값은 `Stopped` 와 다르다(준비 판정에서도 갈린다) · 그 유닛에 온 명령은 바로 거절로 답하고 사유를 싣는다(context 202611/20261124b 서버 규칙 갈래 · 수정 갈래 B I17).

use std::time::Duration;

use common::system::{Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

fn unit(id: &str, enabled: bool) -> Unit {
    Unit { id: id.into(), kind: UnitKind::Process, order: 3, role: "sfu".into(), addr: None, cmd: vec!["true".into()], enabled }
}

fn sup(units: &[Unit]) -> std::sync::Arc<Supervisor> {
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
    let s = Supervisor::new(units, cfg, tx);
    s.mark_state(Role::Bus, UnitState::Running, "l1");
    s
}

#[test]
fn a_disabled_unit_is_disabled_not_stopped() {
    let s = sup(&[unit("d1", false), unit("e1", true)]);
    let st = |id: &str| s.unit(id).map(|u| u.state());
    assert_eq!(st("d1"), Some(UnitState::Disabled), "S-sym-RULE-OPS-SUPERVISOR-3① an enabled=false unit stands as Disabled");
    assert_ne!(UnitState::Disabled.as_str(), UnitState::Stopped.as_str(), "S-sym-RULE-OPS-SUPERVISOR-3① Disabled is a value of its own, not Stopped");
    assert_eq!(s.down_units(), vec!["e1".to_string()], "S-sym-RULE-OPS-SUPERVISOR-3① Disabled does not take the node out of ready — a stopped enabled unit would");
}

#[tokio::test]
async fn a_disabled_unit_refuses_load_at_once_with_the_reason() {
    let s = sup(&[unit("d1", false)]);
    for which in ["load", "stop", "kill"] {
        let got = tokio::time::timeout(Duration::from_secs(2), s.request("d1", which)).await;
        assert!(matches!(&got, Ok(Err(e)) if e.contains("disabled")), "S-sym-RULE-OPS-SUPERVISOR-3② {which} on a disabled unit is answered at once as a refusal that names the reason: {got:?}");
    }
    assert_eq!(s.unit("d1").map(|u| u.state()), Some(UnitState::Disabled), "S-sym-RULE-OPS-SUPERVISOR-3② the refused load does not take — the unit stays Disabled");
}

