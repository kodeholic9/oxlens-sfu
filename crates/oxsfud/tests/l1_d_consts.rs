// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 1층 쓰는 곳 감사 — 시험 항목서의 판정 조건 가운데 「누가 쓰는가」를 제품 소스로 재는 것(context 202611/20261124b 넷째 갈래). 감사는 `support::product_lines`(파일 · 함수 · 줄)로 부르는 자리를 모아 원천이 정한 쓰는 곳과 맞댄다.

mod support;

use support::*;

fn callers(src: &[Src], pat: &str, def: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = src.iter().filter(|l| l.text.contains(pat) && !l.text.contains(def)).map(|l| (l.file.clone(), l.func.clone())).collect();
    v.sort();
    v.dedup();
    v
}

fn pair(f: &str, g: &str) -> (String, String) {
    (f.to_string(), g.to_string())
}

#[test]
fn the_layer_state_is_written_only_on_the_row_worker_paths() {
    let src = product_lines();
    assert!(src.len() > 1_000, "stand check — the product sources were read ({})", src.len());
    let stores = callers(&src, "lstate.store(", "fn put(");
    assert_eq!(stores, vec![pair("sim/mod.rs", "put")], "S-sym-IMPL-SIM-SWITCH-3 the layer state has one writer — put");
    assert_eq!(
        callers(&src, "put(row", "fn put("),
        vec![pair("sim/mod.rs", "adopt_state"), pair("sim/mod.rs", "judge"), pair("sim/mod.rs", "reset")],
        "S-sym-IMPL-SIM-SWITCH-3 put is reached only by adopting a target, the per-packet judgment and the row reset"
    );
    assert_eq!(callers(&src, "sim::reset(", "fn reset("), vec![pair("fwd/mod.rs", "apply_row_cmd")], "S-sym-IMPL-SIM-SWITCH-3 the reset comes only from the row command");
    assert_eq!(callers(&src, "sim::admit(", "fn admit("), vec![pair("fwd/mod.rs", "send_down")], "S-sym-IMPL-SIM-SWITCH-3 the per-packet judgment comes only from the send decision");
    assert_eq!(
        callers(&src, "adopt(", "fn adopt("),
        vec![pair("sim/mod.rs", "distribute"), pair("sim/mod.rs", "tick_v1")],
        "S-sym-IMPL-SIM-SWITCH-3 adopting a target comes only from the automatic judgment"
    );
    let control = callers(&src, "request(row", "fn request(");
    assert_eq!(control, vec![pair("sim/mod.rs", "subscribe_layer")], "stand check — the control's layer request is found");
    let req: Vec<&Src> = src.iter().filter(|l| l.file == "sim/mod.rs" && l.func == "request").collect();
    assert!(!req.is_empty() && req.iter().all(|l| !l.text.contains("put(") && !l.text.contains("reset(") && !l.text.contains("adopt")), "S-sym-IMPL-SIM-SWITCH-3 the control's request does not write the layer state");
}

#[test]
fn a_withdrawn_infra_request_is_logged_as_answered_and_never_counted_as_a_drop() {
    let src = product_lines();
    let tick: Vec<&Src> = src.iter().filter(|l| l.file == "rtcp/mod.rs" && l.func == "pli_tick").collect();
    assert!(tick.len() > 10, "stand check — the PLI decision step was read ({})", tick.len());
    let body = tick.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" ");
    assert!(body.contains("matches!(go, PliGo::Send | PliGo::Answered)") && body.contains("\"pli decided\"") && body.contains("\"in.go\" = go.name()"), "S-sym-IMPL-RTCP-TERMINATE-10 a withdrawn request writes the pli decided line with in.go");
    assert!(src.iter().any(|l| l.file == "rtcp/mod.rs" && l.text.contains("PliGo::Answered => \"answered\"")), "S-sym-IMPL-RTCP-TERMINATE-10 the in.go value of a withdrawn request is answered");
    let counted: Vec<&str> = tick.iter().filter(|l| l.text.contains("drop_count(")).map(|l| l.text.trim()).collect();
    assert!(!counted.is_empty() && counted.iter().all(|l| !l.contains("Answered")), "S-sym-IMPL-RTCP-TERMINATE-10 a withdrawn request is not a drop — only unbound and throttled requests count: {counted:?}");
    assert!(body.contains("PliGo::Idle | PliGo::Hold | PliGo::Answered => {}"), "S-sym-IMPL-RTCP-TERMINATE-10 answered sends nothing and counts nothing");
}
