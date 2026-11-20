// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FWD-ROOM · 정:IMPL-PUB-REMOVE · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-SUB-READY-GATE · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-LIFE · 정:IMPL-OPS-DESTROY-REAP · 정:IMPL-SUB-PIPELINE · 정:IMPL-GC-PEER-REAP · model: claude-opus-5-5
//! 1층 제어 층 — 전수 검사 둘째 묶음(context 202611/20261115a). 창 7: 반이중 발행의 방은 그 발행자의 `pub_room` 하나로 가른다(SR 줄 · `READY` 키프레임 요청 · 내림의 발언권 · 전이중 돌림 · `TAKEN` 출처 번호 · 허가 PLI). 창 6-1 · 6-2: 명단 등록이 닫힘 · 정원을 그 임계 구역 안에서 다시 보고, 방을 지우는 길은 내보내기 앞에 닫는다. 창 6-6: 등록 뒤 그 연결의 표를 한 번 더 센다. 창 5-6: 세션 폐기는 Peer 와 무관하게 그 세션의 `arev` 보관값을 지운다. 겹침은 시험이 잠금 하나(발행 스트림 · 명단)를 쥐어 세우고 관측으로 확인한다 — 잠 길이에 기대지 않는다.

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;

use common::envelope::CallKind;
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::gc::Why;
use oxsfud::link::{PathKey, Peer};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::r#pub::{PubStream, PubTrack};
use oxsfud::room::{Room, RoomInit};
use oxsfud::trk::slot_track_id;
use oxsig::msg::{Duplex, Kind, MediaCodecs};
use oxsig::{Code, Failure};
use serde_json::{Value, json};

use support::*;

type Joined = Result<Value, Failure>;

fn join_in_thread(p: &Arc<Peer>, room: &str) -> JoinHandle<Joined> {
    let c = ctx_of(p);
    let body = json!({ "room_id": room });
    std::thread::spawn(move || ops::rooms::join(&c, &body))
}

fn code(r: &Joined) -> Option<u16> {
    r.as_ref().err().map(|f| f.code)
}

fn brief(r: &Joined) -> String {
    match r {
        Ok(_) => "Ok".into(),
        Err(f) => format!("Err {} {}", f.code, f.name),
    }
}

fn kept(sid: &str) -> bool {
    sfu().tables.floors.lock().map(|g| g.contains_key(sid)).unwrap_or(false)
}

struct Half {
    r1: Arc<Room>,
    r2: Arc<Room>,
    u: Sub,
    l: Sub,
    a: Arc<PubStream>,
    at: Arc<PubTrack>,
    v: Arc<PubStream>,
    vt: Arc<PubTrack>,
}

fn half(name: &str) -> Half {
    let r1 = room(&tag(&format!("{name}-r1")));
    let r2 = room(&tag(&format!("{name}-r2")));
    let u = subscriber(&tag(&format!("{name}-u")));
    enter(&u.p, &r1);
    enter(&u.p, &r2);
    oxsfud::aff::set_pub_room(&u.p, Some(&r1.id));
    let l = subscriber(&tag(&format!("{name}-l")));
    enter(&l.p, &r1);
    let (a, at) = published(&u.p, &r2.id, Kind::Audio, Duplex::Half);
    let (v, vt) = published(&u.p, &r2.id, Kind::Video, Duplex::Half);
    Half { r1, r2, u, l, a, at, v, vt }
}

fn grant(h: &Half) {
    assert!(floor::start(&h.r1), "stand check — the room's executor starts");
    let (u, l) = (h.u.p.user.clone(), h.l.p.user.clone());
    floor::run(&h.r1, Entry::of("ws_join", &u), |f, _| Some(f.ws_join(&u, true)));
    floor::run(&h.r1, Entry::of("ws_join", &l), |f, _| Some(f.ws_join(&l, true)));
    floor::run(&h.r1, Entry::of("ws_pub_select", &u), |f, _| Some(f.ws_pub_select(&u)));
    let m = Msg::new(Mt::Request, None, 0, 0);
    floor::run(&h.r1, Entry::of("server_rx", &u), |f, t2| Some(f.server_rx(&u, &m, true, t2)));
}

fn holder(r: &Room) -> Option<String> {
    r.floor.lock().ok().and_then(|f| f.holder())
}

#[test]
fn t7_1_sr_rows_follow_pub_room() {
    let _g = serial();
    let h = half("t71");
    let want = row_of(&h.l.t, Kind::Audio, &slot_track_id(&h.r1.id, Kind::Audio));
    assert!(ops::slot_of(&h.r1.id, Kind::Audio).is_some(), "stand check — the slot map has R1");
    assert!(h.r1.slot(Kind::Audio).rows.load().iter().any(|r| Arc::ptr_eq(r, &want)), "stand check — L's R1 audio slot row is in R1's slot list");
    assert_eq!(h.a.room.load().as_str(), h.r2.id, "stand check — the half stream was published with room_id R2");
    let rows = ops::rows_of_source(&h.at);
    assert!(
        rows.iter().any(|r| Arc::ptr_eq(r, &want)),
        "정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 the SR of a half-duplex track goes to the slot rows of its publisher's pub_room (R1), not of the room named when publishing (R2): {} rows",
        rows.len()
    );
}

