// author: kodeholic (powered by Claude)
// spec: v1.7 · 연:RULE-FLR-MSG-RELEASE · 연:RULE-FLR-MSG-ACK · 연:RULE-FLR-MSG-QUEUE · 연:RULE-FLR-TIMERS · 연:DEV-FLR-RELEASE-ACK · model: claude-sonnet-5-5
//! 1층 발언권 서버 응답 대조 — 반환(RELEASE)은 보낸 사람의 상태로 갈려 ACK 하나 + 본인에게만 확인(TAKEN/IDLE)이고, 순번 질의(QPOS)는 Taken·회수 유예면 큐 소속과 무관하게 답하고 Idle 이면 무응답이며 타이머가 순번 통지를 만들지 않고, 서버는 ACK 에 답하지 않는다(context 202611/20261124b 클라 연동 갈래).

use oxsfud::flr::core::{Floor, Msg, Mt, Step};
use oxsig::floor;

const NOT_IN_QUEUE: u32 = 254;

fn room(users: &[&str]) -> Floor {
    let mut f = Floor::new(10, 30);
    for u in users {
        f.ws_join(u, true);
        f.ws_pub_select(u);
    }
    f
}

fn request_p(f: &mut Floor, u: &str, p: u32) -> Step {
    f.server_rx(u, &Msg::new(Mt::Request, None, p, 0), true, 0)
}

fn release(f: &mut Floor, u: &str) -> Step {
    f.server_rx(u, &Msg::new(Mt::Release, None, 0, 0), true, 0)
}

fn ask(f: &mut Floor, u: &str) -> Step {
    f.server_rx(u, &Msg::new(Mt::QposReq, None, 0, 0), true, 0)
}

fn to<'a>(s: &'a Step, who: &str, t: Mt) -> Vec<&'a Msg> {
    s.out.iter().filter(|o| o.to == who && o.m.t == t).map(|o| &o.m).collect()
}

fn only_to(s: &Step, who: &str) -> bool {
    s.out.iter().all(|o| o.to == who)
}

fn ack_of(t: u32) -> Msg {
    Msg { t: Mt::Ack, a: None, n: t, d: 0, q: 0 }
}

#[test]
fn release_by_a_waiter_or_a_listener_is_acked_once_and_confirmed_to_that_person_alone() {
    let mut f = room(&["a", "b", "c", "d"]);
    let first = request_p(&mut f, "a", 5);
    let before = to(&first, "c", Mt::Taken)[0].q;
    request_p(&mut f, "b", 5);
    assert_eq!(to(&ask(&mut f, "b"), "b", Mt::Qinfo)[0].n, 1, "stand check — b waits at position 1");

    let s = release(&mut f, "b");
    let acks = to(&s, "b", Mt::Ack);
    assert_eq!((acks.len(), acks[0].n), (1, floor::RELEASE as u32), "CP-MSG-FLOOR-RELEASE② the server ACKs a RELEASE once with the released type (12=RELEASE)");
    let taken = to(&s, "b", Mt::Taken);
    assert_eq!(taken.len(), 1, "CP-MSG-FLOOR-RELEASE④ a waiter that releases is told who speaks now — one TAKEN");
    assert_eq!(taken[0].q, before + 1, "CP-MSG-FLOOR-RELEASE④ the confirmation carries the room sequence +1");
    assert!(only_to(&s, "b"), "CP-MSG-FLOOR-RELEASE④ the confirmation is unicast — nothing goes to the others");
    assert_eq!(to(&ask(&mut f, "b"), "b", Mt::Qinfo)[0].n, NOT_IN_QUEUE, "CP-MSG-FLOOR-RELEASE④ the waiter left the queue");

    let s = release(&mut f, "c");
    let acks = to(&s, "c", Mt::Ack);
    assert_eq!((acks.len(), acks[0].n), (1, floor::RELEASE as u32), "CP-MSG-FLOOR-RELEASE② a listener's RELEASE is ACKed once too");
    assert_eq!(to(&s, "c", Mt::Taken).len(), 1, "CP-MSG-FLOOR-RELEASE④ a listener is told who speaks now — one TAKEN");
    assert!(only_to(&s, "c"), "CP-MSG-FLOOR-RELEASE④ unicast to the listener only");
}

