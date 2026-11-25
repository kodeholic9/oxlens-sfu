// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:AFF.op.AFFILIATION · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-FLR-MEMBER-JOIN · model: claude-opus-5-5
//! 1층 제어 층 — 발행 방 고르기의 서버 몫: 반이중만 든 사람이 발행 방을 옮기면 걸린 두 방의 `seq` 가 안 오른다(트랙 층이 안 바뀐다) · `pub_deselect` 와 옛 방을 내리는 `pub_select` 는 그 방 발언권 큐에서 그 사람을 뺀다 · 쥔 화자는 그대로다(context 202611/20261124b 서버 규칙 갈래 · 클라 연동 갈래에서 넘겨받음).

mod support;

use std::sync::Arc;

use common::envelope::CallKind;
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops::floor::{self, Entry};
use oxsfud::ops::{self};
use oxsfud::room::Room;
use oxsig::msg::{Duplex, Kind};
use serde_json::json;

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn queued(r: &Room, u: &str) -> bool {
    r.floor.lock().map(|f| f.queue.iter().any(|w| w.u == u)).unwrap_or(false)
}

fn holder(r: &Room) -> Option<String> {
    r.floor.lock().ok().and_then(|f| f.holder())
}

#[test]
fn moving_a_half_duplex_publisher_moves_no_room_seq() {
    let _g = serial();
    let (r1, r2) = (create(&tag("afs-1")), create(&tag("afs-2")));
    let u = subscriber(&tag("afs-u"));
    enter(&u.p, &r1);
    enter(&u.p, &r2);
    let _a = published(&u.p, &r1.id, Kind::Audio, Duplex::Half);
    let _v = published(&u.p, &r1.id, Kind::Video, Duplex::Half);
    oxsfud::aff::set_pub_room(&u.p, Some(&r1.id));
    let before = (r1.version().seq, r2.version().seq);
    ops::media::move_pub(&u.p, Some(&r2), None).expect("stand check — pub_select into r2");
    assert_eq!(oxsfud::aff::pub_room(&u.p).as_deref(), Some(r2.id.as_str()), "stand check — the publish room moved");
    assert_eq!((r1.version().seq, r2.version().seq), before, "CP-OP-AFFILIATION⑤ a half-duplex move bumps neither room's seq — nobody sees a gap or resyncs");
}

#[test]
fn leaving_the_publish_room_takes_the_member_out_of_its_floor_queue() {
    let _g = serial();
    let (r1, r2) = (create(&tag("afq-1")), create(&tag("afq-2")));
    let (a, b) = (subscriber(&tag("afq-a")), subscriber(&tag("afq-b")));
    for p in [&a.p, &b.p] {
        enter(p, &r1);
    }
    enter(&b.p, &r2);
    let (ua, ub) = (a.p.user.clone(), b.p.user.clone());
    let queue_b = |r: &Arc<Room>| {
        oxsfud::aff::set_pub_room(&b.p, Some(&r.id));
        floor::run(r, Entry::of("ws_pub_select", &ub), |f, _| Some(f.ws_pub_select(&ub)));
        floor::run(r, Entry::of("server_rx", &ub), |f, t2| Some(f.server_rx(&ub, &Msg::new(Mt::Request, None, 1, 0), true, t2)));
    };
    for u in [&ua, &ub] {
        floor::run(&r1, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    }
    oxsfud::aff::set_pub_room(&a.p, Some(&r1.id));
    floor::run(&r1, Entry::of("ws_pub_select", &ua), |f, _| Some(f.ws_pub_select(&ua)));
    floor::run(&r1, Entry::of("server_rx", &ua), |f, t2| Some(f.server_rx(&ua, &Msg::new(Mt::Request, None, 1, 0), true, t2)));
    assert_eq!(holder(&r1).as_deref(), Some(ua.as_str()), "stand check — a holds the floor in r1");
    queue_b(&r1);
    assert!(queued(&r1, &ub), "stand check — b waits in r1's queue");
    let res = ops::media::affiliation(&ctx_of(&b.p), &json!({ "pub_deselect": r1.id, "pub_deselect_join_id": format!("j-{}", r1.id) }));
    assert!(res.is_ok(), "stand check — pub_deselect is answered: {res:?}");
    assert!(!queued(&r1, &ub), "CP-OP-AFFILIATION⑧ pub_deselect takes b out of r1's floor queue");
    assert_eq!(holder(&r1).as_deref(), Some(ua.as_str()), "CP-OP-AFFILIATION⑧ the speaker keeps the floor");
    queue_b(&r1);
    assert!(queued(&r1, &ub), "stand check — b waits in r1's queue again");
    let res = ops::media::affiliation(&ctx_of(&b.p), &json!({ "pub_select": r2.id, "pub_select_join_id": format!("j-{}", r2.id) }));
    assert!(res.is_ok(), "stand check — pub_select to r2 is answered: {res:?}");
    assert!(!queued(&r1, &ub), "CP-OP-AFFILIATION⑧ a pub_select that takes r1 down also takes b out of r1's floor queue");
    assert_eq!(holder(&r1).as_deref(), Some(ua.as_str()), "CP-OP-AFFILIATION⑧ the speaker keeps the floor");
}

#[test]
fn a_switch_deselects_the_old_room_first_and_releases_its_floor() {
    let _g = serial();
    let (r0, r1) = (create(&tag("afw-0")), create(&tag("afw-1")));
    let (a, o) = (subscriber(&tag("afw-a")), subscriber(&tag("afw-o")));
    for p in [&a.p, &o.p] {
        enter(p, &r0);
    }
    enter(&a.p, &r1);
    let (ua, uo) = (a.p.user.clone(), o.p.user.clone());
    for u in [&ua, &uo] {
        floor::run(&r0, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    }
    oxsfud::aff::set_pub_room(&a.p, Some(&r0.id));
    floor::run(&r0, Entry::of("ws_pub_select", &ua), |f, _| Some(f.ws_pub_select(&ua)));
    floor::run(&r0, Entry::of("server_rx", &ua), |f, t2| Some(f.server_rx(&ua, &Msg::new(Mt::Request, None, 1, 0), true, t2)));
    assert_eq!(holder(&r0).as_deref(), Some(ua.as_str()), "stand check — a holds the floor in r0");
    let body = json!({ "pub_deselect": r0.id, "pub_deselect_join_id": format!("j-{}", r0.id), "pub_select": r1.id, "pub_select_join_id": format!("j-{}", r1.id) });
    let res = ops::media::affiliation(&ctx_of(&a.p), &body);
    assert!(res.is_ok(), "stand check — the switch is answered: {res:?}");
    assert_eq!(holder(&r0), None, "CP-OP-AFFILIATION② deselect applies before select — the floor a held in r0 is released by the switch");
    assert_eq!(oxsfud::aff::pub_room(&a.p).as_deref(), Some(r1.id.as_str()), "CP-OP-AFFILIATION② then select applies — the publish room is r1");
}