#[test]
fn t7_2_ready_asks_keyframe_of_the_speaker_in_pub_room() {
    let _g = serial();
    let h = half("t72");
    h.r1.slot(Kind::Video).bind(Some(Arc::new(h.u.p.user.clone())), 1);
    assert_eq!(h.r1.slot(Kind::Video).speaker().as_deref().map(|x| x.as_str()), Some(h.u.p.user.as_str()), "stand check — U is the R1 video slot speaker");
    assert_eq!(h.vt.pli_want.load(Ordering::Acquire), 0, "stand check — no keyframe request yet");
    let body = json!({ "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "arev": 0 });
    assert!(h.l.p.subscribe.path.load().is_none(), "stand check — L's subscribe transport has no nominated path yet");
    let res = ops::media::ready(&ctx_of(&h.l.p), &body);
    assert!(res.is_ok(), "stand check — READY accepted: {res:?}");
    assert_eq!(
        h.vt.pli_want.load(Ordering::Acquire),
        0,
        "정:IMPL-SUB-READY-GATE S-sym-IMPL-SUB-READY-GATE-3① READY before the subscribe transport has a path does not ask yet — the keyframe would vanish before the path"
    );
    h.l.p.subscribe.path.store(Some(Arc::new(PathKey::Udp("198.51.100.7:40000".parse().expect("addr")))));
    ops::media::ready_owed(&h.l.p.subscribe);
    assert_ne!(
        h.vt.pli_want.load(Ordering::Acquire),
        0,
        "정:IMPL-FWD-ROOM 정:IMPL-SUB-READY-GATE S-sym-IMPL-FWD-ROOM-2 S-sym-IMPL-SUB-READY-GATE-3② once the path is up the deferred READY in R1 asks a keyframe of the R1 slot speaker's half video published under another room name"
    );
}

#[test]
fn t7_3_unpublishing_half_releases_the_floor_in_pub_room() {
    let _g = serial();
    let h = half("t73");
    grant(&h);
    assert_eq!(holder(&h.r1).as_deref(), Some(h.u.p.user.as_str()), "stand check — U holds the R1 floor");
    let body = json!({ "action": "remove", "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "track_ids": [h.a.track_id, h.v.track_id] });
    let res = ops::media::publish(&ctx_of(&h.u.p), &body);
    assert!(res.is_ok(), "stand check — remove accepted: {res:?}");
    assert_ne!(
        holder(&h.r1).as_deref(),
        Some(h.u.p.user.as_str()),
        "정:IMPL-PUB-REMOVE 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 taking the half tracks down releases the floor in the publisher's pub_room first"
    );
}

#[test]
fn t7_4_turning_full_lists_in_pub_room() {
    let _g = serial();
    let h = half("t74");
    let _rt = sfu().rt.enter();
    let body = json!({ "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "track_id": h.v.track_id, "duplex": "full" });
    let res = ops::media::track_set(&ctx_of(&h.u.p), &body);
    assert!(res.is_ok(), "stand check — TRACK_SET accepted: {res:?}");
    assert_eq!(h.v.duplex(), Duplex::Full, "stand check — the track is full duplex now");
    let in_r1 = ops::media::room_streams(&h.r1.id).iter().any(|s| Arc::ptr_eq(s, &h.v));
    let in_r2 = ops::media::room_streams(&h.r2.id).iter().any(|s| Arc::ptr_eq(s, &h.v));
    assert!(
        h.v.room.load().as_str() == h.r1.id && in_r1 && !in_r2 && oxsfud::fwd::on_pub_room(&h.v, &h.r1.id),
        "정:IMPL-AFF-PUB-MOVE 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 a half track turned full is listed in the pub_room it flows in (room {} · in R1 {in_r1} · in R2 {in_r2})",
        h.v.room.load()
    );
}

#[test]
fn t7_8_turning_full_releases_the_floor_in_pub_room() {
    let _g = serial();
    let h = half("t78");
    grant(&h);
    assert_eq!(holder(&h.r1).as_deref(), Some(h.u.p.user.as_str()), "stand check — U holds the R1 floor");
    let _rt = sfu().rt.enter();
    let body = json!({ "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "track_id": h.a.track_id, "duplex": "full" });
    let res = ops::media::track_set(&ctx_of(&h.u.p), &body);
    assert!(res.is_ok(), "stand check — TRACK_SET accepted: {res:?}");
    assert_ne!(
        holder(&h.r1).as_deref(),
        Some(h.u.p.user.as_str()),
        "정:IMPL-AFF-PUB-MOVE 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 turning a half track full releases the floor in the pub_room it flows in"
    );
}

#[test]
fn t7_5_taken_carries_sources_and_grant_asks_keyframe() {
    let _g = serial();
    let h = half("t75");
    h.l.p.publish.dc_open.store(true, Ordering::Release);
    let mut rx = h.l.p.publish.dc_rx.lock().expect("dc_rx").take().expect("dc receiver");
    grant(&h);
    assert_eq!(holder(&h.r1).as_deref(), Some(h.u.p.user.as_str()), "stand check — U holds the R1 floor");
    let taken = drive(async {
        let end = tokio::time::Instant::now() + WAIT;
        loop {
            let Ok(Some(buf)) = tokio::time::timeout_at(end, rx.recv()).await else { return None };
            let Some(fr) = oxsig::dc::decode(&buf) else { continue };
            if let Some(m) = oxsig::floor::decode(fr.payload)
                && m.msg_type == oxsig::floor::TAKEN
            {
                return Some(m);
            }
        }
    });
    let want_pli = drive(async {
        let end = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < end {
            if h.vt.pli_want.load(Ordering::Acquire) != 0 {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        false
    });
    let taken = taken.expect("stand check — L received TAKEN");
    assert_eq!(
        h.r1.slot(Kind::Video).speaker().as_deref().map(|x| x.as_str()),
        Some(h.u.p.user.as_str()),
        "stand check — the executor bound U to the R1 video slot (the half video matches the room codec)"
    );
    let mut got = taken.sources();
    got.sort();
    let mut want = vec![h.a.csrc, h.v.csrc];
    want.sort();
    assert_eq!(
        got, want,
        "정:IMPL-FLR-MEDIA-BIND 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 TAKEN carries the source numbers of the speaker's half tracks published under another room name"
    );
    assert!(
        want_pli,
        "정:IMPL-FLR-MEDIA-BIND 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 the grant asks a keyframe of the speaker's half video published under another room name"
    );
}

#[test]
fn t7_7_half_slot_reads_the_pub_room_of_the_transport() {
    let _g = serial();
    let h = half("t77");
    let got = oxsfud::fwd::half_slot(&h.u.p.publish, Kind::Video);
    assert!(
        got.as_ref().is_ok_and(|(r, f)| r.as_str() == h.r1.id && Arc::ptr_eq(f, h.r1.slot(Kind::Video))),
        "정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 the half slot of a transport is the slot of its pub_room (R1)"
    );
    oxsfud::aff::set_pub_room(&h.u.p, Some(&h.r2.id));
    let moved = oxsfud::fwd::half_slot(&h.u.p.publish, Kind::Audio);
    assert!(
        moved.as_ref().is_ok_and(|(r, f)| r.as_str() == h.r2.id && Arc::ptr_eq(f, h.r2.slot(Kind::Audio))),
        "정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 one value swap moves the half slot (pub_select)"
    );
    oxsfud::aff::set_pub_room(&h.u.p, Some("t77-no-such-room"));
    assert_eq!(
        oxsfud::fwd::half_slot(&h.u.p.publish, Kind::Audio).err(),
        Some("prefan_gate"),
        "정:IMPL-FWD-ROOM 정:RULE-OPS-DROPS a pub_room without a slot is dropped as prefan_gate"
    );
    oxsfud::aff::set_pub_room(&h.u.p, None);
    assert_eq!(
        oxsfud::fwd::half_slot(&h.u.p.publish, Kind::Audio).err(),
        Some("no_pub_room"),
        "정:IMPL-FWD-ROOM 정:RULE-OPS-DROPS no pub_room — no half slot, dropped as no_pub_room"
    );
    let rows = ops::rows_of_source(&h.at);
    assert!(rows.is_empty(), "정:IMPL-FWD-ROOM without a pub_room the SR of a half track goes nowhere: {} rows", rows.len());
}

#[test]
fn t7_10_ready_skips_a_slot_speaker_whose_pub_room_moved() {
    let _g = serial();
    let h = half("t710");
    h.r1.slot(Kind::Video).bind(Some(Arc::new(h.u.p.user.clone())), 1);
    oxsfud::aff::set_pub_room(&h.u.p, Some(&h.r2.id));
    let body = json!({ "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "arev": 0 });
    let res = ops::media::ready(&ctx_of(&h.l.p), &body);
    assert!(res.is_ok(), "stand check — READY accepted: {res:?}");
    assert_eq!(
        h.vt.pli_want.load(Ordering::Acquire),
        0,
        "정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 a slot speaker left over from before the pub_room moved away is not asked a keyframe in R1 — its half tracks flow in R2 now"
    );
}

#[test]
fn t7_9_stall_detection_sees_the_speaker_slot_in_pub_room() {
    let _g = serial();
    let h = half("t79");
    let body = json!({ "room_id": h.r1.id, "join_id": format!("j-{}", h.r1.id), "arev": h.l.t.arev() });
    assert!(ops::media::ready(&ctx_of(&h.l.p), &body).is_ok(), "stand check — READY acknowledges L's table");
    grant(&h);
    let bound = drive(async {
        let end = tokio::time::Instant::now() + WAIT;
        while tokio::time::Instant::now() < end {
            if h.r1.slot(Kind::Audio).speaker().as_deref().map(|x| x.as_str()) == Some(h.u.p.user.as_str()) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        false
    });
    assert!(bound, "stand check — the executor bound U to the R1 audio slot");
    let tid = slot_track_id(&h.r1.id, Kind::Audio);
    let row = row_of(&h.l.t, Kind::Audio, &tid);
    assert!(row.open(), "stand check — L's R1 audio slot row is open");
    let got = oxsfud::ops::timers::eligible_from(&h.r1, &tid, &row, &h.l.p.user, common::now_ms());
    assert!(
        got.is_some(),
        "정:IMPL-ROOM-STALL 정:IMPL-FWD-ROOM S-sym-IMPL-FWD-ROOM-2 the stall detector judges the slot row of a speaker whose half tracks were published under another room name — the room is the speaker's pub_room, the same value the forwarding path reads"
    );
}

fn stream_room_readers(src: &[Src]) -> Vec<&Src> {
    src.iter()
        .filter(|l| (l.text.contains(".room.load") || l.text.contains(".room.store")) && !l.text.contains("down.room") && !l.text.contains("row.room"))
        .collect()
}

#[test]
fn t7_6_half_room_is_asked_only_through_pub_room() {
    let src = product_lines();
    let allowed: &[(&str, &str, usize)] = &[
        ("trk/mod.rs", "entry", 1),
        ("trk/mod.rs", "listed", 1),
        ("gc/mod.rs", "leave", 2),
        ("ops/media.rs", "voice", 1),
        ("ops/media.rs", "announce_add", 1),
        ("ops/media.rs", "unpublish", 2),
        ("ops/media.rs", "track_set", 1),
        ("ops/media.rs", "set_duplex_of", 1),
        ("ops/media.rs", "move_pub", 2),
        ("ops/rooms.rs", "resume", 1),
        ("ops/timers.rs", "stat", 1),
        ("fwd/mod.rs", "on_pub_room", 1),
    ];
    let mut bad = Vec::new();
    let readers = stream_room_readers(&src);
    for l in &readers {
        if !allowed.iter().any(|(f, func, _)| l.file == *f && l.func == *func) {
            bad.push(format!("the stream room is touched outside the list-room owners: {}", at(l)));
        }
    }
    for (f, func, n) in allowed {
        let got = readers.iter().filter(|l| l.file == *f && l.func == *func).count();
        if got > *n {
            bad.push(format!("{f} {func} touches the stream room {got} times (at most {n})"));
        }
    }
    for l in readers.iter().filter(|l| l.func == "set_duplex_of" && !l.text.contains(".room.store(")) {
        bad.push(format!("set_duplex_of reads the stream room instead of the pub_room it checked: {}", at(l)));
    }
    let lines_of = |f: &str, func: &str| -> Vec<&Src> { src.iter().filter(|l| l.file == f && l.func == func).collect() };
    for (f, func, pat) in [
        ("fwd/mod.rs", "forward", "half_slot("),
        ("rtcp/mod.rs", "pli_bound", "half_slot("),
        ("ops/mod.rs", "rows_of_source", "half_slot("),
        ("ops/timers.rs", "eligible_from", "half_slot("),
        ("ops/floor.rs", "slot_sources", "half_in("),
        ("ops/floor.rs", "slot_video", "half_in("),
        ("ops/media.rs", "ready_pli", "half_in("),
    ] {
        if !lines_of(f, func).iter().any(|l| l.text.contains(pat)) {
            bad.push(format!("{f} {func} does not ask the half room through {pat}"));
        }
    }
    let defs: Vec<&Src> = src.iter().filter(|l| l.text.contains("fn half_slot(")).collect();
    if defs.len() != 1 || defs[0].file != "fwd/mod.rs" {
        bad.push(format!("half_slot is defined once, in fwd/mod.rs: {:?}", defs.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let callers: Vec<&Src> = src.iter().filter(|l| l.text.contains("half_slot(") && !l.text.contains("fn half_slot(")).collect();
    for l in &callers {
        if !["forward", "pli_bound", "rows_of_source", "eligible_from", "half_in"].contains(&l.func.as_str()) {
            bad.push(format!("half_slot is called outside the half-room readers: {}", at(l)));
        }
    }
    let set = lines_of("ops/media.rs", "set_duplex_of");
    let store = set.iter().position(|l| l.text.contains("room.store("));
    let listed = set.iter().position(|l| l.text.contains("listed.store("));
    if !matches!((store, listed), (Some(a), Some(b)) if a < b) {
        bad.push(format!("set_duplex_of stores the list room before it lists the track: room.store at {store:?} · listed.store at {listed:?}"));
    }
    assert!(
        bad.is_empty(),
        "정:IMPL-FWD-ROOM 정:RULE-FWD-STEPS 정:IMPL-ROOM-STALL S-sym-IMPL-FWD-ROOM-2 the room of a half-duplex track is read from its publisher's pub_room through one function the forwarding path also calls — the stream's room field is the list room only: {bad:#?}"
    );
}

#[test]
fn t61_1_join_is_refused_when_the_room_closes_before_registration() {
    let _g = serial();
    let r = room(&tag("t611-r"));
    let a = stream(&tag("t611-v"), &r.id, Kind::Audio);
    list(&a, true);
    let u = subscriber(&tag("t611-u"));
    let held = a.tracks.lock().expect("tracks");
    let t1 = join_in_thread(&u.p, &r.id);
    let stood = wait_until(|| u.p.in_room(&r.id));
    r.close();
    drop(held);
    let res = t1.join().expect("t1");
    assert!(stood, "stand failed — the join never reached its own resync (Peer.rooms holds R) before the room closed");
    assert_eq!(
        code(&res),
        Some(Code::RoomNotFound.num()),
        "정:IMPL-ROOM-JOIN 정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-JOIN-4 a join whose registration comes after the room closed is refused with ROOM_NOT_FOUND: {}",
        brief(&res)
    );
    assert!(!r.has(&u.p.user), "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-4 the refused join is not in the roster of the closed room");
    assert!(!u.p.in_room(&r.id), "정:IMPL-ROOM-JOIN the refused join takes R out of Peer.rooms again");
    assert!(sfu().peer(&u.p.user).is_none(), "정:IMPL-ROOM-JOIN the refused join reaps the Peer that had no other room");
    let b = breaks(&u);
    assert!(b.is_empty(), "정:IMPL-ROOM-JOIN the rows the join loaded are taken out again: {b:?}");
}

#[test]
fn t61_2_refused_join_keeps_the_other_room() {
    let _g = serial();
    let r0 = room(&tag("t612-r0"));
    let r = room(&tag("t612-r"));
    let a = stream(&tag("t612-v"), &r.id, Kind::Audio);
    list(&a, true);
    let u = subscriber(&tag("t612-u"));
    enter(&u.p, &r0);
    let held = a.tracks.lock().expect("tracks");
    let t1 = join_in_thread(&u.p, &r.id);
    let stood = wait_until(|| u.p.in_room(&r.id));
    r.close();
    drop(held);
    let res = t1.join().expect("t1");
    assert!(stood, "stand failed — the join never reached its own resync");
    assert_eq!(code(&res), Some(Code::RoomNotFound.num()), "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-4 refused with ROOM_NOT_FOUND: {}", brief(&res));
    assert!(!r.has(&u.p.user) && !u.p.in_room(&r.id), "정:IMPL-ROOM-JOIN the refused join leaves neither the roster nor Peer.rooms holding R");
    assert!(u.p.in_room(&r0.id) && r0.has(&u.p.user), "정:IMPL-ROOM-JOIN the other room stays");
    assert!(sfu().peer(&u.p.user).is_some_and(|p| Arc::ptr_eq(&p, &u.p)), "정:IMPL-ROOM-JOIN the Peer that holds another room is not reaped");
    assert!(sfu().table(&u.p).is_some_and(|t| Arc::ptr_eq(&t, &u.t)), "the table stays registered");
    assert!(holds(&u.t, &slot_track_id(&r0.id, Kind::Audio), &r0.id), "the R0 slot row stays loaded");
    assert!(!holds(&u.t, &slot_track_id(&r.id, Kind::Audio), &r.id), "정:IMPL-ROOM-JOIN no row of R stays loaded");
    let b = breaks(&u);
    assert!(b.is_empty(), "정:IMPL-ROOM-JOIN lists equal the table after the back-out: {b:?}");
}

#[test]
fn t61_3_destroy_closes_before_emptying_the_roster() {
    let _g = serial();
    let r = room(&tag("t613-r"));
    let r0 = room(&tag("t613-r0"));
    let m = subscriber(&tag("t613-m"));
    enter(&m.p, &r);
    enter(&m.p, &r0);
    let b = stream(&tag("t613-b"), &r0.id, Kind::Audio);
    list(&b, true);
    let u = subscriber(&tag("t613-u"));
    let v = r.version();
    let mut env = envelope(CallKind::Admin, None, None);
    env.method = Some("POST".into());
    env.path = Some(format!("/admin/rooms/{}/destroy", r.id));
    env.body = json!({ "if_version": format!("{}:{}", v.epoch, v.seq) });
    let held = b.tracks.lock().expect("tracks");
    let d = std::thread::spawn(move || ops::admin::handle(&env));
    let stood = wait_until(|| !r.has(&m.p.user));
    let destroying = !d.is_finished();
    let res = ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": r.id }));
    drop(held);
    let out = d.join().expect("destroy");
    assert!(stood && destroying, "stand failed — the destroy never stood inside emptying the roster (M out of the roster · destroy still running)");
    assert!(out.is_ok(), "stand check — destroy answered: {out:?}");
    assert!(
        !r.has(&u.p.user),
        "정:IMPL-ROOM-LIFE 정:IMPL-OPS-DESTROY-REAP S-sym-IMPL-ROOM-JOIN-4 a join that started while the destroy was emptying the roster is not left in the roster of the removed room (join: {})",
        brief(&res)
    );
    assert_eq!(
        code(&res),
        Some(Code::RoomNotFound.num()),
        "정:IMPL-ROOM-LIFE the room is closed before its roster is read — the join is refused: {}",
        brief(&res)
    );
}

#[test]
fn t61_5_join_to_a_closed_room_is_refused_before_building_a_peer() {
    let _g = serial();
    let r = room(&tag("t615-r"));
    r.close();
    let user = tag("t615-u");
    let c = oxsfud::ops::handle::Ctx {
        user: user.clone(),
        sid: format!("sid-{user}"),
        session: format!("ses-{user}"),
        pc_mode: oxsig::msg::PcMode::Two,
        source_tag: false,
        home: None,
        claims: None,
        over: None,
    };
    let res = ops::rooms::join(&c, &json!({ "room_id": r.id }));
    assert_eq!(
        code(&res),
        Some(Code::RoomNotFound.num()),
        "정:IMPL-ROOM-JOIN 정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-JOIN-4 a join to a room already closed is refused early — no Peer is built for it (this harness has no media, so building one fails otherwise): {}",
        brief(&res)
    );
    assert!(sfu().peer(&user).is_none() && !r.has(&user), "정:IMPL-ROOM-JOIN nothing of the refused join stays");
}

#[test]
fn t61_4_expiry_closes_in_the_same_step() {
    let r = Room::new(RoomInit {
        id: "t614".into(),
        name: "t614".into(),
        capacity: 10,
        unused_ttl: 0,
        departure_ttl: 0,
        media_codecs: MediaCodecs::default(),
        epoch: "e".into(),
        floor: oxsfud::flr::core::Floor::new(10, 30),
        record: false,
    });
    assert!(r.ttl_due(common::now_ms(), 5_000), "stand check — the empty room is due");
    assert!(r.join(member("a"), None).is_err(), "정:IMPL-ROOM-LIFE 정:IMPL-ROOM-JOIN a closed room refuses the registration");
    assert!(r.closed(), "정:IMPL-ROOM-LIFE an expiry that saw the room empty and due closes it in the same critical section");
    assert!(!r.has("a"), "정:IMPL-ROOM-LIFE 정:IMPL-ROOM-JOIN a closed room takes no registration");
}

#[test]
fn t62_1_registration_recounts_capacity() {
    let _g = serial();
    let r = room_with(&tag("t621-r"), 1);
    let a = stream(&tag("t621-v"), &r.id, Kind::Audio);
    list(&a, true);
    let u1 = subscriber(&tag("t621-u1"));
    let held = a.tracks.lock().expect("tracks");
    let t1 = join_in_thread(&u1.p, &r.id);
    let stood = wait_until(|| u1.p.in_room(&r.id));
    r.join(member("t621-u2"), None).expect("U2 registered first");
    let one = r.user_count();
    drop(held);
    let res = t1.join().expect("t1");
    assert!(stood && one == 1, "stand failed — the join passed the early capacity check ({stood}) and U2 registered first (count {one})");
    assert!(
        r.user_count() <= r.capacity,
        "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-3 joins that passed the early check together never exceed the capacity: {} of {} ({})",
        r.user_count(),
        r.capacity,
        brief(&res)
    );
    assert_eq!(code(&res), Some(Code::RoomFull.num()), "정:IMPL-ROOM-JOIN the registration that sees a full room refuses with ROOM_FULL: {}", brief(&res));
    assert!(!u1.p.in_room(&r.id) && sfu().peer(&u1.p.user).is_none(), "정:IMPL-ROOM-JOIN the refused join backs out (Peer.rooms · Peer)");
}

#[test]
fn t62_3_rejoin_flag_does_not_skip_capacity_for_one_not_in_the_roster() {
    let r = Room::new(RoomInit {
        id: "t623".into(),
        name: "t623".into(),
        capacity: 1,
        unused_ttl: oxsfud::room::PERMANENT,
        departure_ttl: oxsfud::room::PERMANENT,
        media_codecs: MediaCodecs::default(),
        epoch: "e".into(),
        floor: oxsfud::flr::core::Floor::new(10, 30),
        record: false,
    });
    r.join(member("x"), None).expect("x registered");
    assert!(r.join(member("u"), None).is_err(), "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-3 the registration of one not in the roster sees the full room");
    assert!(
        !r.has("u") && r.user_count() == 1,
        "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-3 capacity is skipped only for one already in the roster — a rejoin whose old entry was kicked meanwhile is counted (count {})",
        r.user_count()
    );
}

#[test]
fn t62_4_one_in_the_roster_and_hidden_are_not_counted() {
    let mk = |id: &str| {
        Room::new(RoomInit {
            id: id.into(),
            name: id.into(),
            capacity: 1,
            unused_ttl: oxsfud::room::PERMANENT,
            departure_ttl: oxsfud::room::PERMANENT,
            media_codecs: MediaCodecs::default(),
            epoch: "e".into(),
            floor: oxsfud::flr::core::Floor::new(10, 30),
            record: false,
        })
    };
    let r = mk("t624");
    r.join(member("u"), None).expect("u registered");
    r.join(member("u"), None).expect("u registered again in the full room");
    assert!(r.has("u") && r.user_count() == 1, "정:IMPL-ROOM-JOIN one already in the roster registers again in a full room");
    let mut h = member("h");
    h.hidden = true;
    r.join(h, None).expect("hidden registered in the full room");
    assert!(r.has("h") && r.user_count() == 1, "정:IMPL-ROOM-JOIN hidden has no capacity axis");
}

#[test]
fn t62_5_registration_after_a_kick_announces_joined() {
    let r = Room::new(RoomInit {
        id: "t625".into(),
        name: "t625".into(),
        capacity: 10,
        unused_ttl: oxsfud::room::PERMANENT,
        departure_ttl: oxsfud::room::PERMANENT,
        media_codecs: MediaCodecs::default(),
        epoch: "e".into(),
        floor: oxsfud::flr::core::Floor::new(10, 30),
        record: false,
    });
    r.join(member("u"), None).expect("registered");
    let again = r.version().seq;
    r.join(member("u"), None).expect("registered again");
    assert_eq!(r.version().seq, again, "정:IMPL-ROOM-JOIN ② one already in the roster registers again without joined and without seq++");
    assert!(r.remove("u").is_some(), "stand check — the kick takes u out of the roster");
    r.announce_left("u");
    let kicked = r.version().seq;
    r.join(member("u"), None).expect("registered after the kick");
    assert!(
        r.has("u") && r.version().seq == kicked + 1,
        "정:IMPL-ROOM-JOIN ⑥ a rejoin that was kicked before its registration is a new entry — joined and seq++ (seq {} after the kick {kicked})",
        r.version().seq
    );
}

#[test]
fn t62_2_concurrent_joins_never_exceed_capacity() {
    let _g = serial();
    for round in 0..40 {
        let r = room_with(&tag("t622-r"), 3);
        let peers: Vec<Sub> = (0..8).map(|i| subscriber(&tag(&format!("t622-{round}-{i}")))).collect();
        let gate = Arc::new(std::sync::Barrier::new(peers.len()));
        let hs: Vec<JoinHandle<Joined>> = peers
            .iter()
            .map(|s| {
                let (c, body, g) = (ctx_of(&s.p), json!({ "room_id": r.id }), gate.clone());
                std::thread::spawn(move || {
                    g.wait();
                    ops::rooms::join(&c, &body)
                })
            })
            .collect();
        let res: Vec<Joined> = hs.into_iter().map(|h| h.join().expect("join")).collect();
        let ok = res.iter().filter(|x| x.is_ok()).count();
        assert!(
            res.iter().filter(|x| x.is_err()).all(|x| code(x) == Some(Code::RoomFull.num())),
            "round {round}: every refusal is ROOM_FULL: {:?}",
            res.iter().map(brief).collect::<Vec<_>>()
        );
        assert!(
            ok == 3 && r.user_count() == 3,
            "정:IMPL-ROOM-JOIN S-sym-IMPL-ROOM-JOIN-3 round {round}: {ok} joins succeeded · {} in the roster of capacity 3",
            r.user_count()
        );
    }
}

#[test]
fn t66_1_join_recounts_after_registration() {
    let _g = serial();
    let r = room(&tag("t661-r"));
    let other = tag("t661-v");
    let a = stream(&other, &r.id, Kind::Audio);
    list(&a, true);
    let s = stream(&other, &r.id, Kind::Video);
    let u = subscriber(&tag("t661-u"));
    let held = a.tracks.lock().expect("tracks");
    let t1 = join_in_thread(&u.p, &r.id);
    let stood = wait_until(|| u.p.in_room(&r.id));
    let roster = r.st.lock().expect("roster");
    drop(held);
    let counted = wait_until(|| holds(&u.t, &slot_track_id(&r.id, Kind::Audio), &r.id));
    let before = holds(&u.t, &s.track_id, &r.id);
    list(&s, true);
    drop(roster);
    let res = t1.join().expect("t1");
    assert!(
        stood && counted && !before,
        "stand failed — the join counted its table (stood {stood} · counted {counted}) before S was listed (already loaded {before})"
    );
    assert!(res.is_ok(), "stand check — the join succeeds: {res:?}");
    let in_res = res.as_ref().ok().map(|v| v["mline_table"].to_string().contains(&s.track_id)).unwrap_or(false);
    assert!(
        holds(&u.t, &s.track_id, &r.id) && in_res,
        "정:IMPL-ROOM-JOIN 정:IMPL-SUB-PIPELINE S-sym-IMPL-ROOM-JOIN-5 a track listed between the joiner's own count and its registration is loaded by one more count after registration (table {} · response {in_res})",
        holds(&u.t, &s.track_id, &r.id)
    );
    assert!(breaks(&u).is_empty(), "lists equal the table: {:?}", breaks(&u));
}

#[test]
fn t66_2_join_recounts_voice_order_after_registration() {
    let _g = serial();
    let r = room(&tag("t662-r"));
    let a = stream(&tag("t662-a"), &r.id, Kind::Audio);
    voiced(&a, 5);
    list(&a, true);
    let z = stream(&tag("t662-z"), &r.id, Kind::Audio);
    voiced(&z, 1);
    list(&z, true);
    for i in 0..24 {
        let o = stream(&tag("t662-o"), &r.id, Kind::Audio);
        voiced(&o, 10 + i);
        list(&o, true);
    }
    let u = subscriber(&tag("t662-u"));
    let held = a.tracks.lock().expect("tracks");
    let t1 = join_in_thread(&u.p, &r.id);
    let stood = wait_until(|| u.p.in_room(&r.id));
    let roster = r.st.lock().expect("roster");
    drop(held);
    let counted = wait_until(|| holds(&u.t, &slot_track_id(&r.id, Kind::Audio), &r.id));
    let before = holds(&u.t, &z.track_id, &r.id);
    voiced(&z, 1_000);
    drop(roster);
    let res = t1.join().expect("t1");
    assert!(stood && counted && !before, "stand failed — the join counted its table with Z left out (stood {stood} · counted {counted} · Z loaded {before})");
    assert!(res.is_ok(), "stand check — the join succeeds: {res:?}");
    assert!(
        holds(&u.t, &z.track_id, &r.id),
        "정:IMPL-ROOM-JOIN 정:IMPL-SUB-PIPELINE S-sym-IMPL-ROOM-JOIN-5 a voice-order change between the joiner's own count and its registration is taken in by one more count after registration"
    );
}

#[test]
fn t6_audit_registration_and_closing_owners() {
    let src = product_lines();
    let mut bad = Vec::new();
    let ins: Vec<&Src> = src.iter().filter(|l| l.text.contains("s.members.insert(")).collect();
    if ins.len() != 1 || ins[0].file != "room/mod.rs" || ins[0].func != "join" {
        bad.push(format!("members.insert only in room/mod.rs Room::join: {:?}", ins.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let closes: Vec<&Src> = src.iter().filter(|l| l.text.contains("s.closed = true")).collect();
    let mut owners: Vec<&str> = closes.iter().filter(|l| l.file == "room/mod.rs").map(|l| l.func.as_str()).collect();
    owners.sort();
    if closes.iter().any(|l| l.file != "room/mod.rs") || owners != ["close", "ttl_due"] {
        bad.push(format!("a room is closed only by Room::close and the expiry: {:?}", closes.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let destroy: Vec<&Src> = src.iter().filter(|l| l.text.contains("destroy_room(") && !l.text.contains("fn destroy_room(")).collect();
    let mut by: Vec<&str> = destroy.iter().map(|l| l.func.as_str()).collect();
    by.sort();
    if by != ["destroy", "sweep"] {
        bad.push(format!("destroy_room is called by the admin destroy and the sweep: {:?}", destroy.iter().map(|l| at(l)).collect::<Vec<_>>()));
    }
    let d: Vec<&Src> = src.iter().filter(|l| l.file == "ops/admin.rs" && l.func == "destroy").collect();
    let close = d.iter().position(|l| l.text.contains(".close()"));
    let users = d.iter().position(|l| l.text.contains(".users()"));
    if !matches!((close, users), (Some(c), Some(u)) if c < u) {
        bad.push(format!("the admin destroy closes the room before it reads the roster: close at {close:?} · users at {users:?}"));
    }
    assert!(bad.is_empty(), "정:IMPL-ROOM-JOIN 정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-JOIN-4 one registration owner · closing comes first: {bad:#?}");
}

#[test]
fn t56_1_session_gone_forgets_the_kept_generation_without_a_peer() {
    let _g = serial();
    let r = room(&tag("t561-r"));
    let u = subscriber(&tag("t561-u"));
    enter(&u.p, &r);
    assert!(oxsfud::gc::leave(&u.p, &r.id, Why::Leave, false), "leave finds the peer");
    assert!(sfu().peer(&u.p.user).is_none() && kept(&u.p.sid), "stand check — the last leave reaped the Peer and kept its arev");
    let res = ops::rooms::session_gone(&envelope(CallKind::SessionGone, Some(&u.p.user), Some(&u.p.sid)));
    assert!(res.is_ok());
    assert!(
        !kept(&u.p.sid),
        "정:IMPL-GC-PEER-REAP S-sym-IMPL-GC-PEER-REAP-3 the session's kept arev is forgotten when the session ends even if its Peer left first"
    );
}

#[test]
fn t56_2_old_session_gone_after_takeover_forgets_only_the_old_sid() {
    let _g = serial();
    let r = room(&tag("t562-r"));
    let user = tag("t562-u");
    let old = subscriber_as(&user, "t562-x");
    enter(&old.p, &r);
    oxsfud::gc::evict(&old.p, None, Why::Evict);
    let new = subscriber_as(&user, "t562-y");
    enter(&new.p, &r);
    assert!(kept("t562-x") && sfu().peer(&user).is_some_and(|p| Arc::ptr_eq(&p, &new.p)), "stand check — the old sid is kept and the new Peer is named");
    let res = ops::rooms::session_gone(&envelope(CallKind::SessionGone, Some(&user), Some("t562-x")));
    assert!(res.is_ok());
    assert!(
        !kept("t562-x"),
        "정:IMPL-GC-PEER-REAP S-sym-IMPL-GC-PEER-REAP-3 the old session's kept arev is forgotten even when the user's Peer is another session's"
    );
    assert!(sfu().peer(&user).is_some_and(|p| Arc::ptr_eq(&p, &new.p)), "the new Peer is not touched");
    assert!(sfu().table(&new.p).is_some_and(|t| Arc::ptr_eq(&t, &new.t)) && r.has(&user) && new.p.in_room(&r.id), "the new session stays in the room");
}

#[test]
fn t56_3_session_gone_with_its_peer_evicts_and_forgets() {
    let _g = serial();
    let r = room(&tag("t563-r"));
    let u = subscriber(&tag("t563-u"));
    enter(&u.p, &r);
    let res = ops::rooms::session_gone(&envelope(CallKind::SessionGone, Some(&u.p.user), Some(&u.p.sid)));
    assert!(res.is_ok());
    assert!(sfu().peer(&u.p.user).is_none() && !r.has(&u.p.user), "정:IMPL-GC-PEER-REAP the session's Peer is evicted");
    assert!(!kept(&u.p.sid), "정:IMPL-GC-PEER-REAP the kept arev is forgotten after the eviction kept it");
}

#[test]
fn t56_audit_kept_generation_owners() {
    let src = product_lines();
    let call = |pat: &str, def: &str| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            src.iter().filter(|l| l.text.contains(pat) && !l.text.contains(def)).map(|l| (l.file.clone(), l.func.clone())).collect();
        v.sort();
        v
    };
    let forget = call("tables.forget(", "fn forget(");
    let keep = call("tables.keep(", "fn keep(");
    assert_eq!(
        forget,
        vec![("ops/rooms.rs".to_string(), "session_gone".to_string()), ("ops/timers.rs".to_string(), "home_lost".to_string())],
        "정:IMPL-GC-PEER-REAP the kept arev is forgotten only where the end of a session reaches this server"
    );
    assert_eq!(keep, vec![("ops/media.rs".to_string(), "retire_table".to_string())], "정:IMPL-GC-PEER-REAP the arev is kept only when a table is retired");
}

#[test]
fn t57_layer_matching_sees_only_packets_with_a_send_path() {
    let src = product_lines();
    let admits: Vec<&Src> = src.iter().filter(|l| l.text.contains("sim::admit(")).collect();
    let callers: Vec<String> = admits.iter().map(|l| at(l)).collect();
    assert!(
        admits.len() == 1 && admits[0].file == "fwd/mod.rs" && admits[0].func == "send_down",
        "정:RULE-FWD-STEPS layer matching is called once, inside send_down: {callers:?}"
    );
    let path = src.iter().find(|l| l.file == "fwd/mod.rs" && l.func == "send_down" && l.text.contains("send_ok(")).map(|l| l.line);
    assert!(
        matches!(path, Some(p) if p < admits[0].line),
        "정:RULE-FWD-STEPS S-sym-RULE-FWD-STEPS-4① FWD.model.AdmitAfterPath layer matching (adopt · keyframe commit · retry PLI) runs after the send path check — before it, a joiner whose READY beats the nomination spends the PLI window and loses the keyframe: send_ok at {path:?} · admit at {}",
        admits[0].line
    );
}
