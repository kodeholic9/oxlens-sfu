// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-DOC-COND-INDEX · 정:IMPL-LINK-PEER-STATE · 정:RULE-GC-LEAVE-SEVEN · model: claude-opus-5-5
//! 1층 소스 가드 — Peer 를 없애는 길은 방 퇴장(마지막 방이면 회수 — `ROOM_LEAVE` · 관리자 축출 · 방 폭파)과 세션 쪽 사건(같은 사람 새 입장의 축출 · 세션 폐기 · 홈 잃음)뿐이다: 회수 · 축출 · 퇴장을 부르는 자리는 그 함수들 안에만 있고 · 쌍별 생존 판정(미디어 침묵)과 패킷 길에는 하나도 없다(클라 연동 갈래에서 넘겨받은 연동규격서 색인 행 · context 202611/20261124b).

mod support;

use std::collections::BTreeSet;

use support::*;

#[test]
fn only_room_leave_and_session_events_end_a_peer_never_media_silence() {
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let calls = |l: &&Src| l.text.contains("gc::reap(") || l.text.contains("gc::evict(") || l.text.contains("gc::leave(") || (l.file == "gc/mod.rs" && (l.text.trim_start().starts_with("reap(") || l.text.trim_start().starts_with("leave(")));
    let sites: BTreeSet<String> = lines.iter().filter(calls).map(|l| format!("{}::{}", l.file, l.func)).collect();
    let allowed: BTreeSet<String> = [
        "gc/mod.rs::leave",
        "gc/mod.rs::evict",
        "ops/admin.rs::reap",
        "ops/admin.rs::destroy",
        "ops/rooms.rs::join",
        "ops/rooms.rs::back_out",
        "ops/rooms.rs::leave",
        "ops/rooms.rs::session_gone",
        "ops/timers.rs::home_lost",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert!(sites.is_subset(&allowed) && !sites.is_empty(), "C-cond-RULE-DOC-COND-INDEX-22③ a Peer ends only through room leave or session events — reap/evict/leave callers: {sites:?}");
    let silent: Vec<String> = lines.iter().filter(calls).filter(|l| l.func == "liveness" || l.file.starts_with("fwd/") || l.file.starts_with("link/")).map(at).collect();
    assert!(silent.is_empty(), "C-cond-RULE-DOC-COND-INDEX-22③ the pair-liveness check (media silence) and the packet path never end a Peer: {silent:?}");
    let live: Vec<&Src> = lines.iter().filter(|l| l.file == "ops/timers.rs" && l.func == "liveness").collect();
    assert!(live.iter().any(|l| l.text.contains("PeerState::Suspect")), "stand check — liveness is the pair-expiry judge");
}
