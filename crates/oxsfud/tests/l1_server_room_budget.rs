// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:IMPL-SUB-TABLE · 연:ROOM.ledger.BundleCap · model: claude-opus-5-5
//! 1층 제어 층 — 구독 예산의 순서: 슬롯 몫(정책 기본 40) 안에서 방 수 한도(20방)까지 슬롯 방에 들어가도 `4005` 가 안 나고 21방째는 `4004`(방 수 한도가 먼저) · 슬롯 몫을 줄인 정책에서만 그 몫이 찬 뒤 `4005`(개인 몫이 남아 있어도) · 개인 트랙은 나머지 몫으로 실린다 · `details` 는 개수(context 202611/20261124b 서버 규칙 갈래).

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

fn join(u: &Sub, r: &Room) -> Result<Value, oxsig::Failure> {
    ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": r.id }))
}

fn slot_rows(u: &Sub) -> usize {
    u.t.rows().len()
}

#[test]
fn twenty_slot_rooms_fit_the_default_share_and_the_twenty_first_is_listen_limit() {
    let _g = serial();
    assert_eq!(sfu().policy.media.slot_mid_reserve, 40, "stand check — the policy default slot share is 40");
    let u = subscriber(&tag("bud-u"));
    let rooms: Vec<Arc<Room>> = (0..21).map(|i| create(&tag(&format!("bud{i}")))).collect();
    for (i, r) in rooms.iter().take(20).enumerate() {
        let res = join(&u, r);
        assert!(res.is_ok(), "S-cond-RULE-DOC-COND-INDEX-SERVER-9 slot room {} of 20 inside the default share joins without 4005: {res:?}", i + 1);
    }
    assert_eq!(slot_rows(&u), 40, "stand check — twenty slot rooms load forty slot rows (audio · video each)");
    let f = join(&u, &rooms[20]).expect_err("S-cond-RULE-DOC-COND-INDEX-SERVER-9 the twenty-first room is refused");
    assert_eq!(f.code, 4004, "S-cond-RULE-DOC-COND-INDEX-SERVER-9 the room count limit comes first — 4004, not 4005: {f:?}");
    assert_eq!(f.details, Some(json!({ "limit": 20, "current": 20 })), "S-cond-RULE-DOC-COND-INDEX-SERVER-9 details are counts: {f:?}");
}

#[test]
fn a_smaller_slot_share_refuses_slot_rooms_with_mid_limit_while_individual_rows_still_load() {
    let _g = serial();
    let u = subscriber_slots(&tag("bud-s"), &tag("sid-bud-s"), 10);
    let rooms: Vec<Arc<Room>> = (0..6).map(|i| create(&tag(&format!("bds{i}")))).collect();
    for r in rooms.iter().take(5) {
        assert!(join(&u, r).is_ok(), "stand check — slot rooms join until the reduced share (10) is full");
    }
    assert_eq!(slot_rows(&u), 10, "stand check — five slot rooms fill the reduced share of ten");
    let f = join(&u, &rooms[5]).expect_err("S-cond-RULE-DOC-COND-INDEX-SERVER-9 a slot room past the reduced share is refused");
    assert_eq!(f.code, 4005, "S-cond-RULE-DOC-COND-INDEX-SERVER-9 the reduced slot share gives 4005 although the individual share (128 − 10) is left: {f:?}");
    assert_eq!(f.details, Some(json!({ "limit": 10, "current": 10 })), "S-cond-RULE-DOC-COND-INDEX-SERVER-9 details are counts of slot rows: {f:?}");
    assert!(!u.p.in_room(&rooms[5].id), "stand check — the refused room is backed out");
    let pubr = subscriber(&tag("bud-p"));
    enter(&pubr.p, &rooms[0]);
    let (v, _) = published(&pubr.p, &rooms[0].id, Kind::Video, Duplex::Full);
    list(&v, true);
    ops::media::resync_peer(&u.p, false);
    assert!(holds(&u.t, &v.track_id, &rooms[0].id), "S-cond-RULE-DOC-COND-INDEX-SERVER-9 an individual track still loads from the remaining share after the slot share is full");
    assert_eq!(slot_rows(&u), 11, "S-cond-RULE-DOC-COND-INDEX-SERVER-9 the individual row is added beside the ten slot rows");
}
