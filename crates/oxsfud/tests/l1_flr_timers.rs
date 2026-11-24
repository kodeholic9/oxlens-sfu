// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-FLR-TIMERS · 연:FLR.wire · model: claude-opus-5-5
//! 1층 서버 발언권 타이머 — 값은 원장(oxsig timers) · 시작 = 그 타이머가 걸린 집합에 든다 · 정지 = 빠진다 · 만료 = 그 입구의 출력. 걸린 집합을 실제 시계로 거는 것은 `ops/floor.rs` `rearm`(context 202611/20261124b 본게임 묶음 3).

use oxsfud::flr::core::{Floor, G, Msg, Mt, Step};
use oxsig::floor::{deny, revoke, timers};

fn room(users: &[&str]) -> Floor {
    let mut f = Floor::new(10, 30);
    for u in users {
        f.ws_join(u, true);
        f.ws_pub_select(u);
    }
    f
}

fn request(f: &mut Floor, u: &str, p: u32) -> Step {
    f.server_rx(u, &Msg::new(Mt::Request, None, p, 0), true, 0)
}

fn release(f: &mut Floor, u: &str) -> Step {
    f.server_rx(u, &Msg::new(Mt::Release, None, 0, 0), true, 0)
}

fn sent(s: &Step, to: &str, t: Mt) -> Vec<Msg> {
    s.out.iter().filter(|o| o.to == to && o.m.t == t).map(|o| o.m.clone()).collect()
}

fn on(f: &Floor, t: &str) -> bool {
    f.armed_server().contains(&t)
}

fn on_p(f: &Floor, u: &str, t: &str) -> bool {
    f.armed_participant(u).contains(&t)
}

#[test]
fn t1_runs_while_held_and_its_expiry_enters_idle_without_telling_the_speaker() {
    assert_eq!(timers::T1_MS, 4000, "CP-TIMER-T1① 4 seconds");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    assert!(on(&f, "T1"), "CP-TIMER-T1① armed while the floor is taken");
    f.rtp("a");
    let r = f.t1();
    assert_eq!(f.g, G::Idle, "CP-TIMER-T1③ expiry enters idle");
    assert_eq!((sent(&r, "b", Mt::Idle).len(), sent(&r, "a", Mt::Idle).len()), (1, 0), "CP-TIMER-T1③ IDLE to the room — none to the speaker");
    let mut f = room(&["a", "b", "c"]);
    request(&mut f, "a", 5);
    request(&mut f, "b", 5);
    let r = f.t1();
    assert_eq!((f.holder().as_deref(), sent(&r, "b", Mt::Granted).len()), (Some("b"), 1), "CP-TIMER-T1③ expiry with a queue hands the floor to the next speaker");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 1);
    f.rtp("a");
    request(&mut f, "b", 9);
    assert!(on(&f, "T3") && on(&f, "T1"), "positive control — a preemption puts the floor in revoke grace (T3) with T1 still running");
    f.t1();
    assert!(!on(&f, "T3"), "CP-TIMER-T1④ CP-TIMER-T3② T1 expiry during the grace stops T3");
}

#[test]
fn t2_starts_at_the_first_rtp_and_its_expiry_revokes_with_cause_two() {
    assert_eq!(timers::T2_MS, 30000, "CP-TIMER-T2① 30 seconds");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    assert!(!on(&f, "T2"), "CP-TIMER-T2① not at the grant");
    f.rtp("a");
    assert!(on(&f, "T2"), "CP-TIMER-T2① starts at the first RTP");
    let r = f.t2();
    assert_eq!(sent(&r, "a", Mt::Revoke).iter().map(|m| m.n).collect::<Vec<_>>(), vec![revoke::TOO_LONG as u32], "CP-TIMER-T2② expiry revokes with cause 2");
}

#[test]
fn t3_is_the_revoke_grace_and_its_expiry_enters_idle() {
    assert_eq!(timers::T3_MS, 3000, "CP-TIMER-T3① 3 seconds");
    let grace = || {
        let mut f = room(&["a", "b", "c"]);
        request(&mut f, "a", 1);
        f.rtp("a");
        request(&mut f, "b", 9);
        f
    };
    let mut f = grace();
    assert!(on(&f, "T3"), "CP-TIMER-T3① REVOKE starts the grace");
    release(&mut f, "a");
    assert!(!on(&f, "T3"), "CP-TIMER-T3② the speaker's RELEASE stops it");
    let mut f = grace();
    let r = f.rtp("a");
    assert_eq!((sent(&r, "a", Mt::Revoke).len(), f.holder().as_deref()), (0, Some("a")), "CP-TIMER-T3③ the speaker's RTP during the grace is still carried — no violation revoke · still the holder");
    let r = request(&mut f, "c", 9);
    assert!(sent(&r, "c", Mt::Granted).is_empty() && on(&f, "T3"), "CP-TIMER-T3④ a preempting request during the grace is not granted — the grace goes on");
    f.t3();
    assert!(!on(&f, "T3") && f.holder().as_deref() != Some("a"), "CP-TIMER-T3③ expiry ends the grace — the revoked speaker no longer holds");
    let mut f = grace();
    let r = f.ws_leave("a");
    assert!(r.labels.contains(&"FLR.S.G.PREVOKE.client_release") && f.g == G::Prevoke && on(&f, "T3"), "CP-TIMER-T3④ the revoked speaker leaving during the grace changes nothing — the grace runs on");
}

