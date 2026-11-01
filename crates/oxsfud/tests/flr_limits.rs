// author: kodeholic (powered by Claude)
// spec: v1.7 · 연:ASM-FLR-DELIVER-FIRST · 연:ASM-FLR-T1-OVER-T132 · 연:ASM-FLR-RETX-BEFORE-EXPIRE · 연:ASM-FLR-T8-OVER-T132 · 연:ASM-FLR-SPEAKER-RTP · 연:ASM-FLR-T20-OVER-RTP · 연:ASM-FLR-TIMER-EMPTY · 연:ASM-FLR-RTP-INSTANT · 연:ASM-FLR-HUMAN-SPEED · 연:ASM-FLR-STALE-GUARD · 연:RACE-FLR-LATE-ACK · 연:RACE-FLR-LATE-BCAST-IDLE · 연:RACE-FLR-LATE-CONFIRM-IDLE · 연:RACE-FLR-UNICAST-OVERTAKES-BROADCAST · 연:RACE-FLR-TAP-OVERTAKE · 연:RACE-FLR-T101-OVERTAKE · 연:RACE-FLR-LATE-RELEASE · 연:RACE-FLR-T132-PREEMPT · 연:RACE-FLR-LOST-GRANTED-PREEMPT · 연:RACE-FLR-DC-STALE · 연:RULE-FLR-SEQ · model: claude-opus-5-5
//! 모형 한계 1층 — 모형이 「사람 동작은 통이 빈 뒤에만」 가정으로 보지 않는 창에서 서버가 하는 일을 잰다(받아들인 경주의 결말 · 일련번호 규칙).

use oxsfud::flr::core::{Floor, Msg, Mt, Step, G};
use oxsig::floor::timers;

fn room(users: &[&str]) -> Floor {
    let mut f = Floor::new(10, 30);
    for u in users {
        f.ws_join(u, true, true);
        f.ws_pub_select(u);
    }
    f
}

fn request(f: &mut Floor, u: &str) -> Step {
    request_p(f, u, 5)
}

fn request_p(f: &mut Floor, u: &str, p: u32) -> Step {
    f.server_rx(u, &Msg::new(Mt::Request, None, p, 0), 0)
}

fn release(f: &mut Floor, u: &str) -> Step {
    f.server_rx(u, &Msg::new(Mt::Release, None, 0, 0), 0)
}

fn taken_seq(s: &Step, to: &str) -> u32 {
    s.out.iter().find(|o| o.to == to && o.m.t == Mt::Taken).map(|o| o.m.q).expect("TAKEN to listener")
}

#[test]
fn late_ack_of_old_taken_stops_t21_of_new_taken() {
    let mut f = room(&["a", "b", "c"]);
    let s = request(&mut f, "a");
    let old = taken_seq(&s, "b");
    release(&mut f, "a");
    let s = request(&mut f, "c");
    let new = taken_seq(&s, "b");
    assert_ne!(old, new);
    assert!(f.armed_participant("b").contains(&"T21"));
    let late = Msg { t: Mt::Ack, a: None, n: 2, d: 0, q: old };
    f.server_rx("b", &late, 0);
    assert!(!f.armed_participant("b").contains(&"T21"), "RACE-FLR-LATE-ACK: an ACK carries no number, so the late one stops T21");
    assert_eq!(f.part_name("b"), "NP_TAKEN");
}

#[test]
fn granted_ignored_by_client_does_not_block_channel_forever() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    let q = request(&mut f, "b");
    assert!(q.out.iter().any(|o| o.to == "b" && o.m.t == Mt::Qinfo));
    let s = release(&mut f, "a");
    assert!(s.out.iter().any(|o| o.to == "b" && o.m.t == Mt::Granted));
    assert_eq!(f.holder().as_deref(), Some("b"));
    let mut grants = 0;
    while f.armed_server().contains(&"T20") {
        let r = f.t20();
        grants += r.out.iter().filter(|o| o.to == "b" && o.m.t == Mt::Granted).count();
    }
    assert!(grants >= 1);
    assert!(f.armed_server().contains(&"T1"), "RACE-FLR-LATE-BCAST-IDLE: T1 is armed once T20 is spent");
    let r = f.t1();
    assert_eq!(f.g, G::Idle);
    assert_eq!(f.holder(), None);
    assert!(r.out.iter().any(|o| o.to == "a" && o.m.t == Mt::Idle), "the channel is free again");
}

#[test]
fn t21_retransmission_is_a_new_send_with_a_new_number() {
    let mut f = room(&["a", "b"]);
    let s = request(&mut f, "a");
    let first = taken_seq(&s, "b");
    let r = f.t21("b");
    let again = taken_seq(&r, "b");
    assert_eq!(again, (first + 1) % 65_536, "RULE-FLR-SEQ: every send of TAKEN raises the number");
}

#[test]
fn granted_carries_no_number() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    release(&mut f, "a");
    assert!(f.seq > 0);
    let s = request(&mut f, "b");
    let g = s.out.iter().find(|o| o.to == "b" && o.m.t == Mt::Granted).expect("GRANTED");
    assert_eq!(g.m.q, 0, "RULE-FLR-SEQ: only IDLE and TAKEN carry the number");
}

fn sent(s: &Step, to: &str, t: Mt) -> usize {
    s.out.iter().filter(|o| o.to == to && o.m.t == t).count()
}

