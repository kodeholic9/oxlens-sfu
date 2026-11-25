// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-PAIR-LIVENESS · 정:RULE-GC-FOUR · 정:RULE-LINK-TRANSPORT · model: claude-opus-5-5
//! 1층 상수 대조 · 소스 가드 — 시계가 둘이다(게이트웨이 세션 창 60s = Peer 수명 · UDP 쌍 생존 30s) · 미디어 침묵으로 Peer 를 거두는 경로가 없다(gc 호출 자리가 정해진 여덟 곳뿐 · 생존 검사는 Suspect 만 올린다) · SCTP 연관에 데이터 재전송 한도가 없다(제품이 sctp-proto 기본 설정을 그대로 쓰고 그 기본이 None)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::collections::BTreeSet;

use oxsfud::gc::Why;

use support::*;

fn why_name(w: Why) -> &'static str {
    match w {
        Why::Leave => "leave",
        Why::Kick => "kick",
        Why::Destroy => "destroy",
        Why::Evict => "evict",
        Why::SessionGone => "session_gone",
        Why::HomeLost => "home_lost",
    }
}

#[test]
fn the_session_window_is_sixty_seconds_and_the_udp_pair_lives_thirty() {
    let shipped = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml");
    let p = common::policy::Policy::load(&shipped).expect("stand check — the shipped policy.toml loads");
    assert_eq!(common::policy::Policy::default().hub.resume_window_ms, 60_000, "S-cond-RULE-DOC-COND-INDEX-SERVER-3 S-cond-IMPL-LINK-PEER-STATE-1 the policy default session window is 60s");
    assert_eq!(p.hub.resume_window_ms, 60_000, "S-cond-RULE-DOC-COND-INDEX-SERVER-3 S-cond-IMPL-LINK-PEER-STATE-1 the shipped policy session window is 60s");
    assert_eq!(oxsfud::link::UDP_PAIR_EXPIRE_MS, 30_000, "S-cond-RULE-DOC-COND-INDEX-SERVER-3 S-cond-IMPL-LINK-PEER-STATE-1 a UDP pair lives 30s without a verified check");
}

#[test]
fn no_path_reaps_a_peer_for_media_silence() {
    let names: BTreeSet<&str> = [Why::Leave, Why::Kick, Why::Destroy, Why::Evict, Why::SessionGone, Why::HomeLost].into_iter().map(|w| {
        assert_eq!(w.as_str(), why_name(w), "stand check — the reason name");
        w.as_str()
    }).collect();
    assert_eq!(names.len(), 6, "S-cond-RULE-DOC-COND-INDEX-SERVER-3 S-cond-IMPL-LINK-PEER-STATE-1 the reap reasons are six and none is media silence: {names:?}");
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let sites: BTreeSet<String> = lines
        .iter()
        .filter(|l| !l.file.starts_with("gc/") && l.text.contains("crate::gc::") && !l.text.trim_start().starts_with("use "))
        .map(|l| format!("{} {}", l.file, l.func))
        .collect();
    let want: BTreeSet<String> = [
        "ops/rooms.rs join",
        "ops/rooms.rs back_out",
        "ops/rooms.rs leave",
        "ops/rooms.rs session_gone",
        "ops/admin.rs reap",
        "ops/admin.rs destroy",
        "ops/timers.rs home_lost",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(sites, want, "S-cond-RULE-DOC-COND-INDEX-SERVER-3 S-cond-IMPL-LINK-PEER-STATE-1 a peer is reaped only by leave · kick · destroy · evict · session window · home lost — no media silence timer calls gc");
    let live: Vec<String> = lines.iter().filter(|l| l.file == "ops/timers.rs" && l.func == "liveness" && l.text.contains("gc::")).map(at).collect();
    assert!(live.is_empty(), "S-cond-RULE-DOC-COND-INDEX-SERVER-3 the pair liveness sweep only raises Suspect — it never reaps: {live:?}");
}

#[test]
fn the_sctp_association_has_no_data_retransmit_limit() {
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let builds: Vec<String> = lines.iter().filter(|l| l.text.contains("ServerConfig::new()") && l.file.starts_with("link/")).map(at).collect();
    assert_eq!(builds.len(), 1, "stand check — the SCTP endpoint is built once: {builds:?}");
    let tuned: Vec<String> = lines.iter().filter(|l| l.text.contains("max_data_retransmits") || l.text.contains("TransportConfig")).map(at).collect();
    assert!(tuned.is_empty(), "S-cond-RULE-DOC-COND-INDEX-SERVER-12① the server never tunes the SCTP transport — no data retransmit limit is set: {tuned:?}");
    let d = format!("{:?}", sctp_proto::ServerConfig::new().transport);
    assert!(d.contains("max_data_retransmits: None"), "S-cond-RULE-DOC-COND-INDEX-SERVER-12① the sctp-proto default the server uses has no data retransmit limit: {d}");
}
