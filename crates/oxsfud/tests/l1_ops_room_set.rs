// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-LIFE · 정:IMPL-FLR-LOCK · 정:IMPL-FWD-ROOM · 정:RULE-CONC-SIX · 정:RULE-CONC-HOT-PATH · 정:RULE-BUS-PRESENCE · 정:IMPL-OPS-DESTROY-REAP · model: claude-opus-5-5
//! 1층 제어 층 — 전수 검사 셋째 묶음(context 202611/20261115b). 창 8: 방을 넣고 빼는 그 걸음이 패킷 길의 방 → 슬롯 사본까지 고친다. 창 6-7: 같은 id 로 다시 만든 방은 새 방이다 — 옛 방을 쥔 늦은 지우기는 새 방을 지우지 않고 · 같은 id 만들기가 겹쳐도 집행자는 하나 · 집행자를 띄우지 않았거나 멈춘 방에서는 판정이 돌지 않는다. 창 6-10: 방을 지우면 그 방의 `room` 토큰이 철회되고, 지운 뒤 늦게 든 토큰은 그 방의 마지막 참조와 함께 걷힌다. 받침 런타임은 스레드 하나라 집행자는 시험이 양보할 때만 돈다 — 잠 길이에 기대지 않고 양보 횟수 상한으로 기다린다. 겹침 시험 둘(T8-2 · T67-2)은 스레드를 `Barrier` 로 함께 세우는 통계 시험이다. T8-2 는 색인 쓰기를 잠금 밖으로 되돌리면 잡는다(기준 방이 색인 짓기를 길게 해 창을 넓힌다). T67-2 는 같은 id 겹침의 재현이 아니라 대조군이다 — 고치기 전 빌드에서도 겹침을 세우지 못했고(겹쳐야 하는 구간이 첫째의 넣기 앞 µs), 집행자 하나를 결정적으로 지키는 것은 T67-3(시작의 CAS)이다.

mod support;

use std::sync::{Arc, Barrier};

use common::envelope::{CallKind, Envelope};
use oxsfud::flr::core::{Floor, Msg, Mt};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::room::{PERMANENT, Room, RoomInit};
use oxsig::msg::{Kind, MediaCodecs, Permission};
use serde_json::json;

use support::*;

const YIELDS: usize = 10_000;

fn create_env(id: &str) -> Envelope {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    e
}

