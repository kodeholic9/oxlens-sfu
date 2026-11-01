// author: kodeholic (powered by Claude)
// spec: v1.7 · 연:ASM-FLR-DELIVER-FIRST · 연:RACE-FLR-LATE-ACK · 연:RACE-FLR-LATE-BCAST-IDLE · 연:RACE-FLR-UNICAST-OVERTAKES-BROADCAST · 연:RULE-FLR-SEQ · model: claude-opus-5-5
//! 모형 한계 1층 — 모형이 「사람 동작은 통이 빈 뒤에만」 가정으로 보지 않는 창에서 서버가 하는 일을 잰다(받아들인 경주의 결말 · 일련번호 규칙).

use oxsfud::flr::core::{Floor, Msg, Mt, G};

fn room(users: &[&str]) -> Floor {
    let mut f = Floor::new(10, 30);
    for u in users {
        f.ws_join(u, true, true);
        f.ws_pub_select(u);
    }
    f
}

fn request(f: &mut Floor, u: &str) -> oxsfud::flr::core::Step {
    f.server_rx(u, &Msg::new(Mt::Request, None, 5, 0), 0)
}

fn release(f: &mut Floor, u: &str) -> oxsfud::flr::core::Step {
    f.server_rx(u, &Msg::new(Mt::Release, None, 0, 0), 0)
}

fn taken_seq(s: &oxsfud::flr::core::Step, to: &str) -> u32 {
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
