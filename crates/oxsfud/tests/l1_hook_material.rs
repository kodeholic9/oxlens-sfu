// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-EVENTS · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · model: claude-opus-5-5
//! 1층 사실 재료 — 프로세스 안 라우터(듣는 주소 127.0.0.1:0)에 sfud 버스를 붙이고 `int/ev/room/{R}` 를 라우터 쪽 구독으로 받아 hub 만 읽는 재료(표지 0xF001)를 본다: 방 생성(토큰을 쥔 뒤 · 멱등 생성은 0) · 방 소멸(TTL · 운영 폭파 — 닫히기 직전 요약) · 녹화 시작 · 끝(명단 앞뒤 견줌 · 투명이어도 · 둘째 녹화 참가자는 0) · 발언권 획득 · 비움 · 회수(전이마다 하나 · 낸 차례 · 판번호 없음). 버스는 프로세스에 하나라 이 파일에는 버스를 쓰는 시험만 둔다. 기다림은 구독의 다음 표본이고 시한(`WAIT`)은 상한일 뿐이다.

mod support;

use std::sync::{Arc, OnceLock};

use common::envelope::{CallKind, Material, Notice, OP_FACT_MATERIAL};
use common::keyspace::Keys;
use oxmesh::{Mesh, Subscriber};
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::room::{Member, Room};
use serde_json::json;

use support::*;

const NS: &str = "l1h";

static ROUTER: OnceLock<Mesh> = OnceLock::new();

fn router() -> &'static Mesh {
    ROUTER.get_or_init(|| {
        let s = sfu();
        drive(async {
            let mut cfg = oxmesh::Config::router("l1h-router");
            cfg.listen = vec!["127.0.0.1:0".into()];
            let mesh = Mesh::open(cfg).await.expect("in-process router");
            let at = mesh.bound().first().expect("router bound address").to_string();
            oxsfud::bus::Bus::open(&s.node, &s.epoch, NS, &at).await.expect("sfud bus on the in-process router");
            mesh
        })
    })
}

fn listen(id: &str) -> Subscriber {
    let mut sub = drive(router().subscribe(&Keys::new(NS).ev_room(id))).expect("ev/room subscription");
    let probe = Notice { op: 0, body: json!({ "probe": id }), exclude: Vec::new(), room_id: Some(id.to_string()), sfu_id: None };
    let ready = drive(async {
        for _ in 0..20 {
            oxsfud::bus::room_notice(id, probe.clone());
            if let Ok(Some(_)) = tokio::time::timeout(std::time::Duration::from_millis(200), sub.recv()).await {
                return true;
            }
        }
        false
    });
    assert!(ready, "stand check — the router-side ev/room subscription stands before the stimulus");
    let _ = materials_within(&mut sub, std::time::Duration::from_millis(300));
    sub
}

fn materials_within(sub: &mut Subscriber, wait: std::time::Duration) -> Vec<Material> {
    drive(async {
        let mut out = Vec::new();
        while let Ok(Some(s)) = tokio::time::timeout(wait, sub.recv()).await {
            let n: Notice = serde_json::from_slice(&s.payload).expect("notice JSON");
            if n.op == OP_FACT_MATERIAL {
                out.push(Material::of_notice(&n).expect("material body"));
            }
        }
        out
    })
}

