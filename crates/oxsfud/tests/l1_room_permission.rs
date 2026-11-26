// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ROOM-PERMISSION · 정:RULE-HOOK-PERMISSION · 연:RULE-ROOM-PERMISSION-MAP · 연:RULE-PUB-REVOKED · 연:DEV-ROOM-HIDDEN-SELF-PSTATE · model: claude-opus-5-5
//! 1층 권한 집행 — 대외 권한 제어 Call(`CallKind::Permission`)을 sfud 에 넣고 프로세스 안 라우터(듣는 주소 127.0.0.1:0)의 `ev/room` · `ev/user` 구독으로 통지를 본다: ①멱등(통지 · seq 0) · 실패 코드 ②적용 ③회수(전이중 트랙 소급 제거 · 화자 회수 · 큐 빼기) ④통지(②③ 뒤 · 방 전원 · 투명이면 본인에게만 version 없이). 버스는 프로세스에 하나라 이 파일에는 버스를 쓰는 시험만 둔다.

mod support;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use common::envelope::{CallKind, Notice};
use common::keyspace::Keys;
use oxmesh::{Mesh, Subscriber};
use oxsfud::flr::core::{G, Msg, Mt};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::ops::permission::{FloorMove, floor_move, floor_perm};
use oxsfud::room::{Member, Room};
use oxsig::msg::{Duplex, Kind, Permission};
use oxsig::{Code, Failure, Op};
use serde_json::{Value, json};

use support::*;

const NS: &str = "l1p";

static ROUTER: OnceLock<Mesh> = OnceLock::new();

fn router() -> &'static Mesh {
    ROUTER.get_or_init(|| {
        let s = sfu();
        drive(async {
            let mut cfg = oxmesh::Config::router("l1p-router");
            cfg.listen = vec!["127.0.0.1:0".into()];
            let mesh = Mesh::open(cfg).await.expect("in-process router");
            let at = mesh.bound().first().expect("router bound address").to_string();
            oxsfud::bus::Bus::open(&s.node, &s.epoch, NS, &at).await.expect("sfud bus on the in-process router");
            mesh
        })
    })
}

fn probe(id: &str) -> Notice {
    Notice { op: 0, body: json!({ "probe": id }), exclude: Vec::new(), room_id: None, sfu_id: None }
}

fn ready(sub: &mut Subscriber, send: impl Fn()) {
    let ok = drive(async {
        for _ in 0..20 {
            send();
            if let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(200), sub.recv()).await {
                return true;
            }
        }
        false
    });
    assert!(ok, "stand check — the router-side subscription stands before the stimulus");
    let _ = notices_within(sub, Duration::from_millis(300));
}

fn listen_room(id: &str) -> Subscriber {
    let mut sub = drive(router().subscribe(&Keys::new(NS).ev_room(id))).expect("ev/room subscription");
    ready(&mut sub, || oxsfud::bus::room_notice(id, probe(id)));
    sub
}

fn listen_user(user: &str) -> Subscriber {
    let mut sub = drive(router().subscribe(&Keys::new(NS).ev_user(user))).expect("ev/user subscription");
    ready(&mut sub, || oxsfud::bus::user_notice(user, probe(user)));
    sub
}

fn notices_within(sub: &mut Subscriber, wait: Duration) -> Vec<Notice> {
    drive(async {
        let mut out = Vec::new();
        while let Ok(Some(s)) = tokio::time::timeout(wait, sub.recv()).await {
            let n: Notice = serde_json::from_slice(&s.payload).expect("notice JSON");
            if n.op != 0 {
                out.push(n);
            }
        }
        out
    })
}

fn notices(sub: &mut Subscriber) -> Vec<Notice> {
    notices_within(sub, WAIT)
}

