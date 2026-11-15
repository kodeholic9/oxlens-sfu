// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-TRN-IDEMPOTENT · 연:RULE-ROOM-LEAVE-FAIL · 정:IMPL-GC-PEER-REAP · model: claude-fable-5-2
//! 1층 제어 층 — 옛 세션의 늦은 요청(context 202611/20261115e). 같은 사람이 새 세션으로 다시 붙은 뒤에 닿은 옛 세션의 `ROOM_LEAVE` · `MESSAGE` 는 「세션 + 방」으로 가려 `3002` 이고 새 세션의 Peer 는 그 방에 그대로다. 겹침 없이 요청 하나로 선다.

mod support;

use oxsfud::ops;
use oxsfud::ops::handle::Ctx;
use oxsfud::link::Peer;
use oxsig::Failure;
use serde_json::{Value, json};

use support::*;

fn old_of(p: &Peer) -> Ctx {
    let mut c = ctx_of(p);
    c.sid = format!("old-{}", p.sid);
    c
}

fn code(r: &Result<Value, Failure>) -> Option<u16> {
    r.as_ref().err().map(|f| f.code)
}

#[test]
fn old_session_leave_without_join_id_keeps_the_new_session_in_the_room() {
    let _g = serial();
    let r = room(&tag("old-leave"));
    let u = subscriber(&tag("u"));
    enter(&u.p, &r);
    let got = ops::rooms::leave(&old_of(&u.p), &json!({ "room_id": r.id }));
    assert_eq!(code(&got), Some(3002), "an old session's leave is answered as not in the room");
    assert!(u.p.in_room(&r.id), "the new session's peer keeps the room");
    assert!(r.member(&u.p.user).is_some(), "the roster keeps the new session's member");
}

#[test]
fn old_session_leave_with_the_join_id_keeps_the_new_session_in_the_room() {
    let _g = serial();
    let r = room(&tag("old-leave-jid"));
    let u = subscriber(&tag("u"));
    enter(&u.p, &r);
    let jid = u.p.join_id(&r.id).expect("stand check — entered with a join id");
    let got = ops::rooms::leave(&old_of(&u.p), &json!({ "room_id": r.id, "join_id": jid }));
    assert_eq!(code(&got), Some(3002), "an old session's leave is answered as not in the room even with the join id");
    assert!(u.p.in_room(&r.id), "the new session's peer keeps the room");
}

#[test]
fn the_same_session_still_leaves() {
    let _g = serial();
    let r = room(&tag("same-leave"));
    let u = subscriber(&tag("u"));
    enter(&u.p, &r);
    let got = ops::rooms::leave(&ctx_of(&u.p), &json!({ "room_id": r.id }));
    assert!(got.is_ok(), "the session of the peer leaves: {:?}", got.as_ref().err().map(|f| f.code));
    assert!(!u.p.in_room(&r.id), "the peer left the room");
    assert!(r.member(&u.p.user).is_none(), "the roster dropped the member");
}

#[test]
fn old_session_message_is_refused() {
    let _g = serial();
    let r = room(&tag("old-msg"));
    let u = subscriber(&tag("u"));
    enter(&u.p, &r);
    let got = ops::media::message(&old_of(&u.p), &json!({ "room_id": r.id, "content": "late" }));
    assert_eq!(code(&got), Some(3002), "an old session's message is answered as not in the room");
}

#[test]
fn old_session_remove_of_a_roomless_stream_is_refused() {
    let _g = serial();
    let r = room(&tag("old-remove"));
    let u = subscriber(&tag("u"));
    let (st, _t) = published(&u.p, "", oxsig::msg::Kind::Audio, oxsig::msg::Duplex::Full);
    let body = json!({ "action": "remove", "room_id": r.id, "join_id": "j-none", "track_ids": [st.track_id] });
    let got = ops::media::publish(&old_of(&u.p), &body);
    assert_eq!(code(&got), Some(3002), "an old session cannot take the roomless stream of the new session down");
    assert!(sfu().pubs.get(&u.p.user).is_some_and(|c| c.find(&st.track_id).is_some()), "the stream stays");
}

#[test]
fn old_session_affiliation_is_refused() {
    let _g = serial();
    let r = room(&tag("old-aff"));
    let u = subscriber(&tag("u"));
    enter(&u.p, &r);
    let jid = u.p.join_id(&r.id).expect("stand check — entered with a join id");
    let before = oxsfud::aff::pub_room(&u.p);
    let got = ops::media::affiliation(&old_of(&u.p), &json!({ "pub_select": r.id, "pub_select_join_id": jid }));
    assert_eq!(code(&got), Some(3002), "an old session's affiliation is answered as not in the room");
    assert_eq!(oxsfud::aff::pub_room(&u.p), before, "the new session's publish room is untouched");
}

#[test]
fn audit_the_requester_peer_is_found_through_one_function() {
    let mut by_name: Vec<(String, String)> =
        product_lines().iter().filter(|l| l.text.contains("peer(&c.user)")).map(|l| (l.file.clone(), l.func.clone())).collect();
    by_name.sort();
    by_name.dedup();
    assert_eq!(
        by_name,
        vec![("ops/media.rs".to_string(), "own".to_string()), ("ops/rooms.rs".to_string(), "join".to_string())],
        "연:RULE-TRN-IDEMPOTENT the requester's peer is found by name only in own (session compared) and in the join that tells the sessions apart"
    );
}
