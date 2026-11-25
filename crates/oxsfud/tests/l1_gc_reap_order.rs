// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-GC-PEER-REAP · model: claude-opus-5-5
//! 1층 소스 가드 · 제어 층 — Peer 를 거둘 때 참조부터 끊고 본체를 내린다(Peer 집합에서 빼기 → 발행 스트림 → 배정표 → 전송 등록 해제) · Peer · 배정표를 집합에 넣는 곳은 입장의 `new_peer` 하나라 패킷 길이 거두는 중인 Peer 를 되살릴 길이 없다 · 거둔 뒤 Peer · 표 · 전송이 남지 않는다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::atomic::Ordering;

use oxsfud::gc::Why;
use oxsig::msg::{Duplex, Kind};

use support::*;

#[test]
fn a_reap_cuts_the_references_before_the_body_and_nothing_else_puts_a_peer_back() {
    let lines = product_lines();
    assert!(lines.len() > 1_000, "stand check — the product sources were read ({})", lines.len());
    let reap: Vec<&Src> = lines.iter().filter(|l| l.file == "gc/mod.rs" && l.func == "reap").collect();
    let at = |needle: &str| reap.iter().position(|l| l.text.contains(needle)).unwrap_or_else(|| panic!("stand check — reap has {needle}"));
    let order = [at("g.remove(&peer.user)"), at("drop_stream(peer, &st)"), at("retire_table(peer"), at("m.unregister(&tr)")];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "S-sym-RULE-GC-LEAVE-SEVEN-1 reap cuts the Peer reference first, then streams · table · transport registration: {order:?}");
    let puts: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.text.contains("s.tables.insert(") || l.text.contains("peers.lock()"))
        .filter(|(i, l)| l.text.contains("s.tables.insert(") || lines.get(i + 1).is_some_and(|n| n.text.contains(".insert(")))
        .map(|(_, l)| format!("{} {}", l.file, l.func))
        .collect();
    assert_eq!(puts, vec!["ops/rooms.rs new_peer".to_string(), "ops/rooms.rs new_peer".to_string()], "S-sym-RULE-GC-LEAVE-SEVEN-1 only the join's new_peer puts a Peer or a table — no packet path can bring a reaped Peer back");
}

#[test]
fn after_a_reap_no_peer_table_or_open_transport_is_left() {
    let _g = serial();
    let r = room(&tag("rp1"));
    let u = subscriber(&tag("rp1-u"));
    enter(&u.p, &r);
    let (st, _) = published(&u.p, &r.id, Kind::Audio, Duplex::Full);
    list(&st, true);
    assert!(sfu().peer(&u.p.user).is_some() && sfu().table(&u.p).is_some(), "stand check — the Peer and its table are registered");
    oxsfud::gc::reap(&u.p, Why::SessionGone);
    assert!(sfu().peer(&u.p.user).is_none(), "S-sym-RULE-GC-LEAVE-SEVEN-1 no ghost Peer is left after the reap");
    assert!(sfu().table(&u.p).is_none(), "S-sym-RULE-GC-LEAVE-SEVEN-1 the Peer's table is gone with it");
    assert!(u.p.transports().iter().all(|t| t.closed.load(Ordering::Acquire)), "S-sym-RULE-GC-LEAVE-SEVEN-1 every transport of the reaped Peer is closed");
    assert!(sfu().pubs.get(&u.p.user).is_none(), "S-sym-RULE-GC-LEAVE-SEVEN-1 the Peer's publish context is gone");
}
