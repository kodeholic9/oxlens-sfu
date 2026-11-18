// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · 정:RULE-OPS-SURFACE · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! 1층 녹음 유닛의 감독 — `role = "rec"` 유닛은 `/healthz/ready` 에 세지 않는다(미디어 서버는 센다 — 대조) · 슈퍼바이저가 띄우고 멈추는 유닛 목록에는 든다 · 게이트웨이가 녹화 참가자 토큰(`rec-{node}` · 참가자 종류 1 · 투명)을 지어 넘긴다 · rec 인자는 rec 유닛에만.

use std::time::Duration;

use common::system::{ApiKey, Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState, rec_user};
use oxhubd::ses::token::Accounts;

fn unit(id: &str, role: &str) -> Unit {
    Unit { id: id.into(), kind: UnitKind::Process, order: 3, role: role.into(), addr: None, cmd: vec!["true".into()], enabled: true }
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
fn a_rec_unit_that_is_not_running_does_not_fail_ready() {
    let s = sup(&[unit("r1", "rec")]);
    assert_eq!(s.process_units().iter().map(|u| (u.id.as_str(), u.role)).collect::<Vec<_>>(), vec![("r1", Role::Rec)], "the supervisor starts and stops the rec unit");
    assert!(s.down_units().is_empty(), "정:RULE-OPS-SURFACE S-sym-IMPL-OPS-REC-UNIT-2 — a rec unit not Running does not take the node out of ready");
    let c = sup(&[unit("n1", "sfu"), unit("r1", "rec")]);
    assert_eq!(c.down_units(), vec!["n1".to_string()], "대조 — a media server that is not Running does");
}

#[test]
fn the_hub_base_is_the_listen_and_base_path_with_loopback_for_any() {
    use oxhubd::cfg::hub_base;
    assert_eq!(hub_base("0.0.0.0:1974", "/media", false), "http://127.0.0.1:1974/media", "정:IMPL-CFG-SYSTEM-FILE --hub-base — 모든 주소면 같은 갈래의 루프백");
    assert_eq!(hub_base("[::]:1974", "/media", true), "https://[::1]:1974/media", "[hub.tls] 면 https");
    assert_eq!(hub_base("10.0.0.5:8443", "/", true), "https://10.0.0.5:8443", "listen · base_path 그대로");
}

#[test]
fn the_gateway_signs_a_hidden_recorder_token_for_the_unit() {
    let acc = Accounts { keys: vec![ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0, 1], hidden_allowed: true, ops_allowed: false, hook_allowed: false }], jwt_secret: None };
    let tok = acc.sign_unit(&rec_user("n1"), 3_600).expect("signed");
    let id = acc.verify_user(&tok).expect("valid");
    assert_eq!((id.user_id.as_str(), id.claims.participant_type, id.claims.hidden), ("rec-n1", 1, true), "정:IMPL-OPS-REC-UNIT 신원 — rec-{{node}} · participant_type 1 · hidden");
    let none = Accounts { keys: Vec::new(), jwt_secret: None };
    assert!(none.sign_unit("rec-n1", 60).is_none(), "no secret — no token (the unit is not started)");
}