fn create(id: &str) -> Arc<Room> {
    let res = drive(ops::rooms::create(&create_env(id)));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn destroy(r: &Arc<Room>) {
    drive(ops::rooms::destroy_room(r, "SV-IMPL-ROOM-LIFE", "l1"));
}

fn fresh(id: &str) -> Arc<Room> {
    Room::new(RoomInit {
        id: id.into(),
        name: id.into(),
        capacity: 1_000,
        unused_ttl: PERMANENT,
        departure_ttl: PERMANENT,
        media_codecs: MediaCodecs::default(),
        epoch: sfu().epoch.clone(),
        floor: Floor::new(10, 30),
        record: Vec::new(),
    })
}

fn settle(mut f: impl FnMut() -> bool) -> bool {
    for _ in 0..YIELDS {
        if f() {
            return true;
        }
        drive(tokio::task::yield_now());
    }
    f()
}

fn turns(n: usize) {
    for _ in 0..n {
        drive(tokio::task::yield_now());
    }
}

fn slot_is(id: &str, r: &Room) -> bool {
    [Kind::Audio, Kind::Video].into_iter().all(|k| ops::slot_of(id, k).is_some_and(|f| Arc::ptr_eq(&f, r.slot(k))))
}

fn no_slot(id: &str) -> bool {
    [Kind::Audio, Kind::Video].into_iter().all(|k| ops::slot_of(id, k).is_none())
}

fn judge(r: &Arc<Room>, u: &str, l: &str) {
    floor::run(r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    floor::run(r, Entry::of("ws_join", l), |f, _| Some(f.ws_join(l, true)));
    floor::run(r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
    let m = Msg::new(Mt::Request, None, 0, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

fn holder(r: &Room) -> Option<String> {
    r.floor.lock().ok().and_then(|f| f.holder())
}

fn speaker(r: &Room) -> Option<String> {
    r.slot(Kind::Audio).speaker().map(|s| s.as_str().to_string())
}

fn in_floor(r: &Room, u: &str) -> bool {
    r.floor.lock().map(|f| f.members.contains(u)).unwrap_or(false)
}

#[test]
fn t8_0_rooms_made_in_turn_all_show_their_slots() {
    let _g = serial();
    let (x, y) = (tag("t80-x"), tag("t80-y"));
    let base = floor::count();
    let rx = create(&x);
    let ry = create(&y);
    assert!(slot_is(&x, &rx) && slot_is(&y, &ry), "정:IMPL-FWD-ROOM control — two rooms made one after another both show their slots");
    destroy(&rx);
    assert!(no_slot(&x), "control — the removed room's slot is gone");
    assert!(slot_is(&y, &ry), "control — removing one room leaves the other room's slot");
    destroy(&ry);
    assert!(settle(|| floor::count() == base), "stand check — the executors of the removed rooms end");
}

#[test]
fn t8_1_inserting_a_room_makes_its_slot_reachable() {
    let _g = serial();
    let s = sfu();
    let id = tag("t81");
    let r = fresh(&id);
    assert!(no_slot(&id), "stand check — no slot for the id before the room is put");
    let (have, put) = s.rooms.insert(r.clone());
    assert!(put && Arc::ptr_eq(&have, &r), "stand check — the room set took this room");
    let shown = slot_is(&id, &r);
    destroy(&r);
    assert!(
        shown,
        "정:IMPL-ROOM-LIFE 정:IMPL-FWD-ROOM S-sym-IMPL-ROOM-LIFE-3 putting a room in the room set makes its slots visible on the packet path in that same step — not in a separate step that can be forgotten or raced"
    );
}

#[test]
fn t8_1b_removing_a_room_takes_out_only_its_slot() {
    let _g = serial();
    let s = sfu();
    let id = tag("t81b");
    let r = fresh(&id);
    let (have, put) = s.rooms.insert(r.clone());
    assert!(put && Arc::ptr_eq(&have, &r) && slot_is(&id, &r), "stand check — the room and its slots are in");
    let other = fresh(&id);
    let (kept, again) = s.rooms.insert(other.clone());
    assert!(!again && Arc::ptr_eq(&kept, &r), "정:IMPL-ROOM-LIFE a second put of one id answers the room already there");
    assert!(!s.rooms.remove(&other), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-4 removing another room of the same id removes nothing");
    assert!(
        s.rooms.get(&id).is_some_and(|x| Arc::ptr_eq(&x, &r)) && slot_is(&id, &r),
        "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-3 the room and its slots stay after a removal of another room of the same id"
    );
    assert!(s.rooms.remove(&r), "stand check — removing the room itself removes it");
    assert!(
        s.rooms.get(&id).is_none() && no_slot(&id),
        "정:IMPL-ROOM-LIFE 정:IMPL-FWD-ROOM S-sym-IMPL-ROOM-LIFE-3 taking a room out of the room set takes its slots off the packet path in that same step"
    );
    assert!(!s.rooms.remove(&r), "정:IMPL-ROOM-LIFE removing a room twice removes it once");
}

#[test]
fn t8_2_slot_index_matches_the_room_set_under_races() {
    const BASE: usize = 2_000;
    const THREADS: usize = 16;
    const ROUNDS: usize = 200;
    let _g = serial();
    let s = sfu();
    let before = floor::count();
    let base: Vec<Arc<Room>> = (0..BASE).map(|_| room(&tag("t82-base"))).collect();
    let prefix = tag("t82");
    let start = Arc::new(Barrier::new(THREADS + 1));
    let end = Arc::new(Barrier::new(THREADS + 1));
    let hs: Vec<_> = (0..THREADS)
        .map(|i| {
            let (start, end, prefix) = (start.clone(), end.clone(), prefix.clone());
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
                let mut failed = Vec::new();
                for round in 0..ROUNDS {
                    start.wait();
                    let res = rt.block_on(ops::rooms::create(&create_env(&format!("{prefix}-{round}-{i}"))));
                    if let Err(f) = res {
                        failed.push(format!("round {round}: {} {}", f.code, f.name));
                    }
                    end.wait();
                }
                failed
            })
        })
        .collect();
    let mut bad = 0;
    let mut first = String::new();
    for round in 0..ROUNDS {
        start.wait();
        end.wait();
        let ids: Vec<String> = (0..THREADS).map(|i| format!("{prefix}-{round}-{i}")).collect();
        let made: Vec<Arc<Room>> = ids.iter().filter_map(|id| s.rooms.get(id)).collect();
        assert_eq!(made.len(), THREADS, "stand check — every create of round {round} put its room");
        let lost: Vec<String> = made.iter().chain(base.iter()).filter(|r| !slot_is(&r.id, r)).map(|r| r.id.clone()).collect();
        if !lost.is_empty() {
            bad += 1;
            if first.is_empty() {
                first = format!("round {round} lost {} of {}: {:?}", lost.len(), made.len() + base.len(), &lost[..lost.len().min(4)]);
            }
        }
        for r in &made {
            destroy(r);
        }
        assert!(ids.iter().all(|id| no_slot(id)), "stand check — round {round}'s rooms are gone after removal");
    }
    let failed: Vec<String> = hs.into_iter().flat_map(|h| h.join().expect("create thread")).collect();
    for r in &base {
        destroy(r);
    }
    assert!(settle(|| floor::count() <= before), "stand check — the executors of the removed rooms end");
    assert!(failed.is_empty(), "stand check — every create answered Ok: {failed:?}");
    assert_eq!(
        bad, 0,
        "정:IMPL-ROOM-LIFE 정:IMPL-FWD-ROOM S-sym-IMPL-ROOM-LIFE-3 rooms made at the same time all keep their slots on the packet path — {bad} of {ROUNDS} rounds lost a room's slot (first: {first})"
    );
}

#[test]
fn t67_0_destroy_then_recreate_in_turn_gives_a_working_room() {
    let _g = serial();
    let id = tag("t670");
    let (u, l) = (tag("t670-u"), tag("t670-l"));
    let base = floor::count();
    let a = create(&id);
    assert_eq!(floor::count(), base + 1, "stand check — making a room starts one executor");
    destroy(&a);
    assert!(s_none(&id), "stand check — the first removal takes the room and its slot out");
    assert!(settle(|| floor::count() == base), "control — removing the room ends its executor");
    let b = create(&id);
    assert!(!Arc::ptr_eq(&a, &b), "stand check — the second make is a new room");
    assert!(slot_is(&id, &b), "control — the new room's slots are visible");
    judge(&b, &u, &l);
    assert_eq!(holder(&b).as_deref(), Some(u.as_str()), "control — the floor is judged in the new room");
    assert!(settle(|| speaker(&b).as_deref() == Some(u.as_str())), "control — the new room's executor binds the speaker on the new room's slot");
    assert_eq!(floor::count(), base + 1, "control — one executor for the new room");
    destroy(&b);
    assert!(settle(|| floor::count() == base), "control — removing the new room ends its executor");
}

fn s_none(id: &str) -> bool {
    sfu().rooms.get(id).is_none() && no_slot(id)
}

#[test]
fn t67_1_a_late_destroy_of_the_old_room_does_not_touch_the_recreated_room() {
    let _g = serial();
    let s = sfu();
    let id = tag("t671");
    let (u, l) = (tag("t671-u"), tag("t671-l"));
    let base = floor::count();
    let a = create(&id);
    destroy(&a);
    assert!(s_none(&id), "stand check — the first removal takes the room and its slot out");
    assert!(settle(|| floor::count() == base), "stand check — the old room's executor ended");
    let b = create(&id);
    assert!(!Arc::ptr_eq(&a, &b), "stand check — the second make is a new room");
    destroy(&a);
    let kept = s.rooms.get(&id).is_some_and(|x| Arc::ptr_eq(&x, &b));
    assert!(kept, "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-4 a late removal holding the old room does not remove the new room of the same id from the room set");
    assert!(slot_is(&id, &b), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-4 a late removal of the old room leaves the new room's slots on the packet path");
    judge(&b, &u, &l);
    assert_eq!(
        holder(&b).as_deref(),
        Some(u.as_str()),
        "정:IMPL-ROOM-LIFE 정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 the new room still judges the floor after a late removal of the old room"
    );
    assert!(
        settle(|| speaker(&b).as_deref() == Some(u.as_str())),
        "정:IMPL-ROOM-LIFE 정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 the new room's executor still runs and binds the speaker on the new room's slot"
    );
    assert_eq!(speaker(&a), None, "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-4 nothing of the new room is executed on the old room's slot");
    assert_eq!(floor::count(), base + 1, "정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 one executor — the new room's");
    destroy(&b);
    assert!(settle(|| floor::count() == base), "the executor count comes back once the new room is removed");
}

#[test]
fn t67_2_two_creates_of_one_id_start_one_executor() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 300;
    let _g = serial();
    let s = sfu();
    let before = floor::count();
    let prefix = tag("t672");
    let start = Arc::new(Barrier::new(THREADS + 1));
    let end = Arc::new(Barrier::new(THREADS + 1));
    let hs: Vec<_> = (0..THREADS)
        .map(|_| {
            let (start, end, prefix) = (start.clone(), end.clone(), prefix.clone());
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
                let mut out = Vec::new();
                for round in 0..ROUNDS {
                    start.wait();
                    let res = rt.block_on(ops::rooms::create(&create_env(&format!("{prefix}-{round}"))));
                    out.push(res.map(|v| v.get("created_at").and_then(|x| x.as_u64()).unwrap_or(0)).map_err(|f| format!("{} {}", f.code, f.name)));
                    end.wait();
                }
                out
            })
        })
        .collect();
    let mut made_at = Vec::new();
    let mut bad = 0;
    let mut first = String::new();
    for round in 0..ROUNDS {
        start.wait();
        end.wait();
        let id = format!("{prefix}-{round}");
        let room = s.rooms.get(&id).expect("stand check — the round's room is in the room set");
        made_at.push(room.created_at);
        turns(8);
        let n = Arc::strong_count(&room);
        if n != 3 {
            bad += 1;
            if first.is_empty() {
                first = format!("round {round}: {n} holders of the room (room set 1 + this test 1 + executors {})", n.saturating_sub(2));
            }
        }
        destroy(&room);
    }
    let answers: Vec<Vec<Result<u64, String>>> = hs.into_iter().map(|h| h.join().expect("create thread")).collect();
    assert!(settle(|| floor::count() <= before), "stand check — the executors of the removed rooms end");
    for (round, at) in made_at.iter().enumerate() {
        for a in &answers {
            assert_eq!(a[round].as_ref().ok(), Some(at), "stand check — every create of round {round} answered the one room of that id");
        }
    }
    assert_eq!(
        bad, 0,
        "정:IMPL-ROOM-LIFE 정:IMPL-FLR-LOCK 정:RULE-CONC-SIX S-sym-IMPL-ROOM-LIFE-4 creates of one id that overlap start one executor — {bad} of {ROUNDS} rounds started more (first: {first})"
    );
}

#[test]
fn t67_4_no_judging_without_a_running_executor() {
    let _g = serial();
    let r = room(&tag("t674"));
    let (u, l) = (tag("t674-u"), tag("t674-l"));
    let base = floor::count();
    let st = floor::run(&r, Entry::of("ws_join", &u), |f, _| Some(f.ws_join(&u, true)));
    assert!(
        st.out.is_empty() && st.fx.is_empty() && st.labels.is_empty() && !in_floor(&r, &u),
        "정:IMPL-FLR-LOCK a room whose executor was never started does not judge the floor"
    );
    assert!(floor::start(&r), "stand check — the room's executor starts");
    floor::run(&r, Entry::of("ws_join", &u), |f, _| Some(f.ws_join(&u, true)));
    assert!(in_floor(&r, &u), "stand check — once the executor is started the floor is judged");
    destroy(&r);
    let st = floor::run(&r, Entry::of("ws_join", &l), |f, _| Some(f.ws_join(&l, true)));
    assert!(st.out.is_empty() && st.fx.is_empty() && !in_floor(&r, &l), "정:IMPL-FLR-LOCK a removed room does not judge the floor");
    assert!(settle(|| floor::count() == base), "the executor ends once the room is removed");
}

#[test]
fn t67_5_a_stop_before_start_keeps_the_executor_off() {
    let _g = serial();
    let r = room(&tag("t675"));
    let base = floor::count();
    assert!(!floor::stop(&r), "정:IMPL-FLR-LOCK 정:IMPL-ROOM-LIFE a stop of a never-started room has no executor to wait for");
    assert!(!floor::start(&r), "정:IMPL-FLR-LOCK 정:IMPL-ROOM-LIFE a start after the stop starts nothing — the removed room keeps no running executor");
    assert!(!r.exec.running() && floor::count() == base, "정:IMPL-FLR-LOCK 정:IMPL-ROOM-LIFE no executor runs for the stopped room");
    destroy(&r);
    let fresh = room(&tag("t675-b"));
    assert!(floor::start(&fresh), "stand check — a fresh room still starts");
    destroy(&fresh);
    assert!(settle(|| floor::count() == base), "stand check — the fresh room's executor ends with it");
}

#[test]
fn t67_3_start_twice_runs_one_executor() {
    let _g = serial();
    let r = room(&tag("t673"));
    let base = floor::count();
    assert!(floor::start(&r), "stand check — the first start runs the executor");
    assert!(!floor::start(&r), "정:IMPL-FLR-LOCK 정:RULE-CONC-SIX S-sym-IMPL-ROOM-LIFE-4 a second start of the same room starts nothing");
    assert_eq!(floor::count(), base + 1, "정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 one executor for the room");
    assert_eq!(Arc::strong_count(&r), 3, "정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 the room is held by the room set, this test and one executor");
    destroy(&r);
    assert!(settle(|| floor::count() == base), "정:IMPL-FLR-LOCK S-sym-IMPL-ROOM-LIFE-4 the executor ends once its room is removed");
    assert!(!floor::start(&r), "정:IMPL-ROOM-LIFE a removed room does not start an executor again");
    assert_eq!(floor::count(), base, "정:IMPL-ROOM-LIFE no executor for a removed room");
}

async fn seen(sub: &mut oxmesh::TokenSub, key: &str, put: bool) -> bool {
    loop {
        match tokio::time::timeout(WAIT, sub.recv()).await {
            Ok(Some(oxmesh::TokenEvent::Put(k))) if put && k == key => return true,
            Ok(Some(oxmesh::TokenEvent::Delete(k))) if !put && k == key => return true,
            Ok(Some(_)) => continue,
            _ => return false,
        }
    }
}

#[test]
fn t610_2_destroying_a_room_withdraws_its_token() {
    let _g = serial();
    let r = room(&tag("t6102"));
    let key = format!("l1/room/{}", r.id);
    let (put, deleted) = drive(async {
        let mut rc = oxmesh::Config::router("l1-t6102");
        rc.listen = vec!["127.0.0.1:0".into()];
        let mesh = oxmesh::Mesh::open(rc).await.expect("in-process router");
        let mut sub = mesh.token_sub("l1/room/**", false).await.expect("token subscription");
        let unit = common::bus::Client::open("l1-t6102", "sfud", &mesh.bound()[0].to_string()).await.expect("sfud bus client");
        let t = unit.declare_token(&key).await.expect("room token");
        let put = seen(&mut sub, &key, true).await;
        r.hold_token(t);
        ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1").await;
        let deleted = seen(&mut sub, &key, false).await;
        (put, deleted)
    });
    assert!(put, "stand check — the in-process router shows the room token");
    assert!(r.st.lock().map(|s| s.token.is_none()).unwrap_or(false), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 a removed room holds no room token");
    assert!(deleted, "정:IMPL-ROOM-LIFE 정:RULE-BUS-PRESENCE S-sym-IMPL-ROOM-LIFE-5 removing a room withdraws its room token (the key's Delete is seen)");
}

#[test]
fn t610_1_a_token_that_lands_on_a_removed_room_goes_with_its_last_reference() {
    let _g = serial();
    let r = room(&tag("t6101"));
    let key = format!("l1/room/{}", r.id);
    let (put, held, only, alive, deleted) = drive(async move {
        let mut rc = oxmesh::Config::router("l1-t6101");
        rc.listen = vec!["127.0.0.1:0".into()];
        let mesh = oxmesh::Mesh::open(rc).await.expect("in-process router");
        let mut sub = mesh.token_sub("l1/room/**", false).await.expect("token subscription");
        let unit = common::bus::Client::open("l1-t6101", "sfud", &mesh.bound()[0].to_string()).await.expect("sfud bus client");
        let t = unit.declare_token(&key).await.expect("room token");
        let put = seen(&mut sub, &key, true).await;
        ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1").await;
        r.hold_token(t);
        let held = r.st.lock().map(|s| s.token.is_some()).unwrap_or(false);
        let only = Arc::strong_count(&r) == 1;
        let alive = mesh.tokens(&key, WAIT).await.map(|ks| ks.contains(&key)).unwrap_or(false);
        drop(r);
        let deleted = seen(&mut sub, &key, false).await;
        (put, held, only, alive, deleted)
    });
    assert!(put, "stand check — the in-process router shows the room token");
    assert!(held && only, "stand check — the late token sits in the removed room and this test holds that room's last reference");
    assert!(alive, "stand check — before the last reference goes the token is still declared");
    assert!(
        deleted,
        "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 a room token that comes back after its room was removed is withdrawn together with the room's last reference — it does not outlive the room"
    );
}

const WORKER_AREAS: [&str; 7] = ["fwd", "rtcp", "pub", "link", "trk", "sim", "sub"];

fn holds_room(t: &str) -> bool {
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    ["Arc<Room>", "Arc<room::Room>", "Arc<crate::room::Room>"].iter().any(|w| t.contains(w))
}

fn is_room_set_map(l: &Src) -> bool {
    l.file == "room/mod.rs" && l.text.trim() == "pub map: Mutex<HashMap<String, Arc<Room>>>,"
}

fn worker_area(file: &str) -> bool {
    WORKER_AREAS.iter().any(|a| file.starts_with(&format!("{a}/")))
}

#[test]
fn t_src_room_things_have_one_owner() {
    let lines = product_lines();
    assert!(lines.iter().any(|l| l.file == "ops/mod.rs" && l.func == "slot_of"), "stand check — the watch reads ops::slot_of");
    let mut bad = Vec::new();
    for l in &lines {
        let t = l.text.as_str();
        for w in ["static CLOCKS", "fn clocks", "publish_slots", "room_tokens", "undeclare_room"] {
            if t.contains(w) {
                bad.push(format!("{} has `{w}`", at(l)));
            }
        }
        if t.contains("slots.store(") && !(l.file == "room/mod.rs" && matches!(l.func.as_str(), "insert" | "remove")) {
            bad.push(format!("{} writes the slot index outside Rooms::insert / remove", at(l)));
        }
        if t.contains("rooms.remove(") && !(l.file == "ops/rooms.rs" && l.func == "destroy_room") {
            bad.push(format!("{} removes a room outside destroy_room", at(l)));
        }
        if t.contains("floor::start(") && !(l.file == "ops/rooms.rs" && l.func == "create") {
            bad.push(format!("{} starts an executor outside create", at(l)));
        }
        if t.contains("declare_room(") && !t.contains("fn declare_room") && !(l.file == "ops/rooms.rs" && matches!(l.func.as_str(), "create" | "redeclare")) {
            bad.push(format!("{} declares a room token outside create / redeclare", at(l)));
        }
        if ["static ", "pub static ", "pub(crate) static "].iter().any(|h| t.trim_start().starts_with(h))
            && ["RoomFloor", "SlotFan", "Token"].iter().any(|w| t.contains(w))
        {
            bad.push(format!("{} keeps a room thing in a process-wide table", at(l)));
        }
        if t.contains("HashMap<String, Arc<RoomFloor>>")
            || t.contains("HashMap<String, Token>")
            || (t.contains("HashMap<String, [Arc<SlotFan>; 2]>") && l.file != "room/mod.rs")
        {
            bad.push(format!("{} keys a room thing by room id outside the room set", at(l)));
        }
        if l.file == "ops/mod.rs" && l.func == "slot_of" && ["lock(", "rooms.get(", "rooms.all(", "rooms.map"].iter().any(|w| t.contains(w)) {
            bad.push(format!("{} takes a lock or reads the room set on the packet path", at(l)));
        }
        if holds_room(t) && !t.contains("fn ") && !is_room_set_map(l) {
            bad.push(format!("{} keeps an Arc<Room> outside the room set", at(l)));
        }
        if t.contains("rooms.map") && !(l.file == "ops/timers.rs" && l.func == "stat") {
            bad.push(format!("{} touches the room set map without Rooms::insert / remove", at(l)));
        }
        if worker_area(&l.file) && [".rooms.get(", ".rooms.all(", ".rooms.map", ".rooms.slots"].iter().any(|w| t.contains(w)) {
            bad.push(format!("{} reads the room set from a worker area", at(l)));
        }
    }
    let loads = lines.iter().filter(|l| l.file == "ops/mod.rs" && l.func == "slot_of" && l.text.contains(".load()")).count();
    if loads != 1 {
        bad.push(format!("ops/mod.rs slot_of reads the index {loads} times — one RCU read"));
    }
    let maps = lines.iter().filter(|l| holds_room(&l.text) && is_room_set_map(l)).count();
    let stat_reads = lines.iter().filter(|l| l.text.contains("rooms.map") && l.file == "ops/timers.rs" && l.func == "stat").count();
    let unread: Vec<&str> = WORKER_AREAS.iter().copied().filter(|a| !lines.iter().any(|l| l.file.starts_with(&format!("{a}/")))).collect();
    if maps != 1 || stat_reads != 1 || !unread.is_empty() {
        bad.push(format!(
            "stand check — the watch sees the room set map {maps} times · the stat length read {stat_reads} times · worker areas not read {unread:?}"
        ));
    }
    assert!(
        bad.is_empty(),
        "정:IMPL-ROOM-LIFE 정:RULE-CONC-HOT-PATH room things have one owner (the room, or the room set's own index) and the packet path reads the slot index with one RCU read:\n{}",
        bad.join("\n")
    );
}

#[test]
fn a_recreated_room_takes_the_token_permission_again() {
    let _g = serial();
    let id = tag("perm3");
    let r = room(&id);
    let down = Permission { publish_audio: false, publish_video: true, publish_screen: true, floor_request: false };
    r.join(member("a"), Some(Permission::ALL)).expect("registered");
    assert!(r.set_permission("a", down).is_some(), "stand check — bits lowered in the old room");
    drive(async { ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1").await });
    let again = room(&id);
    again.join(member("a"), Some(Permission::ALL)).expect("registered");
    assert_eq!(again.member("a").map(|m| m.permission), Some(Permission::ALL), "S-sym-IMPL-ROOM-PERMISSION-3③ the recreated room does not carry the destroyed room's override — re-join takes the token value");
    assert!(!Arc::ptr_eq(&r, &again), "stand check — a new room under the same id");
}

#[test]
fn one_user_in_two_rooms_of_one_server_is_one_peer() {
    let _g = serial();
    let (x, y) = (room(&tag("c2-x")), room(&tag("c2-y")));
    let u = subscriber(&tag("c2-u"));
    let c = ctx_of(&u.p);
    assert!(ops::rooms::join(&c, &json!({ "room_id": x.id })).is_ok(), "stand check — joins the first room");
    let first = sfu().peer(&u.p.user).expect("peer after the first join");
    assert!(ops::rooms::join(&c, &json!({ "room_id": y.id })).is_ok(), "stand check — joins the second room");
    let second = sfu().peer(&u.p.user).expect("peer after the second join");
    assert!(Arc::ptr_eq(&first, &second), "S-cond-RULE-DOC-COND-INDEX-SERVER-2 one user in two rooms of the same server keeps one Peer");
    assert!(second.in_room(&x.id) && second.in_room(&y.id) && x.has(&u.p.user) && y.has(&u.p.user), "S-cond-RULE-DOC-COND-INDEX-SERVER-2 both rooms hang on that one Peer — joining the second does not move the user out of the first");
}

#[test]
fn a_second_ready_notifies_the_floor_again_and_keeps_the_first_stall_origin() {
    let _g = serial();
    let mut f = Floor::new(10, 30);
    for u in ["a", "b"] {
        f.ws_join(u, true);
        f.ws_pub_select(u);
    }
    for i in 0..2 {
        let s = f.ready("b");
        assert_eq!(s.out.iter().filter(|o| o.to == "b" && o.m.t == Mt::Idle).count(), 1, "S-sym-IMPL-SUB-READY-GATE-2 READY #{i} with no speaker gets IDLE once");
    }
    f.server_rx("a", &Msg::new(Mt::Request, None, 5, 0), true, 0);
    for i in 0..2 {
        let s = f.ready("b");
        assert_eq!(s.out.iter().filter(|o| o.to == "b" && o.m.t == Mt::Taken).count(), 1, "S-sym-IMPL-SUB-READY-GATE-2 READY #{i} with a speaker gets TAKEN once");
    }
    let x = room(&tag("rg2-x"));
    let sub = subscriber(&tag("rg2-s"));
    enter(&sub.p, &x);
    let (first, _) = sub.t.ready(&x.id, 0, 1_000);
    let (second, _) = sub.t.ready(&x.id, 0, 5_000);
    assert!(first && !second, "stand check — the first READY is the first");
    assert_eq!(sub.t.ready_at(&x.id), Some(1_000), "S-sym-IMPL-SUB-READY-GATE-2 the stall origin stays at the first READY");
}
