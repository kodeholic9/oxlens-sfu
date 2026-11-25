// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SIM-SWITCH · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-RTCP-TERMINATE · 정:RULE-SIM-TICK · model: claude-opus-5-5
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

#[test]
fn allocation_points_are_the_layer_rates_the_publisher_track_measured() {
    let src = product_lines();
    let body = |file: &str, func: &str| -> Vec<String> { src.iter().filter(|l| l.file == file && l.func == func).map(|l| l.text.trim().to_string()).collect() };
    let cands = body("sim/mod.rs", "candidates");
    assert!(cands.len() > 10, "stand check — the allocation candidates were read ({})", cands.len());
    let joined = cands.join(" ");
    assert!(joined.contains("sibling(t, l).map(|x| price_tl(&x, now_ms))") && joined.contains("strategy::layer_points(layer_strategy(), &raw,"), "S-sym-RULE-SIM-TICK-11 S-sym-RULE-SIM-TICK-16 a point's value is the bitrate measured on that layer's publisher track, not a nominal rate");
    assert!(body("sim/mod.rs", "price_tl").join(" ").contains("t.rate_tl.lock()"), "S-sym-RULE-SIM-TICK-11 S-sym-RULE-SIM-TICK-16 the measured value is the publisher track's own window");
    let writers = callers(&src, "rate_tl.lock()", "fn price_tl(");
    assert_eq!(writers, vec![pair("fwd/mod.rs", "forward"), pair("sim/mod.rs", "price_tl")], "S-sym-RULE-SIM-TICK-16 the window is filled on the publisher's ingress — no subscriber-side copy");
    let fwd = body("fwd/mod.rs", "forward");
    let add = fwd.iter().position(|l| l.contains("rate_tl.lock()"));
    let fan = fwd.iter().position(|l| l.starts_with("let mut targets"));
    assert!(matches!((add, fan), (Some(a), Some(t)) if a < t), "S-sym-RULE-SIM-TICK-16 the publisher's bytes are counted before any subscriber is chosen, so a late subscriber's first allocation already sees them: {add:?} {fan:?}");
}

#[test]
fn every_downlink_send_reports_to_the_probe_cluster_of_its_connection() {
    let src = product_lines();
    let body = |file: &str, func: &str| -> String { src.iter().filter(|l| l.file == file && l.func == func).map(|l| l.text.trim()).collect::<Vec<_>>().join(" ") };
    let on_sent = body("sim/mod.rs", "on_sent");
    assert!(on_sent.contains("tr.rtc.sim.lock()") && on_sent.contains("c.on_sent(bytes,") && on_sent.contains("if cluster != 0 { return cluster; }") && on_sent.contains("c.prober.media(now_us as i64, bytes)"), "S-sym-RULE-SIM-TICK-20 S-sym-RULE-SIM-TICK-10① every sent packet adds its bytes to the connection's estimate, and one without its own cluster takes the running probe cluster's tag");
    assert_eq!(callers(&src, "sim::on_sent(", "fn on_sent("), vec![pair("rtcp/mod.rs", "sent_twcc")], "S-sym-RULE-SIM-TICK-20 S-sym-RULE-SIM-TICK-10① the estimate hears sends only through sent_twcc");
    assert_eq!(
        callers(&src, "sent_twcc(", "fn sent_twcc("),
        vec![pair("fwd/mod.rs", "send_down"), pair("rtcp/mod.rs", "send_built")],
        "S-sym-RULE-SIM-TICK-20 S-sym-RULE-SIM-TICK-10① media (send_down) and every built packet (send_built) report their send"
    );
    assert_eq!(callers(&src, "send_built(", "fn send_built("), vec![pair("rtcp/mod.rs", "drain_rtx"), pair("sim/mod.rs", "pad")], "stand check — retransmission and padding are the built packets");
    assert!(body("rtcp/mod.rs", "drain_rtx").contains("padding: false, cluster: 0 };"), "S-sym-RULE-SIM-TICK-20 S-sym-RULE-SIM-TICK-10① a retransmission carries no cluster of its own, so it takes the running probe's tag like media");
    assert!(body("sim/mod.rs", "pad").contains("padding: true, cluster };"), "S-sym-RULE-SIM-TICK-10① padding carries the probe cluster it was made for");
}

#[test]
fn rr_loss_and_repeat_nack_reach_only_the_v1_judgment() {
    let src = product_lines();
    assert_eq!(callers(&src, "rr_loss.load(", "fn tick_v1("), vec![pair("sim/mod.rs", "tick_v1")], "S-sym-RULE-SIM-TICK-9 the subscriber's RR loss is read only by the v1 judgment");
    assert_eq!(callers(&src, "l.repeats)", "fn tick_v1("), vec![pair("sim/mod.rs", "tick_v1")], "S-sym-RULE-SIM-TICK-9 the repeat-NACK count is read only by the v1 judgment");
    assert_eq!(callers(&src, "policy.tick(", "fn tick("), vec![pair("sim/mod.rs", "tick_v1")], "S-sym-RULE-SIM-TICK-9 the v1 layer policy runs only in tick_v1");
    assert_eq!(callers(&src, "tick_v1(tr", "fn tick_v1("), vec![pair("sim/mod.rs", "drive")], "S-sym-RULE-SIM-TICK-9 tick_v1 is driven only from drive");
    let drive: Vec<String> = src.iter().filter(|l| l.file == "sim/mod.rs" && l.func == "drive").map(|l| l.text.trim().to_string()).collect();
    let at = |p: &str| -> Vec<usize> { drive.iter().enumerate().filter(|(_, l)| l.contains(p)).map(|(i, _)| i).collect() };
    let (v1, v2, tw, calls) = (at("Mode::V1 =>"), at("Mode::V2 => {}"), at("if twccless {"), at("tick_v1(tr, now_ms);"));
    assert!(v1.len() == 1 && v2.len() == 1 && tw.len() == 1 && calls.len() == 2, "stand check — drive has one v1 arm, one v2 arm, one twccless branch and two tick_v1 calls: {v1:?} {v2:?} {tw:?} {calls:?}");
    assert!(calls[0] > v1[0] && calls[0] < v2[0], "S-sym-RULE-SIM-TICK-9 the first tick_v1 sits in the v1 mode arm");
    assert!(calls[1] > tw[0] && drive[calls[1] + 1..].iter().take(3).any(|l| l == "return None;"), "S-sym-RULE-SIM-TICK-9 the second tick_v1 sits in the TWCC-less branch, which returns before the v2 allocation");
    let v2_path: Vec<&String> = drive[v2[0]..].iter().filter(|l| l.contains("rr_loss") || l.contains("repeats") || l.contains("policy.tick")).collect();
    assert!(v2_path.is_empty(), "S-sym-RULE-SIM-TICK-9 the v2 path of drive reads no RR loss and no repeat NACK — its layer is set by the target rate alone: {v2_path:?}");
}