#[test]
fn ledger_arithmetic_behind_the_timer_assumptions_holds() {
    const { assert!(timers::T1_MS > timers::T132_MS, "ASM-FLR-T1-OVER-T132") };
    const { assert!(timers::T3_MS >= timers::T8_MS * timers::C8 as u64, "ASM-FLR-RETX-BEFORE-EXPIRE (T3)") };
    const { assert!(timers::T1_MS >= timers::T20_MS * timers::C20 as u64, "ASM-FLR-RETX-BEFORE-EXPIRE (T1)") };
    const { assert!(timers::T8_MS * timers::C8 as u64 > timers::T132_MS, "ASM-FLR-T8-OVER-T132") };
    const { assert!(timers::T101_MS >= 500 && timers::T100_MS >= 500 && timers::T104_MS >= 500, "ASM-FLR-TIMER-EMPTY: the shortest timer is 0.5 s") };
}

#[test]
fn speaker_without_rtp_loses_the_floor_by_t1() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    f.rtp("a");
    assert!(f.armed_server().contains(&"T1"));
    let r = f.t1();
    assert_eq!(f.g, G::Idle, "ASM-FLR-SPEAKER-RTP: a holder whose media stops is released by T1");
    assert_eq!(sent(&r, "b", Mt::Idle), 1);
}

#[test]
fn late_successor_rtp_only_repeats_granted() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    request(&mut f, "b");
    release(&mut f, "a");
    assert!(f.armed_server().contains(&"T20"));
    let r = f.t20();
    assert_eq!(sent(&r, "b", Mt::Granted), 1, "ASM-FLR-T20-OVER-RTP: a late first RTP only costs a repeated GRANTED");
    assert_eq!(f.part_name("b"), "PERMITTED");
    f.rtp("b");
    assert!(!f.armed_server().contains(&"T20"), "the first RTP stops T20");
    assert_eq!(f.holder().as_deref(), Some("b"));
}

#[test]
fn retransmitted_request_and_release_are_idempotent() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    let again = request(&mut f, "a");
    assert_eq!(sent(&again, "a", Mt::Granted), 1, "ASM-FLR-TIMER-EMPTY: a T101 copy of REQUEST is answered by GRANTED again");
    assert_eq!(f.holder().as_deref(), Some("a"));
    release(&mut f, "a");
    let again = release(&mut f, "a");
    assert_eq!(sent(&again, "a", Mt::Idle), 1, "a T100 copy of RELEASE is confirmed by IDLE again");
    assert_eq!(f.g, G::Idle);
}

#[test]
fn rtp_after_release_draws_revoke_then_release_settles() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    release(&mut f, "a");
    let r = f.rtp("a");
    assert_eq!(sent(&r, "a", Mt::Revoke), 1, "ASM-FLR-RTP-INSTANT · RACE-FLR-DC-STALE: media after release draws REVOKE");
    assert_eq!(f.g, G::Idle);
    let r = release(&mut f, "a");
    assert_eq!(sent(&r, "a", Mt::Idle), 1, "the stale holder's release is confirmed");
    assert_eq!(f.part_name("a"), "NP_IDLE");
}

#[test]
fn request_overtaken_by_release_is_granted_and_freed_by_t1() {
    let mut f = room(&["a", "b"]);
    let r = release(&mut f, "a");
    assert_eq!(sent(&r, "a", Mt::Idle), 1);
    let r = request(&mut f, "a");
    assert_eq!(sent(&r, "a", Mt::Granted), 1, "RACE-FLR-TAP-OVERTAKE · RACE-FLR-T101-OVERTAKE: the late REQUEST is granted");
    assert_eq!(f.holder().as_deref(), Some("a"));
    let r = f.t1();
    assert_eq!(f.g, G::Idle, "ASM-FLR-HUMAN-SPEED · ASM-FLR-STALE-GUARD: the silent holder is released by T1");
    assert_eq!(sent(&r, "b", Mt::Idle), 1);
}

#[test]
fn late_release_copy_drops_the_new_wait_until_next_idle() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a");
    release(&mut f, "a");
    request(&mut f, "b");
    let q = request(&mut f, "a");
    assert_eq!(sent(&q, "a", Mt::Qinfo), 1);
    let r = release(&mut f, "a");
    assert!(f.queue.is_empty(), "RACE-FLR-LATE-RELEASE: the late RELEASE copy takes a out of the queue");
    assert_eq!(sent(&r, "a", Mt::Taken), 1);
    let r = release(&mut f, "b");
    assert_eq!(f.g, G::Idle);
    assert_eq!(sent(&r, "a", Mt::Idle), 1, "the next IDLE reaches a");
}

#[test]
fn revoke_during_t132_or_after_lost_granted_is_retried_then_t3_moves_on() {
    let mut f = room(&["a", "b", "c"]);
    request_p(&mut f, "a", 0);
    request_p(&mut f, "b", 0);
    let s = release(&mut f, "a");
    assert_eq!(sent(&s, "b", Mt::Granted), 1);
    let r = request_p(&mut f, "c", 9);
    assert_eq!(sent(&r, "b", Mt::Revoke), 1, "RACE-FLR-T132-PREEMPT · RACE-FLR-LOST-GRANTED-PREEMPT: b is revoked");
    assert_eq!(f.g, G::Prevoke);
    let mut retries = 0;
    while f.armed_participant("b").contains(&"T8") {
        retries += sent(&f.t8("b"), "b", Mt::Revoke);
    }
    assert_eq!(retries, timers::C8 as usize - 1, "REVOKE is retried until C8");
    let r = f.t3();
    assert_eq!(f.holder().as_deref(), Some("c"), "T3 moves the floor on");
    assert_eq!(sent(&r, "b", Mt::Taken), 1, "b learns the new speaker");
}
