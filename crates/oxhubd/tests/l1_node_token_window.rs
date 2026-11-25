// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:RULE-OPS-SURFACE · 정:RULE-BUS-PRESENCE · model: claude-opus-5-5
//! 1층 소스 가드 · 감독자 — 노드의 `node` 토큰은 그 hub 의 미디어 서버 유닛이 처음 `Running` 이 된 뒤에야 선다(`became_running` 이 `Running` 을 적은 다음 `declare_self` · 그 밖에 선언하는 자리 0) · 그래서 토큰이 없는 창(기동 중 · 미디어 서버가 아직 `Running` 이 아님)에는 그 노드의 준비 판정이 거짓이다(`/healthz/ready` 503) · 유닛이 `Running` 이 되면 참(context 202611/20261124b 서버 규칙 갈래).

use std::time::Duration;

use common::system::{Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState};

#[test]
fn the_node_token_stands_only_after_the_media_server_runs_and_ready_is_false_before() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("src").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                for (n, l) in text.split("#[cfg(test)]").next().unwrap_or("").lines().enumerate() {
                    if l.contains(".declare_self(") {
                        sites.push(format!("{}:{}", p.strip_prefix(&root).map(|x| x.display().to_string()).unwrap_or_default(), n + 1));
                    }
                }
            }
        }
    }
    assert_eq!(sites.len(), 1, "S-cond-RULE-DOC-COND-INDEX-SERVER-5③ the node token is declared in one place only: {sites:?}");
    let sup: Vec<String> = std::fs::read_to_string(root.join("ops/supervisor.rs")).expect("supervisor.rs").lines().map(|l| l.trim().to_string()).collect();
    let s = sup.iter().position(|l| l.starts_with("async fn became_running(")).expect("stand check — became_running");
    let running = sup[s..].iter().position(|l| l.contains("u.set(UnitState::Running")).expect("stand check — Running");
    let declare = sup[s..].iter().position(|l| l.contains("bus.declare_self()")).expect("S-cond-RULE-DOC-COND-INDEX-SERVER-5③ became_running declares the node token");
    assert!(running < declare && sites[0].starts_with("ops/supervisor.rs"), "S-cond-RULE-DOC-COND-INDEX-SERVER-5③ the node token is declared only after the media server unit is Running ({running} · {declare} · {sites:?})");
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
    let sv = Supervisor::new(&[unit], cfg, tx);
    sv.mark_state(Role::Bus, UnitState::Running, "l1");
    for st in [UnitState::Inactive, UnitState::Starting] {
        sv.mark_state(Role::Sfu, st, "l1");
        assert!(!sv.down_units().is_empty(), "S-cond-RULE-DOC-COND-INDEX-SERVER-5③ in the window before the token stands ({}) ready is false — 503", st.as_str());
    }
    sv.mark_state(Role::Sfu, UnitState::Running, "l1");
    assert!(sv.down_units().is_empty(), "S-cond-RULE-DOC-COND-INDEX-SERVER-5③ once the media server runs (and the token stands) ready is true — positive control");
}