fn ops_of(ns: &[Notice]) -> Vec<u16> {
    ns.iter().map(|n| n.op).filter(|op| *op != common::envelope::OP_FACT_MATERIAL).collect()
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn call(room: &str, user: &str, p: Permission) -> Result<Value, Failure> {
    let mut e = envelope(CallKind::Permission, None, None);
    e.target_user = Some(user.into());
    e.body = json!({ "room_id": room, "permission": p });
    ops::permission::apply(&e)
}

fn without(f: impl FnOnce(&mut Permission)) -> Permission {
    let mut p = Permission::ALL;
    f(&mut p);
    p
}

fn floor_join(r: &Arc<Room>, u: &str) {
    r.join(member(u), None).expect("registered");
    floor::run(r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    floor::run(r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
}

fn request(r: &Arc<Room>, u: &str) {
    let m = Msg::new(Mt::Request, None, 1, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

#[test]
fn the_same_value_is_nothing_and_failures_are_one_form() {
    let _g = serial();
    router();
    let id = tag("perm-same");
    let r = create(&id);
    r.join(member("ps-u"), None).expect("registered");
    let mut sub = listen_room(&id);
    let seq = r.version().seq;
    let got = call(&id, "ps-u", Permission::ALL);
    assert_eq!(got.ok(), Some(json!(Permission::ALL)), "S-rule-RULE-HOOK-PERMISSION-13① the success is the applied permission — the same value again too");
    assert!(ops_of(&notices(&mut sub)).is_empty(), "S-sym-IMPL-ROOM-PERMISSION-2① · S-rule-RULE-HOOK-PERMISSION-2① the same value sends no notice");
    assert_eq!(r.version().seq, seq, "S-sym-IMPL-ROOM-PERMISSION-2① · S-rule-RULE-HOOK-PERMISSION-2① the same value raises no seq");
    let not_in = call(&id, "ps-nobody", without(|p| p.publish_audio = false)).expect_err("not in the roster");
    assert_eq!(not_in.code, Code::NotInRoom.num(), "S-rule-RULE-HOOK-PERMISSION-13② · S-rule-RULE-HOOK-PERMISSION-10① a person not in the roster is refused NOT_IN_ROOM");
    assert!(r.permission_for("ps-nobody", None).is_default(), "S-rule-RULE-HOOK-PERMISSION-10② nothing is set ahead for that person");
    let no_room = call(&tag("perm-none"), "ps-u", without(|p| p.publish_audio = false)).expect_err("no room");
    assert_eq!(no_room.code, Code::RoomNotFound.num(), "S-rule-RULE-HOOK-PERMISSION-13③ a missing room is ROOM_NOT_FOUND");
    r.remove("ps-u");
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn lowering_a_publish_bit_removes_the_full_duplex_track_before_the_notice() {
    let _g = serial();
    router();
    let id = tag("perm-pub");
    let r = create(&id);
    let u = tag("pp-u");
    let sub_u = subscriber(&u);
    enter(&sub_u.p, &r);
    let (st, _) = published(&sub_u.p, &id, Kind::Audio, Duplex::Full);
    list(&st, true);
    let (vt, _) = published(&sub_u.p, &id, Kind::Video, Duplex::Full);
    list(&vt, true);
    let mut sub = listen_room(&id);
    let seq = r.version().seq;
    let new = without(|p| p.publish_audio = false);
    assert_eq!(call(&id, &u, new).ok(), Some(json!(new)), "S-rule-RULE-HOOK-PERMISSION-13① the success is the applied permission");
    assert!(sfu().pubs.get(&u).and_then(|c| c.find(&st.track_id)).is_none(), "S-sym-IMPL-ROOM-PERMISSION-1① · S-rule-RULE-HOOK-PERMISSION-3① the published audio track is removed at once");
    assert!(sfu().pubs.get(&u).and_then(|c| c.find(&vt.track_id)).is_some(), "stand check — the video track whose bit stays up stays");
    let got = notices(&mut sub);
    let ops = ops_of(&got);
    assert_eq!(ops, vec![Op::TrackEvent.as_u16(), Op::ParticipantState.as_u16()], "S-sym-IMPL-ROOM-PERMISSION-1② the track removal goes to the room first and the permission notice after it {ops:?}");
    let ps = got.iter().find(|n| n.op == Op::ParticipantState.as_u16()).expect("permission notice");
    assert!(ps.exclude.is_empty(), "S-rule-RULE-HOOK-PERMISSION-5① the permission notice goes to the whole room — the person too");
    assert_eq!(ps.body["permission"], json!(new), "S-rule-RULE-HOOK-PERMISSION-5① the notice carries the new permission");
    assert!(ps.body["version"]["seq"].as_u64().is_some_and(|s| s > seq), "S-rule-RULE-HOOK-PERMISSION-5① the notice carries the raised version");
    assert_eq!(r.member(&u).map(|m| m.permission), Some(new), "stand check — the roster holds the applied permission");
    r.remove(&u);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn lowering_the_floor_bit_revokes_the_speaker_and_dequeues_the_waiter() {
    let _g = serial();
    router();
    let id = tag("perm-floor");
    let r = create(&id);
    let (a, b, c) = (tag("pf-a"), tag("pf-b"), tag("pf-c"));
    for u in [&a, &b, &c] {
        floor_join(&r, u);
    }
    request(&r, &a);
    request(&r, &b);
    assert!(r.floor.lock().is_ok_and(|f| f.g == G::Taken && f.queue.iter().any(|w| w.u == b)), "stand check — a speaks and b waits");
    call(&id, &b, without(|p| p.floor_request = false)).expect("applied");
    assert!(r.floor.lock().is_ok_and(|f| !f.queue.iter().any(|w| w.u == b)), "S-sym-IMPL-ROOM-PERMISSION-4① · S-rule-RULE-HOOK-PERMISSION-7③ the waiter whose floor bit goes down leaves the queue");
    call(&id, &a, without(|p| p.floor_request = false)).expect("applied");
    assert!(
        r.floor.lock().is_ok_and(|f| f.g == G::Prevoke && f.revoking.as_deref() == Some(a.as_str())),
        "S-rule-RULE-HOOK-PERMISSION-3② · S-rule-RULE-HOOK-PERMISSION-7③ the speaker whose floor bit goes down is revoked at once"
    );
    call(&id, &a, Permission::ALL).expect("applied");
    assert!(r.floor.lock().is_ok_and(|f| f.parts.get(&a).is_some_and(|p| p.perm)), "stand check — raising the floor bit lets the person ask again");
    for u in [&a, &b, &c] {
        r.remove(u);
    }
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn a_hidden_person_hears_alone_without_version() {
    let _g = serial();
    router();
    let id = tag("perm-hidden");
    let r = create(&id);
    let h = tag("ph-h");
    r.join(Member { hidden: true, ..member(&h) }, None).expect("registered");
    let mut room_sub = listen_room(&id);
    let mut me = listen_user(&h);
    let seq = r.version().seq;
    let new = without(|p| p.publish_video = false);
    call(&id, &h, new).expect("applied");
    assert!(
        !ops_of(&notices(&mut room_sub)).contains(&Op::ParticipantState.as_u16()),
        "S-sym-IMPL-ROOM-PERMISSION-6① · S-rule-RULE-HOOK-PERMISSION-12① no permission notice of a hidden person reaches the room"
    );
    assert_eq!(r.version().seq, seq, "S-sym-IMPL-ROOM-PERMISSION-6① the hidden change raises no seq");
    let mine: Vec<Notice> = notices(&mut me).into_iter().filter(|n| n.op == Op::ParticipantState.as_u16()).collect();
    assert_eq!(mine.len(), 1, "S-sym-IMPL-ROOM-PERMISSION-6① · S-rule-RULE-HOOK-PERMISSION-12① the person alone gets one permission notice {mine:?}");
    assert!(mine[0].body.get("version").is_none(), "S-sym-IMPL-ROOM-PERMISSION-6① · S-rule-RULE-HOOK-PERMISSION-12① the person's own notice carries no version");
    assert_eq!(mine[0].body["permission"], json!(new), "stand check — the own notice carries the new permission");
    assert_eq!(r.member(&h).map(|m| m.permission), Some(new), "S-rule-RULE-HOOK-PERMISSION-12② the change still applies to a hidden person");
    r.remove(&h);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn the_floor_right_is_the_floor_bit_and_the_held_half_duplex_bits() {
    let all = Permission::ALL;
    let no_floor = without(|p| p.floor_request = false);
    let no_audio = without(|p| p.publish_audio = false);
    let no_video = without(|p| p.publish_video = false);
    assert_eq!(floor_perm(&all, &[Kind::Audio, Kind::Video]), (true, "allowed"), "stand check — every bit up");
    assert_eq!(floor_perm(&no_floor, &[]), (false, "floor_request"), "S-rule-RULE-HOOK-PERMISSION-7② the floor bit down takes the right to ask");
    assert_eq!(floor_perm(&no_audio, &[Kind::Audio]), (false, "publish_audio"), "S-rule-RULE-HOOK-PERMISSION-7② the publish bit of a held half-duplex kind down takes the right to ask");
    assert_eq!(floor_perm(&no_video, &[Kind::Video]), (false, "publish_video"), "S-rule-RULE-HOOK-PERMISSION-7② the same for video");
    assert_eq!(floor_perm(&no_audio, &[Kind::Video]), (true, "allowed"), "stand check — a kind not held as half-duplex does not count");
    assert_eq!(floor_move(true, false), FloorMove::Down, "stand check — the right goes");
    assert_eq!(floor_move(false, true), FloorMove::Up, "stand check — the right comes");
    assert_eq!((floor_move(true, true), floor_move(false, false)), (FloorMove::Keep, FloorMove::Keep), "stand check — nothing to move");
}

fn denied(step: &oxsfud::flr::core::Step, u: &str) -> Vec<u32> {
    step.out.iter().filter(|o| o.to == u && o.m.t == Mt::Deny).map(|o| o.m.n).collect()
}

fn ask(r: &Arc<Room>, u: &str) -> oxsfud::flr::core::Step {
    let m = Msg::new(Mt::Request, None, 1, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)))
}

#[test]
fn lowering_a_held_half_duplex_bit_takes_the_right_to_ask() {
    let _g = serial();
    router();
    let id = tag("perm-half");
    let r = create(&id);
    let (u, b) = (tag("ph2-u"), tag("ph2-b"));
    floor_join(&r, &b);
    let sub_u = subscriber(&u);
    enter(&sub_u.p, &r);
    floor::run(&r, Entry::of("ws_join", &u), |f, _| Some(f.ws_join(&u, true)));
    oxsfud::aff::set_pub_room(&sub_u.p, Some(&id));
    floor::run(&r, Entry::of("ws_pub_select", &u), |f, _| Some(f.ws_pub_select(&u)));
    let (st, _) = published(&sub_u.p, &id, Kind::Audio, Duplex::Half);
    list(&st, true);
    call(&id, &u, without(|p| p.publish_audio = false)).expect("applied");
    let step = ask(&r, &u);
    assert_eq!(denied(&step, &u), vec![oxsig::floor::deny::NO_PERMISSION as u32], "S-rule-RULE-HOOK-PERMISSION-7② the REQUEST of a person whose held half-duplex audio bit is down is denied 102");
    assert!(r.floor.lock().is_ok_and(|f| f.g != G::Taken || f.speaker.as_deref() != Some(u.as_str())), "S-rule-RULE-HOOK-PERMISSION-7② the person does not get the floor");
    assert!(sfu().pubs.get(&u).and_then(|c| c.find(&st.track_id)).is_some(), "stand check — the half-duplex track itself is not removed");
    r.remove(&u);
    r.remove(&b);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn joining_again_with_a_lowered_held_half_duplex_bit_has_no_right_to_ask() {
    let _g = serial();
    router();
    let id = tag("perm-join");
    let r = create(&id);
    let (u, b) = (tag("pj-u"), tag("pj-b"));
    floor_join(&r, &b);
    let no_audio = without(|p| p.publish_audio = false);
    r.join(member(&u), None).expect("registered");
    r.set_permission(&u, no_audio).expect("override set");
    r.remove(&u);
    let sub_u = subscriber(&u);
    let (st, _) = published(&sub_u.p, &id, Kind::Audio, Duplex::Half);
    list(&st, true);
    oxsfud::aff::set_pub_room(&sub_u.p, Some(&id));
    let res = ops::rooms::join(&ctx_of(&sub_u.p), &json!({ "room_id": id }));
    assert!(res.is_ok(), "stand check — the join answers {res:?}");
    assert_eq!(r.member(&u).map(|m| m.permission), Some(no_audio), "S-sym-IMPL-ROOM-PERMISSION-3① the override the room remembers comes back on the join");
    floor::run(&r, Entry::of("ws_pub_select", &u), |f, _| Some(f.ws_pub_select(&u)));
    let step = ask(&r, &u);
    assert_eq!(denied(&step, &u), vec![oxsig::floor::deny::NO_PERMISSION as u32], "S-rule-RULE-HOOK-PERMISSION-7② joining with the held half-duplex audio bit down gives no right to ask — the REQUEST is denied 102");
    r.remove(&u);
    r.remove(&b);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn ctl_failures_are_one_failure_form_with_old_codes_and_no_credential_failure() {
    let _g = serial();
    router();
    let id = tag("perm-fail");
    let r = create(&id);
    let u = tag("pfl-u");
    r.join(member(&u), None).expect("registered");
    let no_room = call(&tag("perm-fail-none"), &u, Permission::ALL).expect_err("no room");
    assert!(no_room.code == 3001 && no_room.code == Code::RoomNotFound.num(), "S-rule-RULE-HOOK-PERMISSION-14① — 없는 방은 Failure · 이미 있는 번호 3001 ROOM_NOT_FOUND(새 코드 0) {no_room:?}");
    let not_in = call(&id, "pfl-nobody", Permission::ALL).expect_err("not in the roster");
    assert!(not_in.code == 3002 && not_in.code == Code::NotInRoom.num(), "S-rule-RULE-HOOK-PERMISSION-14① — 명단에 없는 사람은 Failure · 이미 있는 번호 3002 NOT_IN_ROOM {not_in:?}");
    let mut bad = envelope(CallKind::Permission, None, None);
    bad.target_user = Some(u.clone());
    bad.body = json!({ "room_id": id, "permission": "all" });
    let unreadable = ops::permission::apply(&bad).expect_err("unreadable permission");
    let mut nobody = envelope(CallKind::Permission, None, None);
    nobody.body = json!({ "room_id": id, "permission": Permission::ALL });
    let untargeted = ops::permission::apply(&nobody).expect_err("no target user");
    let none = without(|p| {
        p.publish_audio = false;
        p.publish_video = false;
        p.publish_screen = false;
        p.floor_request = false;
    });
    assert_eq!(call(&id, &u, none).ok(), Some(json!(none)), "stand check — every bit goes down");
    assert_eq!(
        call(&id, &u, Permission::ALL).ok(),
        Some(json!(Permission::ALL)),
        "S-rule-RULE-HOOK-PERMISSION-14② — 호출자 신원이 없는(user_id · sid 없음) ctl 이 비트가 모두 내려간 사람을 올려도 자격을 보지 않고 적용된다(자격은 붙을 때 본다)"
    );
    for f in [&no_room, &not_in, &unreadable, &untargeted] {
        assert_ne!(f.code, Code::NotAuthorized.num(), "S-rule-RULE-HOOK-PERMISSION-14② — ctl 의 실패에 자격 실패(2006)는 없다 {f:?}");
    }
    r.remove(&u);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn lowering_bits_takes_back_what_is_up_now_and_refuses_the_next_publish() {
    let _g = serial();
    router();
    let id = tag("perm-now");
    let r = create(&id);
    let (u, b) = (tag("pn-u"), tag("pn-b"));
    floor_join(&r, &b);
    let sub_u = subscriber(&u);
    enter(&sub_u.p, &r);
    oxsfud::aff::set_pub_room(&sub_u.p, Some(&id));
    let (st, _) = published(&sub_u.p, &id, Kind::Audio, Duplex::Full);
    list(&st, true);
    floor::run(&r, Entry::of("ws_join", &u), |f, _| Some(f.ws_join(&u, true)));
    floor::run(&r, Entry::of("ws_pub_select", &u), |f, _| Some(f.ws_pub_select(&u)));
    request(&r, &u);
    assert!(r.floor.lock().is_ok_and(|f| f.g == G::Taken && f.speaker.as_deref() == Some(u.as_str())), "stand check — u publishes audio and holds the floor");
    let down = without(|p| {
        p.publish_audio = false;
        p.floor_request = false;
    });
    call(&id, &u, down).expect("applied");
    let gone = sfu().pubs.get(&u).and_then(|c| c.find(&st.track_id)).is_none();
    let revoked = r.floor.lock().is_ok_and(|f| f.g == G::Prevoke && f.revoking.as_deref() == Some(u.as_str()));
    assert!(
        gone && revoked,
        "S-rule-RULE-HOOK-PERMISSION-3③ — ctl 이 돌아온 그때 이미 올라온 트랙은 내려가 있고 발언은 회수 중이다(그 사람의 다음 발행 · 발언 요청까지 미루지 않는다) track gone {gone} · revoked {revoked}"
    );
    let body = json!({ "room_id": id, "join_id": format!("j-{id}"), "tracks": [{ "kind": "audio", "ssrc": 0x5b5b_0001u32, "mid": "7", "pt": 111, "duplex": "full" }] });
    let next = ops::media::publish(&ctx_of(&sub_u.p), &body);
    assert!(
        next.as_ref().err().is_some_and(|f| f.code == 2006 && f.code == Code::NotAuthorized.num() && f.details == Some(json!({ "denied": ["publish_audio"] }))),
        "S-rule-RULE-HOOK-PERMISSION-7① — ctl 로 비트를 내린 뒤의 발행은 2006 · details.denied = 막힌 비트 이름 배열 {next:?}"
    );
    r.remove(&u);
    r.remove(&b);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}