#[test]
fn release_by_a_listener_of_an_idle_room_is_acked_and_told_idle_alone() {
    let mut f = room(&["a", "b"]);
    let s = release(&mut f, "b");
    assert_eq!(to(&s, "b", Mt::Ack).len(), 1, "CP-MSG-FLOOR-RELEASE② the ACK comes in an idle room as well");
    assert_eq!(to(&s, "b", Mt::Idle).len(), 1, "CP-MSG-FLOOR-RELEASE④ an idle room confirms with IDLE to that person");
    assert!(only_to(&s, "b"), "CP-MSG-FLOOR-RELEASE④ IDLE confirmation is unicast");
    assert_eq!(f.holder(), None, "stand check — the release changed nothing in the room");
}

#[test]
fn release_by_the_speaker_ends_the_floor_with_an_ack_and_no_idle_to_the_speaker() {
    let mut f = room(&["a", "b"]);
    request_p(&mut f, "a", 5);
    let s = release(&mut f, "a");
    assert_eq!(to(&s, "a", Mt::Ack).len(), 1, "CP-MSG-FLOOR-RELEASE② the speaker's RELEASE is ACKed once");
    assert_eq!(to(&s, "a", Mt::Idle).len(), 0, "CP-MSG-FLOOR-RELEASE④ the speaker is not sent IDLE — the ACK is its confirmation");
    assert_eq!(f.holder(), None, "CP-MSG-FLOOR-RELEASE④ the floor went idle");
    assert_eq!(to(&s, "b", Mt::Idle).len(), 1, "stand check — the listener learned the floor is idle");
}

#[test]
fn a_position_query_is_answered_while_taken_or_revoking_whatever_the_queue_and_not_while_idle() {
    let mut f = room(&["a", "b", "c", "e"]);
    let idle = ask(&mut f, "c");
    assert!(idle.out.is_empty(), "CP-MSG-FLOOR-QUEUE-POS-REQUEST② an idle room does not answer");

    request_p(&mut f, "a", 5);
    request_p(&mut f, "b", 5);
    assert_eq!(to(&ask(&mut f, "b"), "b", Mt::Qinfo)[0].n, 1, "stand check — a queued member gets its position");
    let outside = ask(&mut f, "c");
    let q = to(&outside, "c", Mt::Qinfo);
    assert_eq!((q.len(), q[0].n), (1, NOT_IN_QUEUE), "CP-MSG-FLOOR-QUEUE-POS-REQUEST② C-proc-RULE-FLR-PROC-QUEUED-T104④ taken: a member outside the queue is answered with 254");

    let s = request_p(&mut f, "e", 200);
    assert_eq!(to(&s, "a", Mt::Revoke).len(), 1, "stand check — a higher priority pushed the speaker into the revoke grace");
    let during = ask(&mut f, "c");
    let q = to(&during, "c", Mt::Qinfo);
    assert_eq!((q.len(), q[0].n), (1, NOT_IN_QUEUE), "CP-MSG-FLOOR-QUEUE-POS-REQUEST② C-proc-RULE-FLR-PROC-QUEUED-T104④ during the revoke grace the query is still answered — a waiter whose T104 ran out can always ask");
}

#[test]
fn no_server_timer_produces_a_position_notice() {
    let build = || {
        let mut f = room(&["a", "b", "c"]);
        request_p(&mut f, "a", 5);
        request_p(&mut f, "b", 5);
        f
    };
    let probe = build();
    let timers = probe.armed_server();
    assert!(!timers.is_empty(), "stand check — the taken room has server timers armed");
    let mut fired = 0;
    for name in timers {
        let mut f = build();
        let s = match name {
            "T1" => f.t1(),
            "T2" => f.t2(),
            "T3" => f.t3(),
            "T7" => f.t7(),
            "T20" => f.t20(),
            other => panic!("unhandled server timer {other}"),
        };
        fired += s.out.len();
        assert!(!s.out.iter().any(|o| o.m.t == Mt::Qinfo), "CP-MSG-FLOOR-QUEUE-POS-REQUEST③ timer {name} fired and sent no QUEUE_INFO — only the app's query makes one");
    }
    for u in ["a", "b", "c"] {
        for name in build().armed_participant(u) {
            fired += 1;
            let mut f = build();
            let s = match name {
                "T8" => f.t8(u),
                "T9" => f.t9(u),
                "T21" => f.t21(u),
                other => panic!("unhandled participant timer {other}"),
            };
            assert!(!s.out.iter().any(|o| o.m.t == Mt::Qinfo), "CP-MSG-FLOOR-QUEUE-POS-REQUEST③ participant timer {name} of {u} sent no QUEUE_INFO");
        }
    }
    assert!(fired > 0, "stand check — the timers did something when fired (the zero above is not an empty run)");
}

