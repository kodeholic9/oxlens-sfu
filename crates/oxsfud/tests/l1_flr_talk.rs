// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-TALK-REPORT · 연:RULE-CHAT-TALK · 연:RULE-CHAT-TALK-STATE · 연:RULE-FLR-TLV · 연:RULE-FLR-SERVER-EVENTS · model: claude-opus-5-5
//! 1층 발언 알리기 — 허가마다 발언 id(`81330e4c` + 허가 ms + 난수) · `start` 의 `deadline` = 허가 + T1 + T2 + T3 · 끝난 까닭마다 상태 · 방마다 한 줄(끝은 그 시작 뒤 · 승계는 앞 발언의 끝 다음에 새 시작) · 재허가는 같은 id 로 `regrant` · 발언권 TLV `0x20` 에 같은 id. 시험은 알림 줄을 보고자 먼저 꺼내 쥔다(집행자는 알리미를 띄우지 않는다).

mod support;

use std::sync::Arc;

use common::talk::Report;
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops::floor::{self, Entry};
use oxsfud::room::Room;
use oxsig::msg::TalkState;
use tokio::sync::mpsc::UnboundedReceiver;

use support::*;

fn started(id: &str) -> (Arc<Room>, UnboundedReceiver<Report>) {
    let r = room(&tag(id));
    let rx = r.exec.talks.claim().expect("stand check — the talk line is still held by the room");
    assert!(floor::start(&r), "stand check — the room's executor starts");
    (r, rx)
}

fn join(r: &Arc<Room>, u: &str) {
    floor::run(r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
    floor::run(r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
}

fn request(r: &Arc<Room>, u: &str, prio: u32) {
    let m = Msg::new(Mt::Request, None, prio, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

fn release(r: &Arc<Room>, u: &str) {
    let m = Msg::new(Mt::Release, None, 0, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

fn heard_for(r: &Room, ms: u64) {
    while floor::now_ms() <= ms {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut ck = r.exec.clock.lock().expect("clock");
    ck.first_rtp_at = Some(floor::now_ms().saturating_sub(ms));
}

fn drain(rx: &mut UnboundedReceiver<Report>) -> Vec<Report> {
    let mut v = Vec::new();
    while let Ok(x) = rx.try_recv() {
        v.push(x);
    }
    v
}

fn end_of(r: &Report) -> (TalkState, Option<u8>, u64) {
    match r {
        Report::End { state, reason, duration_ms, .. } => (*state, *reason, *duration_ms),
        other => panic!("expected end, got {other:?}"),
    }
}

#[test]
fn a_grant_starts_one_talk_and_the_release_ends_it() {
    let _g = serial();
    let (r, mut rx) = started("talk1");
    let (a, b) = (tag("t1-a"), tag("t1-b"));
    join(&r, &a);
    join(&r, &b);
    let before = common::now_ms();
    request(&r, &a, 5);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 1, "정:IMPL-FLR-TALK-REPORT 3 one grant = one start {got:?}");
    let Report::Start { talk_id, room_id, user_id, start, deadline, sfu_id } = &got[0] else { panic!("start first {got:?}") };
    assert_eq!((room_id.as_str(), user_id.as_str(), sfu_id.as_str()), (r.id.as_str(), a.as_str(), r.epoch.as_str()));
    assert!(talk_id.len() == 36 && talk_id.starts_with(common::talk::SYS_HASH), "연:RULE-CHAT-MSG-ID the talk id is sys hash + grant ms + random {talk_id}");
    assert_eq!(&talk_id[8..20], format!("{:012x}", start & 0xffff_ffff_ffff), "the time field is the grant ms");
    assert!(*start >= before && *start <= common::now_ms());
    assert_eq!(*deadline, start + floor::T1_MS + 30_000 + floor::T3_MS, "정:IMPL-FLR-TALK-REPORT 2 deadline = grant + T1 + T2 + T3");
    let held = r.exec.clock.lock().ok().and_then(|c| c.talk.clone()).map(|t| t.id);
    assert_eq!(held.as_deref(), Some(talk_id.as_str()), "the room holds the open talk id that GRANTED · TAKEN carry");
    heard_for(&r, 900);
    release(&r, &a);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].talk_id(), talk_id, "the end names the same talk");
    let (state, reason, dur) = end_of(&got[0]);
    assert_eq!((state, reason), (TalkState::Ok, None), "연:RULE-CHAT-TALK-STATE a release past the minimum is ok");
    assert!((900..2_000).contains(&dur), "duration counts from the first RTP {dur}");
    assert!(r.exec.clock.lock().map(|c| c.talk.is_none()).unwrap_or(false), "no open talk after idle");
}

#[test]
fn a_short_or_silent_talk_still_ends_with_its_state() {
    let _g = serial();
    let (r, mut rx) = started("talk2");
    let (a, b) = (tag("t2-a"), tag("t2-b"));
    join(&r, &a);
    join(&r, &b);
    request(&r, &a, 5);
    release(&r, &a);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(got[0], Report::Start { .. }) && got[1].talk_id() == got[0].talk_id(), "S-sym-IMPL-FLR-TALK-REPORT-2 — a release right after the grant still queues its end behind its start on the one line {got:?}");
    assert_eq!(end_of(&got[1]), (TalkState::Short, None, 0), "S-sym-IMPL-FLR-TALK-REPORT-2 연:RULE-CHAT-TALK-STATE no RTP and released = short with length 0");
    request(&r, &a, 5);
    heard_for(&r, 100);
    release(&r, &a);
    let got = drain(&mut rx);
    assert_eq!(end_of(&got[1]).0, TalkState::Short, "under floor.talk_min_ms(500) is short");
    request(&r, &a, 5);
    floor::run(&r, Entry::of("t1", ""), |f, _| Some(f.t1()));
    let got = drain(&mut rx);
    assert_eq!(end_of(&got[1]), (TalkState::Silent, None, 0), "연:RULE-CHAT-TALK-STATE T1 without any RTP = silent");
    request(&r, &a, 5);
    heard_for(&r, 3_000);
    floor::run(&r, Entry::of("t1", ""), |f, _| Some(f.t1()));
    let got = drain(&mut rx);
    assert_eq!(end_of(&got[1]).0, TalkState::Lost, "T1 after RTP stopped = lost");
}

#[test]
fn succession_ends_the_cut_talk_before_the_next_starts() {
    let _g = serial();
    let (r, mut rx) = started("talk3");
    let (a, b) = (tag("t3-a"), tag("t3-b"));
    join(&r, &a);
    join(&r, &b);
    request(&r, &a, 1);
    request(&r, &b, 9);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 1, "a preempt only revokes — the talk is still open {got:?}");
    let first = got[0].talk_id().to_string();
    release(&r, &a);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].talk_id(), first, "정:IMPL-FLR-TALK-REPORT 4 the cut talk ends first on the one line");
    assert_eq!(end_of(&got[0]).0..=end_of(&got[0]).0, TalkState::Cut..=TalkState::Cut);
    assert_eq!(end_of(&got[0]).1, Some(oxsig::floor::revoke::PREEMPTED), "cut carries the revoke cause as is");
    let Report::Start { user_id, talk_id, .. } = &got[1] else { panic!("then the successor starts {got:?}") };
    assert_eq!(user_id, &b);
    assert_ne!(talk_id, &first, "a new grant is a new talk id");
}