#[test]
fn t7_repeats_idle_ten_times_with_a_new_number_each() {
    assert_eq!((timers::T7_MS, timers::C7), (1000, 10), "CP-TIMER-T7① 1 second · 10 times");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    let r = release(&mut f, "a");
    let mut seqs = sent(&r, "b", Mt::Idle).iter().map(|m| m.q).collect::<Vec<_>>();
    let mut n = 0;
    while on(&f, "T7") {
        let r = f.t7();
        seqs.extend(sent(&r, "b", Mt::Idle).iter().map(|m| m.q));
        n += 1;
        assert!(n <= 20, "T7 never stopped");
    }
    assert_eq!(seqs.len(), timers::C7 as usize, "CP-TIMER-T7① IDLE ten times in all (the first and nine repeats) then it stops");
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "CP-TIMER-T7① each repeat carries seq + 1 {seqs:?}");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    release(&mut f, "a");
    assert!(on(&f, "T7"), "positive control — IDLE repeat armed");
    request(&mut f, "b", 5);
    assert!(!on(&f, "T7"), "CP-TIMER-T7② a state change stops it");
}

#[test]
fn t8_repeats_revoke_three_times_and_stops_on_release_or_grace_end() {
    assert_eq!((timers::T8_MS, timers::C8), (1000, 3), "CP-TIMER-T8① 1 second · C8 3 times");
    let grace = || {
        let mut f = room(&["a", "b"]);
        request(&mut f, "a", 1);
        f.rtp("a");
        request(&mut f, "b", 9);
        f
    };
    let mut f = grace();
    assert!(on_p(&f, "a", "T8"), "CP-TIMER-T8① REVOKE starts T8 for the revoked speaker");
    release(&mut f, "a");
    assert!(!on_p(&f, "a", "T8"), "CP-TIMER-T8② the speaker's RELEASE stops it");
    let mut f = grace();
    f.t3();
    assert!(!on_p(&f, "a", "T8"), "CP-TIMER-T8② the grace end (T3) stops it");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    release(&mut f, "a");
    f.rtp("a");
    assert!(on_p(&f, "a", "T8"), "CP-TIMER-T8③ a send violation REVOKE(3) also starts T8");
    let mut resent = 0;
    for _ in 0..timers::C8 {
        resent += sent(&f.t8("a"), "a", Mt::Revoke).len();
    }
    let last = f.t8("a");
    assert_eq!((resent, sent(&last, "a", Mt::Revoke).len()), (timers::C8 as usize - 1, 0), "CP-TIMER-T8③ REVOKE again up to C8 in all · none after C8 is spent");
    assert!(!on_p(&f, "a", "T8"), "CP-TIMER-T8③ spent — T8 stops");
}

#[test]
fn t20_repeats_the_succession_grant_until_rtp_release_or_t1() {
    assert_eq!((timers::T20_MS, timers::C20), (1000, 3), "CP-TIMER-T20① 1 second · 3 times");
    let succ = || {
        let mut f = room(&["a", "b"]);
        request(&mut f, "a", 5);
        request(&mut f, "b", 5);
        release(&mut f, "a");
        f
    };
    let mut f = succ();
    assert!(on(&f, "T20"), "CP-TIMER-T20① the succession GRANTED starts T20");
    let mut repeats = 0;
    while on(&f, "T20") {
        repeats += sent(&f.t20(), "b", Mt::Granted).len();
        assert!(repeats <= 10, "T20 never stopped");
    }
    assert_eq!(repeats + 1, timers::C20 as usize, "CP-TIMER-T20① the succession GRANTED three times in all (the first and two repeats)");
    for stop in ["rtp", "release", "t1"] {
        let mut f = succ();
        match stop {
            "rtp" => {
                f.rtp("b");
            }
            "release" => {
                release(&mut f, "b");
            }
            _ => {
                f.t1();
            }
        }
        assert!(!on(&f, "T20"), "CP-TIMER-T20② {stop} stops T20");
    }
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 1);
    f.rtp("a");
    request(&mut f, "b", 9);
    release(&mut f, "a");
    assert!(on(&f, "T20") && f.holder().as_deref() == Some("b"), "CP-TIMER-T20③ the preemptor's succession GRANTED also starts T20");
}

#[test]
fn t9_denies_a_retry_inside_three_seconds_and_grants_after() {
    assert_eq!(timers::T9_MS, 3000, "CP-TIMER-T9 3 seconds");
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 5);
    f.rtp("a");
    f.t2();
    release(&mut f, "a");
    assert!(on_p(&f, "a", "T9"), "positive control — a cause 2 revoke arms T9 for the speaker");
    let r = request(&mut f, "a", 5);
    assert_eq!(sent(&r, "a", Mt::Deny).iter().map(|m| m.n).collect::<Vec<_>>(), vec![deny::RETRY_AFTER as u32], "CP-TIMER-T9① a retry inside T9 is denied with cause 4");
    f.t9("a");
    let r = request(&mut f, "a", 5);
    assert_eq!(sent(&r, "a", Mt::Granted).len(), 1, "CP-TIMER-T9② after T9 the request is granted");
}
