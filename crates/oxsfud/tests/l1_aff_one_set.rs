// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-AFF-ONE-SET · 정:RULE-GC-LEAVE-SEVEN · model: claude-opus-5-5
//! 1층 제어 층 · 소스 가드 — 입장 방 집합은 하나다: `sub_rooms` 는 세션의 입장 방 집합 그대로이고 따로 든 청취 집합이 없다 · 그 집합과 방 명단(방 → 사람) · 발언권 명단은 입장 · 퇴장 길에서만 바뀐다 · 입장 · 교차 입장 · 같은 사람 새 세션 · 퇴장이 오간 뒤 셋이 늘 같다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::{Arc, Once};

use common::envelope::CallKind;
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::{self};
use oxsfud::room::Room;
use serde_json::json;

use support::*;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up (a new session needs transports)");
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn agree(rooms: &[Arc<Room>], users: &[String], when: &str) {
    let s = sfu();
    for r in rooms {
        for u in users {
            let entered = s.peer(u).is_some_and(|p| p.in_room(&r.id));
            let listed = r.has(u);
            let floor = r.floor.lock().map(|f| f.members.contains(u)).unwrap_or(false);
            assert!(
                entered == listed && listed == floor,
                "S-sym-RULE-AFF-ONE-SET-1 after {when} {u} in {}: entered {entered} · room roster {listed} · floor roster {floor} — one set, no window left open",
                r.id
            );
            if let Some(p) = s.peer(u) {
                assert_eq!(oxsfud::aff::affiliation(&p).sub_rooms, p.rooms(), "S-sym-RULE-AFF-ONE-SET-1 sub_rooms is the entered set itself");
            }
        }
    }
}

fn join(c: &Ctx, r: &Room) {
    let res = ops::rooms::join(c, &json!({ "room_id": r.id }));
    assert!(res.is_ok(), "stand check — {} joins {}: {res:?}", c.user, r.id);
}

fn leave(c: &Ctx, r: &Room) {
    let res = ops::rooms::leave(c, &json!({ "room_id": r.id }));
    assert!(res.is_ok(), "stand check — {} leaves {}: {res:?}", c.user, r.id);
}

#[test]
fn the_entered_set_the_room_roster_and_the_floor_roster_never_part() {
    let _g = serial();
    media();
    let (r1, r2) = (create(&tag("one-set-1")), create(&tag("one-set-2")));
    let rooms = [r1.clone(), r2.clone()];
    let subs = [subscriber(&tag("one-set-a")), subscriber(&tag("one-set-b")), subscriber(&tag("one-set-c"))];
    let users: Vec<String> = subs.iter().map(|x| x.p.user.clone()).collect();
    let cx: Vec<Ctx> = subs.iter().map(|x| ctx_of(&x.p)).collect();
    agree(&rooms, &users, "start");
    join(&cx[0], &r1);
    join(&cx[1], &r1);
    agree(&rooms, &users, "two joins");
    join(&cx[0], &r2);
    join(&cx[2], &r2);
    agree(&rooms, &users, "a cross join");
    let mut fresh = ctx_of(&subs[0].p);
    fresh.sid = format!("{}-new", cx[0].sid);
    fresh.session = format!("{}-new", cx[0].session);
    join(&fresh, &r1);
    agree(&rooms, &users, "the same person joining on a new session (the old one evicted, the roster kept)");
    assert!(sfu().peer(&users[0]).is_some_and(|p| p.sid == fresh.sid), "stand check — the new session took over");
    leave(&cx[1], &r1);
    agree(&rooms, &users, "a leave");
    leave(&fresh, &r1);
    leave(&cx[2], &r2);
    agree(&rooms, &users, "the last leaves");
    let lines = product_lines();
    let sites = |pred: &dyn Fn(&str) -> bool| -> Vec<String> { lines.iter().filter(|l| pred(&l.text)).map(|l| format!("{}::{}", l.file, l.func)).collect() };
    let mut entered = sites(&|t| t.contains(".rooms.lock()") && t.contains("Ok(mut "));
    entered.sort();
    entered.dedup();
    assert_eq!(
        entered,
        vec!["gc/mod.rs::leave".to_string(), "ops/rooms.rs::back_out".to_string(), "ops/rooms.rs::join".to_string()],
        "S-sym-RULE-AFF-ONE-SET-1 the entered set is written only on the join and leave paths"
    );
    let mut roster = sites(&|t| t.contains("room.join(") || t.contains("room.remove("));
    roster.sort();
    roster.dedup();
    assert_eq!(
        roster,
        vec!["gc/mod.rs::leave".to_string(), "ops/admin.rs::destroy".to_string(), "ops/rooms.rs::join".to_string()],
        "S-sym-RULE-AFF-ONE-SET-1 the room roster is written only on the join and leave paths (and the destroy sweep)"
    );
    let aff: Vec<&str> = lines.iter().filter(|l| l.file == "aff/mod.rs" && l.func == "affiliation").map(|l| l.text.as_str()).collect();
    assert!(aff.iter().any(|t| t.contains("sub_rooms: p.rooms()")), "S-sym-RULE-AFF-ONE-SET-1 sub_rooms is answered from the entered set — no separate listen set: {aff:?}");
}