#[test]
fn a_regrant_keeps_the_talk_id_and_moves_the_deadline() {
    let _g = serial();
    let (r, mut rx) = started("talk4");
    let (a, b) = (tag("t4-a"), tag("t4-b"));
    join(&r, &a);
    join(&r, &b);
    request(&r, &a, 5);
    let got = drain(&mut rx);
    let Report::Start { talk_id, deadline, .. } = &got[0] else { panic!() };
    std::thread::sleep(std::time::Duration::from_millis(5));
    request(&r, &a, 5);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 1, "{got:?}");
    let Report::Regrant { talk_id: id2, deadline: d2 } = &got[0] else { panic!("regrant {got:?}") };
    assert_eq!(id2, talk_id, "정:IMPL-FLR-TALK-REPORT 1 a regrant is the same talk");
    assert!(d2 > deadline, "the deadline counts again from the regrant");
}

#[test]
fn leaving_with_the_floor_is_ok_but_a_lost_session_is_lost() {
    let _g = serial();
    for (why, want) in [("leave", TalkState::Ok), ("session_gone", TalkState::Lost), ("kick", TalkState::Lost)] {
        let (r, mut rx) = started("talk5");
        let (a, b) = (tag("t5-a"), tag("t5-b"));
        join(&r, &a);
        join(&r, &b);
        request(&r, &a, 5);
        heard_for(&r, 900);
        floor::run(&r, Entry::of("ws_leave", &a).because(why), |f, _| Some(f.ws_leave(&a)));
        let got = drain(&mut rx);
        assert_eq!(end_of(&got[1]).0, want, "연:RULE-FLR-SERVER-EVENTS {why}");
    }
}

#[test]
fn a_room_stopped_with_an_open_talk_ends_it_as_lost() {
    let _g = serial();
    let (r, mut rx) = started("talk6");
    let (a, b) = (tag("t6-a"), tag("t6-b"));
    join(&r, &a);
    join(&r, &b);
    request(&r, &a, 5);
    floor::stop(&r);
    let got = drain(&mut rx);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(end_of(&got[1]).0, TalkState::Lost, "연:RULE-CHAT-TALK-STATE a lost room ends the talk as lost");
}

#[test]
fn granted_and_taken_carry_the_open_talk_id() {
    let id = common::talk::talk_id(common::now_ms());
    for m in [Msg::new(Mt::Granted, None, 5, 30), Msg { q: 3, ..Msg::new(Mt::Taken, Some("a".into()), 0, 0) }] {
        let out = floor::to_mbcp(&m, "r1", None, &[], Some(&id)).expect("encodes");
        let back = oxsig::floor::decode(&oxsig::floor::encode(&out).expect("bytes")).expect("decodes");
        assert_eq!(back.talk().as_deref(), Some(id.as_str()), "연:RULE-FLR-TLV 0x20 on {:?}", m.t);
    }
    let idle = floor::to_mbcp(&Msg::new(Mt::Idle, None, 0, 0), "r1", None, &[], Some(&id)).expect("encodes");
    assert!(idle.get(oxsig::floor::tlv::TALK).is_none(), "IDLE carries no talk id");
}
