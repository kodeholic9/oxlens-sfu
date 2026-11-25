// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-VERSION-ISSUE · 정:RULE-DOC-COND-INDEX-SERVER · model: claude-opus-5-5
//! 1층 제어 층 — 방 하나는 미디어 서버 하나다: 같은 방에 든 두 세션의 입장 응답 `server_config.sfu_id` 가 같고 그 방의 판번호 `epoch` 와 같다 · 방이 사는 동안(입장 · 발행 · 퇴장이 오가도) 그 값이 안 바뀐다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use common::envelope::CallKind;
use oxsfud::ops;
use oxsfud::room::Room;
use oxsig::msg::{Duplex, Kind};
use serde_json::{Value, json};

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn sfu_id(v: &Value) -> String {
    v["server_config"]["sfu_id"].as_str().map(String::from).expect("stand check — the join answer carries server_config.sfu_id")
}

#[test]
fn every_session_in_one_room_sees_one_server_for_the_whole_room_life() {
    let _g = serial();
    let r = create(&tag("one"));
    let (a, b) = (subscriber(&tag("one-a")), subscriber(&tag("one-b")));
    let ja = ops::rooms::join(&ctx_of(&a.p), &json!({ "room_id": r.id })).expect("stand check — a joins");
    let jb = ops::rooms::join(&ctx_of(&b.p), &json!({ "room_id": r.id })).expect("stand check — b joins");
    assert_eq!(sfu_id(&ja), sfu_id(&jb), "S-cond-IMPL-ROOM-VERSION-ISSUE-1 two sessions in the same room see the same sfu_id");
    assert_eq!(sfu_id(&ja), r.version().epoch, "S-cond-IMPL-ROOM-VERSION-ISSUE-1 the sfu_id is the room's version epoch — the room belongs to this server");
    let first = r.version();
    let (v, _) = published(&a.p, &r.id, Kind::Video, Duplex::Full);
    list(&v, true);
    ops::media::announce_add(&v);
    let c = subscriber(&tag("one-c"));
    let jc = ops::rooms::join(&ctx_of(&c.p), &json!({ "room_id": r.id })).expect("stand check — c joins later");
    assert!(oxsfud::gc::leave(&b.p, &r.id, oxsfud::gc::Why::Leave, false), "stand check — b leaves");
    let later = r.version();
    assert!(later.seq > first.seq, "stand check — the room's life moved its version ({} → {})", first.seq, later.seq);
    assert_eq!((later.epoch.clone(), sfu_id(&jc)), (first.epoch.clone(), first.epoch.clone()), "S-cond-IMPL-ROOM-VERSION-ISSUE-1 the sfu_id does not change during the room's life");
}
