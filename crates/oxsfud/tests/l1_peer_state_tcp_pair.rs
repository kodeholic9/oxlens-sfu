// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-PAIR-LIVENESS · model: claude-opus-5-5
//! 1층 소스 가드 · 전송 객체 — 경로가 UDP 쌍인 채 그 쌍이 만료되면 TCP 쌍이 살아 있어도 경로는 죽은 것이다(경로 쌍만 본다) · `Suspect` 에서 `Alive` 로 가는 길은 지명이 실린 검증된 검사 하나뿐이라 TCP 쌍의 생존만으로는 `Suspect` 가 유지된다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use oxsfud::link::{PathKey, PeerState, Role, Transport};

use support::*;

#[test]
fn a_live_tcp_pair_does_not_keep_a_udp_path_alive_nor_lift_suspect() {
    let _g = serial();
    let tr = Transport::new(sfu().next_id(), "pst-u", "ses-pst-u", Role::Subscribe, 0, false);
    let udp = PathKey::Udp("127.0.0.1:50000".parse().expect("addr"));
    let tcp = PathKey::Tcp(7);
    tr.touch_pair(udp, 1_000);
    tr.touch_pair(tcp, 1_000);
    tr.path.store(Some(Arc::new(udp)));
    assert!(tr.path_alive(1_000 + 29_000, |_| true), "stand check — the UDP path pair is alive inside 30 s");
    tr.touch_pair(tcp, 40_000);
    assert!(!tr.path_alive(40_000, |id| id == 7), "SV-STATE-PEER-STATE③ the path is the UDP pair — its expiry kills the path even while the TCP pair lives");
    assert_eq!(tr.set_state(PeerState::Suspect, 40_000), Some(PeerState::Alive), "stand check — the transport goes Suspect");
    tr.touch_pair(tcp, 41_000);
    assert_eq!(tr.peer_state(), PeerState::Suspect, "SV-STATE-PEER-STATE③ traffic on the TCP pair alone does not lift Suspect");
    tr.path.store(Some(Arc::new(tcp)));
    assert!(tr.path_alive(41_000, |id| id == 7), "stand check — once the path is the TCP pair, its connection keeps it alive (positive control)");
    let lines = product_lines();
    let alive: Vec<String> = lines.iter().filter(|l| l.text.contains("set_state(PeerState::Alive")).map(at).collect();
    assert_eq!(alive.len(), 1, "SV-STATE-PEER-STATE③ one place only takes a transport back to Alive: {alive:?}");
    let site = lines.iter().position(|l| l.text.contains("set_state(PeerState::Alive")).expect("site");
    let guard = lines[..site].iter().rev().take(4).any(|l| l.text.contains("if parsed.use_candidate"));
    assert!(alive[0].starts_with("fwd/rt.rs") && guard, "SV-STATE-PEER-STATE③ that place is the verified check carrying USE-CANDIDATE: {alive:?}");
}
