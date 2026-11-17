// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-CHAT-FANOUT · model: claude-opus-5-5
//! 1층 — 배치 후보 · 키 공간 · 고립 판정 · 가라앉기 · `sess` 짝 집합 표 · 뿌리기 범위(공개 API 만).

use common::keyspace::Keys;
use oxhubd::bus::placement;
use oxhubd::bus::view::{TokenEv, View};
use oxsig::Code;

#[test]
fn self_node_is_candidate_before_its_own_echo() {
    let v = View::new("node-a", "h1", 30_000);
    let c = v.candidates(0);
    assert!(c.contains(&"node-a".to_string()), "S-sym-RULE-BUS-PLACEMENT-6 — 자기 node 는 메아리 전에도 후보 {c:?}");
    let picked = placement::pick(c.iter().map(|s| s.as_str()), "r1");
    assert_eq!(picked.as_deref(), Some("node-a"), "S-sym-RULE-BUS-PLACEMENT-6 — 한 대 형상에서 후보가 비지 않는다");
    let mut two = View::new("node-a", "h1", 30_000);
    two.on_token(TokenEv::NodePut { node: "node-b".into(), inst: "h2".into() }, 0);
    let mut other = View::new("node-b", "h2", 30_000);
    other.on_token(TokenEv::NodePut { node: "node-a".into(), inst: "h1".into() }, 0);
    for i in 0..50 {
        let r = format!("room-{i}");
        let a = placement::pick(two.candidates(0).iter().map(|s| s.as_str()), &r);
        let b = placement::pick(other.candidates(0).iter().map(|s| s.as_str()), &r);
        assert_eq!(a, b, "S-sym-RULE-BUS-PLACEMENT-6 — 두 hub 의 배치가 갈리지 않는다 ({r})");
    }
}

#[test]
fn room_id_before_node_one_wildcard_each_question() {
    let k = Keys::new("ns");
    let where_ = k.room_where("r9");
    let of_node = k.rooms_of_node("n1");
    assert_eq!(where_.matches('*').count(), 2, "S-sym-RULE-BUS-KEYSPACE-2 — 「이 방 어디 있나」는 ** 하나 {where_}");
    assert!(where_.starts_with("ox/ns/int/room/r9/"), "S-sym-RULE-BUS-KEYSPACE-2 — 방 id 가 node 앞 {where_}");
    assert_eq!(of_node, "ox/ns/int/room/*/n1/**", "S-sym-RULE-BUS-KEYSPACE-2 — 「그 node 의 방」도 * 한 칸");
    let tok = k.room_token("r9", "n1", "e1");
    assert!(tok.find("r9").unwrap_or(usize::MAX) < tok.find("n1").unwrap_or(0), "S-sym-RULE-BUS-KEYSPACE-2 — 토큰 키 차례 {tok}");
}

#[test]
fn delete_batch_is_judged_after_settle() {
    let mut v = View::new("a", "ha", 30_000);
    v.on_token(TokenEv::NodePut { node: "b".into(), inst: "hb".into() }, 0);
    v.on_token(TokenEv::RoomPut { room: "rb".into(), node: "b".into(), inst: "eb".into() }, 0);
    v.on_token(TokenEv::RoomDel { room: "rb".into(), node: "b".into(), inst: "eb".into() }, 100);
    v.on_token(TokenEv::NodeDel { node: "b".into(), inst: "hb".into() }, 150);
    assert!(v.settle(900).dropped_rooms.is_empty(), "S-sym-RULE-BUS-EVENTS-7 — 가라앉기 전에는 판정하지 않는다");
    let s = v.settle(1_200);
    assert!(s.isolated_now && s.dropped_rooms.is_empty(), "S-sym-RULE-BUS-EVENTS-7 — 본 적 있는 남의 토큰 일괄 소멸은 고립 · 방을 쥔다");
}

fn sess(put: bool, user: &str, node: &str, sid: &str) -> TokenEv {
    let (user, node, sid) = (user.to_string(), node.to_string(), sid.to_string());
    if put { TokenEv::SessPut { user, node, sid } } else { TokenEv::SessDel { user, node, sid } }
}

fn following(node: &str) -> View {
    let mut v = View::new(node, "hc", 30_000);
    v.follow_sessions();
    v.sessions_followed();
    v
}

#[test]
fn sess_table_removes_the_pair_not_the_session() {
    let u = vec!["u".to_string()];
    let mut v = following("node-c");
    v.on_token(sess(true, "u", "node-a", "s1"), 0);
    v.on_token(sess(true, "u", "node-b", "s1"), 10);
    v.on_token(sess(false, "u", "node-a", "s1"), 20);
    assert_eq!(v.fanout_scope(Some(Ok(u.clone()))), Some(vec!["node-b".to_string()]), "S-sym-IMPL-CHAT-FANOUT-1① — 새 node 의 Put 이 옛 node 의 Delete 보다 먼저 와도 표는 U → {{B}}");
    let mut w = following("node-c");
    w.on_token(sess(true, "u", "node-a", "s1"), 0);
    w.on_token(sess(false, "u", "node-a", "s1"), 10);
    w.on_token(sess(true, "u", "node-b", "s1"), 20);
    assert_eq!(w.fanout_scope(Some(Ok(u.clone()))), v.fanout_scope(Some(Ok(u))), "S-sym-IMPL-CHAT-FANOUT-1① — 짝 집합이면 두 사건이 어느 차례로 와도 끝 상태가 같다(정:RULE-BUS-PRESENCE)");
    assert!(v.last_event.is_none() && !v.settle_due(5_000), "정:RULE-BUS-EVENTS — sess 토큰 사건은 방 · node 판정의 가라앉기를 밀지 않는다");
}

#[test]
fn fanout_scope_is_every_node_when_the_roster_fails() {
    let mut v = following("node-a");
    v.on_token(sess(true, "u", "node-b", "s1"), 0);
    assert_eq!(v.fanout_scope(Some(Err(Code::InternalError))), None, "S-sym-IMPL-CHAT-FANOUT-2 — 명단을 못 읽으면 좁히지 않는다(아는 node 전부)");
    assert_eq!(v.fanout_scope(Some(Ok(vec!["nobody".into()]))), Some(Vec::new()), "정:IMPL-CHAT-FANOUT — 명단을 읽었고 멤버 세션이 없으면 다른 node 에 보내지 않는다");
    assert_eq!(v.fanout_scope(Some(Ok(vec!["u".into()]))), Some(vec!["node-b".to_string()]), "정:IMPL-CHAT-FANOUT — 멤버가 세션을 둔 node 만");
    let mut early = View::new("node-a", "ha", 30_000);
    early.follow_sessions();
    early.on_token(sess(true, "u", "node-b", "s1"), 0);
    assert_eq!(early.fanout_scope(Some(Ok(vec!["u".into()]))), None, "정:IMPL-CHAT-FANOUT — 표의 history 를 다 받기 전에는 아는 node 전부");
    let off = View::new("node-a", "ha", 30_000);
    assert_eq!(off.fanout_scope(None), None, "정:IMPL-CHAT-FANOUT — follow_sessions 를 끄면 아는 node 전부");
}
