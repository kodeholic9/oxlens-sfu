// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:IMPL-FWD-ROOM · 정:IMPL-AFF-PUB-ROOM · model: claude-opus-5-5
//! 1층 제어 층 — `pub_select` 는 `pub_room` 값 하나를 바꾸고 끝난다: 바꾼 뒤 그 전송로의 다음 패킷이 고르는 슬롯은 새 방 것이고, 발행 스트림 · 트랙 · SSRC 는 그대로다(다시 발행하지 않는다)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use common::envelope::CallKind;
use oxsfud::ops;
use oxsfud::room::Room;
use oxsig::msg::{Duplex, Kind};
use serde_json::json;

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn shape(p: &oxsfud::link::Peer) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = sfu().pubs.get(&p.user).map(|c| c.streams()).unwrap_or_default().iter().map(|s| (s.track_id.clone(), s.ssrc)).collect();
    v.sort();
    v
}

#[test]
fn after_pub_select_the_next_packet_goes_to_the_new_room_without_republishing() {
    let _g = serial();
    let (r1, r2) = (create(&tag("ps8-a")), create(&tag("ps8-b")));
    let u = subscriber(&tag("ps8-u"));
    enter(&u.p, &r1);
    enter(&u.p, &r2);
    let _a = published(&u.p, &r1.id, Kind::Audio, Duplex::Half);
    let _v = published(&u.p, &r1.id, Kind::Video, Duplex::Half);
    oxsfud::aff::set_pub_room(&u.p, Some(&r1.id));
    let before = shape(&u.p);
    assert_eq!(before.len(), 2, "stand check — two half streams published");
    for k in [Kind::Audio, Kind::Video] {
        let (room, fan) = oxsfud::fwd::half_slot(&u.p.publish, k).expect("stand check — a packet finds the r1 slot");
        assert!(room.as_str() == r1.id && Arc::ptr_eq(&fan, r1.slot(k)), "stand check — {k:?} goes to r1 before the switch");
    }
    ops::media::move_pub(&u.p, Some(&r2), None).expect("S-cond-RULE-DOC-COND-INDEX-SERVER-8 pub_select into r2 is accepted");
    for k in [Kind::Audio, Kind::Video] {
        let (room, fan) = oxsfud::fwd::half_slot(&u.p.publish, k).expect("S-cond-RULE-DOC-COND-INDEX-SERVER-8 the next packet finds a slot");
        assert!(room.as_str() == r2.id && Arc::ptr_eq(&fan, r2.slot(k)), "S-cond-RULE-DOC-COND-INDEX-SERVER-8 the next {k:?} packet after pub_select goes to the new room r2");
    }
    assert_eq!(shape(&u.p), before, "S-cond-RULE-DOC-COND-INDEX-SERVER-8 no republish — the same streams (track id · SSRC) carry on into the new room");
}
