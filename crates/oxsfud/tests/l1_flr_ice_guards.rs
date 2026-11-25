// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-FLR-TLV · 연:RULE-DOC-COND-INDEX · 연:RULE-LINK-RECOVERY-UNIT · model: claude-opus-5-5
//! 1층 소스 가드 — 발언권 우선순위는 요청값 그대로 흐른다(대입 자리가 요청 `p` · 대기자 `w.p` · 그 둘을 옮긴 값뿐 · GRANTED 는 그 값을 u8 그대로 싣는다) · 서버는 ICE 검사를 먼저 내지 않는다(lite — STUN 요청을 짓는 함수를 제품이 부르지 않는다 · 자격 교체는 클라 `ICE_RESTART` 처리 하나)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::collections::BTreeSet;

use support::*;

fn rhs(text: &str, lhs: &str) -> Option<String> {
    let t = text.trim();
    t.strip_prefix(lhs).map(|r| r.trim().to_string())
}

#[test]
fn the_server_never_lowers_a_floor_priority() {
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let core: Vec<&Src> = lines.iter().filter(|l| l.file == "flr/core.rs").collect();
    assert!(core.len() > 300, "stand check — the floor core was read ({})", core.len());
    let sprio: BTreeSet<String> = core.iter().filter_map(|l| rhs(&l.text, "self.sprio =")).collect();
    let rprio: BTreeSet<String> = core.iter().filter_map(|l| rhs(&l.text, "self.rprio =")).collect();
    let want_s: BTreeSet<String> = ["p;", "w.p;"].into_iter().map(String::from).collect();
    let want_r: BTreeSet<String> = ["self.sprio;"].into_iter().map(String::from).collect();
    assert_eq!(sprio, want_s, "C-cond-RULE-FLR-TLV-1② the speaker priority is the request value or the waiter's value — never reduced");
    assert_eq!(rprio, want_r, "C-cond-RULE-FLR-TLV-1② the revoke priority copies the speaker priority");
    let waiters: Vec<String> = core.iter().filter(|l| l.text.contains("Waiter {") && !l.text.contains("struct Waiter")).filter(|l| !l.text.contains(", p, pre:")).map(|l| at(l)).collect();
    assert!(waiters.is_empty(), "C-cond-RULE-FLR-TLV-1② a waiter keeps the request priority as given: {waiters:?}");
    let granted: Vec<String> = core
        .iter()
        .filter(|l| l.text.contains("Mt::Granted, None,"))
        .filter(|l| !(l.text.contains("Mt::Granted, None, self.sprio,") || l.text.contains("Mt::Granted, None, ev.n,")))
        .map(|l| at(l))
        .collect();
    assert!(granted.is_empty(), "C-cond-RULE-FLR-TLV-1② GRANTED carries the speaker priority or the relayed value: {granted:?}");
    let ops: Vec<&Src> = lines.iter().filter(|l| l.file == "ops/floor.rs").collect();
    assert!(ops.iter().any(|l| l.text.contains("mbcp::REQUEST => Msg::new(Mt::Request, None, m.priority().unwrap_or(0) as u32, 0)")), "C-cond-RULE-FLR-TLV-1② the REQUEST priority TLV enters as is");
    assert!(ops.iter().any(|l| l.text.contains("Mt::Granted => mbcp::granted(m.n as u8,")), "C-cond-RULE-FLR-TLV-1② the GRANTED priority TLV leaves as is");
}

#[test]
fn the_server_never_starts_an_ice_check() {
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    assert!(lines.iter().any(|l| l.file == "link/stun.rs" && l.text.contains("pub fn request(")), "stand check — the STUN request builder exists");
    let sends: Vec<String> = lines
        .iter()
        .filter(|l| l.file != "link/stun.rs" && (l.text.contains("stun::request(") || l.text.contains("BINDING_REQUEST")))
        .map(at)
        .collect();
    assert!(sends.is_empty(), "C-cond-RULE-DOC-COND-INDEX-11② the server (ice-lite) never builds a binding request: {sends:?}");
    let restarts: BTreeSet<String> = lines.iter().filter(|l| l.text.contains(".restart(next")).map(|l| format!("{} {}", l.file, l.func)).collect();
    let want: BTreeSet<String> = ["ops/media.rs ice_restart".to_string()].into_iter().collect();
    assert_eq!(restarts, want, "C-cond-RULE-DOC-COND-INDEX-11② credentials change only in the client's ICE_RESTART handler");
    let creds: Vec<String> = lines.iter().filter(|l| l.file != "link/creds.rs" && l.text.contains(".restart(") && !l.text.contains(".restart(next")).filter(|l| l.text.contains("creds")).map(at).collect();
    assert!(creds.is_empty(), "C-cond-RULE-DOC-COND-INDEX-11② no other credential restart: {creds:?}");
}