#[test]
fn the_server_never_answers_an_ack_in_any_participant_state() {
    let mut f = room(&["a", "b", "c", "d", "e"]);
    request_p(&mut f, "a", 5);
    request_p(&mut f, "b", 5);
    let mut seen = Vec::new();
    for u in ["a", "b", "c", "d"] {
        seen.push((u, f.part_name(u)));
    }
    request_p(&mut f, "e", 200);
    seen.push(("a", f.part_name("a")));
    let mut names: Vec<&str> = seen.iter().map(|x| x.1).collect();
    names.sort();
    names.dedup();
    assert_eq!(names, ["NP_TAKEN", "PERMITTED", "PREVOKE"], "stand check — the states covered by this room");
    let mut idle = room(&["x"]);
    assert_eq!(idle.part_name("x"), "NP_IDLE", "stand check — the fourth state, an idle room");
    for t in [floor::GRANTED as u32, floor::TAKEN as u32, floor::RELEASE as u32] {
        assert!(idle.server_rx("x", &ack_of(t), true, 0).out.is_empty(), "CP-MSG-FLOOR-ACK② an ACK for type {t} in NP_IDLE is not answered");
    }
    for u in ["a", "b", "c", "d", "e"] {
        for t in [floor::GRANTED as u32, floor::TAKEN as u32, floor::RELEASE as u32] {
            let s = f.server_rx(u, &ack_of(t), true, 0);
            assert!(s.out.is_empty(), "CP-MSG-FLOOR-ACK② an ACK for type {t} from {u} in {} is not answered", f.part_name(u));
        }
    }
    let probe = release(&mut f, "d");
    assert!(!probe.out.is_empty(), "stand check — the same member is answered when it sends something that needs an answer");
}

#[test]
fn a_floor_refusal_for_a_missing_bit_is_a_dc_deny_102_and_no_floor_code_path_builds_2006() {
    let mut f = Floor::new(10, 30);
    f.ws_join("b", false);
    f.ws_pub_select("b");
    let s = request_p(&mut f, "b", 255);
    let deny: Vec<u32> = to(&s, "b", Mt::Deny).iter().map(|m| m.n).collect();
    assert_eq!(deny, vec![floor::deny::NO_PERMISSION as u32], "CP-CODE-NOT-AUTHORIZED③ a member without the floor bit is refused with DENY 102 on the floor path");
    let floor_srcs = [include_str!("../src/flr/core.rs"), include_str!("../src/flr/talk.rs"), include_str!("../src/ops/floor.rs")];
    for (i, src) in floor_srcs.iter().enumerate() {
        assert!(!src.contains("NotAuthorized"), "CP-CODE-NOT-AUTHORIZED③ floor source {i} never builds code 2006 (NotAuthorized)");
    }
    assert!(include_str!("../src/ops/media.rs").contains("Code::NotAuthorized"), "stand check — the scan sees the real producer: PUBLISH_TRACKS builds 2006");
    assert!(include_str!("../src/flr/core.rs").contains("deny::NO_PERMISSION"), "stand check — the floor source carries the 102 path");
}

#[test]
fn a_member_without_the_floor_bit_is_denied_102_every_time_it_presses() {
    let mut f = Floor::new(10, 30);
    f.ws_join("a", true);
    f.ws_pub_select("a");
    f.ws_join("b", false);
    f.ws_pub_select("b");
    request_p(&mut f, "a", 5);
    let mut causes = Vec::new();
    for p in [5u32, 7, 255, 5, 255] {
        let s = request_p(&mut f, "b", p);
        causes.extend(to(&s, "b", Mt::Deny).iter().map(|m| m.n));
        assert_eq!(s.out.len(), 1, "C-proc-RULE-FLR-PROC-DENY② pressing again at priority {p} sends exactly one thing — the DENY");
    }
    assert_eq!(causes, vec![floor::deny::NO_PERMISSION as u32; 5], "C-proc-RULE-FLR-PROC-DENY② the 102 refusal is the same every time it is pressed again");
    assert_eq!(f.holder().as_deref(), Some("a"), "stand check — the speaker was never disturbed");
}