fn materials(sub: &mut Subscriber) -> Vec<Material> {
    materials_within(sub, WAIT)
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn holds_token(r: &Room) -> bool {
    r.st.lock().map(|s| s.token.is_some()).unwrap_or(false)
}

fn recorder(user: &str) -> Member {
    Member { participant_type: 1, hidden: true, ..member(user) }
}

fn join(r: &Arc<Room>, u: &str) {
    floor::run(r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    floor::run(r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
}

fn request(r: &Arc<Room>, u: &str) {
    let m = Msg::new(Mt::Request, None, 1, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

fn release(r: &Arc<Room>, u: &str) {
    let m = Msg::new(Mt::Release, None, 0, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

#[test]
fn room_created_after_the_token_and_once() {
    let _g = serial();
    router();
    let id = tag("hm-create");
    let mut sub = listen(&id);
    let r = create(&id);
    assert!(holds_token(&r), "stand check — the room holds its token");
    let got = materials(&mut sub);
    assert_eq!(got.len(), 1, "HK-FACT-ROOM-CREATED① one room_created per make {got:?}");
    let Material::RoomCreated(sum) = &got[0] else { panic!("room_created {got:?}") };
    assert_eq!((sum.room_id.as_str(), sum.user_count, sum.rec), (id.as_str(), 0, false), "HK-FACT-ROOM-CREATED① the body is the room summary");
    let again = create(&id);
    assert!(Arc::ptr_eq(&r, &again), "stand check — the second make returns the same room");
    assert!(materials(&mut sub).is_empty(), "HK-FACT-ROOM-CREATED② an idempotent make is no fact");
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn room_closed_before_the_token_goes_with_the_last_summary() {
    let _g = serial();
    router();
    let id = tag("hm-ttl");
    let r = create(&id);
    let mut sub = listen(&id);
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "ttl"));
    assert!(!holds_token(&r), "stand check — the removed room holds no token");
    let got = materials(&mut sub);
    assert_eq!(got.len(), 1, "HK-FACT-ROOM-CLOSED① the ttl removal puts one room_closed {got:?}");
    let Material::RoomClosed(sum) = &got[0] else { panic!("room_closed {got:?}") };
    assert_eq!(sum.room_id, id, "HK-FACT-ROOM-CLOSED① the body is the summary just before closing");
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "ttl"));
    assert!(materials(&mut sub).is_empty(), "stand check — a second removal of the same room puts nothing");

    let id = tag("hm-destroy");
    let r = create(&id);
    r.join(recorder("rec-1"), None).expect("recorder registered");
    let mut sub = listen(&id);
    r.close();
    for u in r.users() {
        r.remove(&u);
    }
    drive(ops::rooms::destroy_room(&r, "OP-PATH-ADMIN-ROOMS-ROOM-ID-DESTROY", "destroy"));
    let got = materials(&mut sub);
    let names: Vec<&str> = got.iter().map(|m| m.name()).collect();
    assert_eq!(names, vec!["recording_ended", "room_closed"], "HK-FACT-ROOM-CLOSED③ the operator destroy empties the room first — the recording ends before the room closes {got:?}");
    let Material::RoomClosed(sum) = &got[1] else { panic!("room_closed {got:?}") };
    assert_eq!((sum.user_count, sum.rec), (0, false), "HK-FACT-ROOM-CLOSED③ the last summary is the emptied room");
}

#[test]
fn recording_starts_with_the_first_recorder_and_ends_with_the_last() {
    let _g = serial();
    router();
    let id = tag("hm-rec");
    let r = create(&id);
    let mut sub = listen(&id);
    r.join(member("hm-user"), None).expect("user registered");
    assert!(materials(&mut sub).is_empty(), "stand check — a plain member changes no rec");
    r.join(recorder("hm-rec-1"), None).expect("recorder registered");
    let got = materials(&mut sub);
    assert_eq!(got.len(), 1, "HK-FACT-RECORDING-STARTED① the first recorder starts the recording {got:?}");
    let Material::RecordingStarted(sum) = &got[0] else { panic!("recording_started {got:?}") };
    assert!(sum.rec && sum.room_id == id, "HK-FACT-RECORDING-STARTED④⑤ the body is the whole summary with rec true — a hidden recorder too {sum:?}");
    assert_eq!(sum.user_count, 1, "HK-FACT-RECORDING-STARTED⑤ the hidden recorder is not in user_count yet rec flips");
    r.join(recorder("hm-rec-2"), None).expect("second recorder registered");
    assert!(materials(&mut sub).is_empty(), "HK-FACT-RECORDING-STARTED② a second recorder is no fact");
    r.remove("hm-rec-1");
    assert!(materials(&mut sub).is_empty(), "HK-FACT-RECORDING-ENDED③ the recording goes on while a recorder stays");
    r.remove("hm-rec-2");
    let got = materials(&mut sub);
    assert_eq!(got.len(), 1, "HK-FACT-RECORDING-ENDED③ the last recorder leaving ends the recording {got:?}");
    let Material::RecordingEnded(sum) = &got[0] else { panic!("recording_ended {got:?}") };
    assert!(!sum.rec, "HK-FACT-RECORDING-ENDED④ the body is the whole summary with rec false");
    r.remove("hm-user");
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn floor_facts_follow_the_state_machine_in_order() {
    let _g = serial();
    router();
    let id = tag("hm-floor");
    let r = create(&id);
    let (a, b, c) = (tag("hm-a"), tag("hm-b"), tag("hm-c"));
    for u in [&a, &b, &c] {
        join(&r, u);
    }
    let mut sub = listen(&id);
    request(&r, &a);
    request(&r, &a);
    release(&r, &a);
    request(&r, &b);
    request(&r, &c);
    release(&r, &b);
    floor::run(&r, Entry::of("perm_down", &c), |f, _| Some(f.perm_down(&c)));
    let got = materials(&mut sub);
    let names: Vec<&str> = got.iter().map(|m| m.name()).collect();
    assert_eq!(
        names,
        vec!["floor_granted", "floor_released", "floor_granted", "floor_granted", "floor_revoked"],
        "HK-FACT-FLOOR-GRANTED①③⑤ · HK-FACT-FLOOR-RELEASED② · HK-FACT-FLOOR-REVOKED④ — one fact per transition in the order put · the re-grant is none · the queue hand-over is a grant without a release {got:?}"
    );
    let Material::FloorGranted(t) = &got[0] else { panic!("granted {got:?}") };
    assert!(t.speaker == a && t.talk.is_some() && t.room == id, "HK-FACT-FLOOR-GRANTED① the grant carries the speaker and the talk id {t:?}");
    let Material::FloorReleased(i) = &got[1] else { panic!("released {got:?}") };
    assert_eq!(i.prev.as_deref(), Some(a.as_str()), "HK-FACT-FLOOR-RELEASED② the release names the previous speaker and carries no talk id");
    let Material::FloorGranted(t2) = &got[2] else { panic!("granted {got:?}") };
    let Material::FloorGranted(t3) = &got[3] else { panic!("granted {got:?}") };
    assert!(t2.speaker == b && t3.speaker == c && t2.talk != t3.talk, "HK-FACT-FLOOR-GRANTED③ the queue hand-over grants the next speaker with a new talk id");
    let Material::FloorRevoked(v) = &got[4] else { panic!("revoked {got:?}") };
    assert_eq!(v.cause as u32, oxsig::floor::revoke::NO_PERMISSION as u32, "HK-FACT-FLOOR-REVOKED④ the revoke carries its cause");
    for m in &got {
        assert!(!serde_json::to_string(m).expect("json").contains("version"), "HK-FACT-FLOOR-GRANTED⑥ · HK-FACT-FLOOR-RELEASED⑥ · HK-FACT-FLOOR-REVOKED⑥ — no version on floor facts {m:?}");
    }
    drive(ops::rooms::destroy_room(&r, "SV-IMPL-ROOM-LIFE", "l1"));
}

#[test]
fn a_destroy_while_talking_closes_after_the_queued_floor_facts() {
    let _g = serial();
    router();
    let id = tag("hm-talking");
    let r = create(&id);
    let (a, b) = (tag("hm-talker"), tag("hm-listener"));
    join(&r, &a);
    join(&r, &b);
    let mut sub = listen(&id);
    const ROUNDS: usize = 400;
    for _ in 0..ROUNDS {
        request(&r, &a);
        release(&r, &a);
    }
    request(&r, &a);
    floor::run(&r, Entry::of("ws_leave", &a).because("destroy"), |f, _| Some(f.ws_leave(&a)));
    r.close();
    for u in r.users() {
        r.remove(&u);
    }
    drive(ops::rooms::destroy_room(&r, "OP-PATH-ADMIN-ROOMS-ROOM-ID-DESTROY", "destroy"));
    let got = materials(&mut sub);
    let names: Vec<&str> = got.iter().map(|m| m.name()).collect();
    let count = |n: &str| names.iter().filter(|x| **x == n).count();
    assert_eq!((count("floor_granted"), count("floor_released")), (ROUNDS + 1, ROUNDS + 1), "HK-FACT-ROOM-CLOSED⑤ the destroy loses no queued floor fact — every grant and release reaches the bus {names:?}");
    assert_eq!(names.last().copied(), Some("room_closed"), "HK-FACT-ROOM-CLOSED⑤ room_closed comes after the queued floor facts — the destroy waits for the room's executor to drain");
    assert_eq!(names[names.len() - 2], "floor_released", "HK-FACT-ROOM-CLOSED⑤ the talker sent out by the destroy is released before the room closes");
    assert_eq!(count("room_closed"), 1, "HK-FACT-ROOM-CLOSED③ one room_closed");
}
