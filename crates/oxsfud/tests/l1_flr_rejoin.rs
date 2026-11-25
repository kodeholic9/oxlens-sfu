// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FLR-MEMBER-JOIN · 연:RULE-FLR-MSG-DENY · model: claude-opus-5-5
//! 1층 발언권 — 다시 든 사람은 참가자로 다시 들어가 요청이 받아들여진다(미입장 거절 0 — 화자가 없으면 허가 · 있으면 대기) · 회수 중에 나갔던 화자가 다시 들면 옛 회수 일화가 먼저 닫힌다(context 202611/20261124b 서버 규칙 갈래).

use oxsfud::flr::core::{Floor, G, Msg, Mt, Step};

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

fn sent(s: &Step, to: &str, t: Mt) -> usize {
    s.out.iter().filter(|o| o.to == to && o.m.t == t).count()
}

fn rejoin(f: &mut Floor, u: &str) -> Step {
    f.ws_leave(u);
    let s = f.ws_join(u, true);
    f.ws_pub_select(u);
    s
}

#[test]
fn a_member_who_joins_again_is_heard_not_denied() {
    let mut f = room(&["a", "b"]);
    rejoin(&mut f, "a");
    let s = request(&mut f, "a", 5);
    assert_eq!(sent(&s, "a", Mt::Deny), 0, "S-sym-IMPL-FLR-MEMBER-JOIN-1 a re-joined member's REQUEST is not denied as not in room");
    assert_eq!((sent(&s, "a", Mt::Granted), f.holder().as_deref()), (1, Some("a")), "S-sym-IMPL-FLR-MEMBER-JOIN-1 with no speaker the re-joined member is GRANTED");
    rejoin(&mut f, "b");
    let s = request(&mut f, "b", 5);
    assert_eq!(sent(&s, "b", Mt::Deny), 0, "S-sym-IMPL-FLR-MEMBER-JOIN-1 with a speaker the re-joined member is not denied either");
    assert!(f.queue.iter().any(|w| w.u == "b"), "S-sym-IMPL-FLR-MEMBER-JOIN-1 with a speaker the re-joined member waits in the queue");
}

#[test]
fn a_speaker_who_left_during_the_revoke_closes_the_old_revoke_on_joining_again() {
    let mut f = room(&["a", "b"]);
    request(&mut f, "a", 1);
    f.rtp("a");
    request(&mut f, "b", 9);
    assert!(f.g == G::Prevoke && f.revoking.as_deref() == Some("a"), "stand check — a higher priority request revokes the speaker a (g {:?} · revoking {:?})", f.g, f.revoking);
    f.ws_leave("a");
    assert_eq!(f.revoking.as_deref(), Some("a"), "stand check — the revoke of a is still open after a left");
    let s = f.ws_join("a", true);
    assert!(s.labels.contains(&"FLR.S.ws.join_revoking"), "S-sym-IMPL-FLR-MEMBER-JOIN-1 joining again during the revoke takes the join_revoking path: {:?}", s.labels);
    assert_ne!(f.revoking.as_deref(), Some("a"), "S-sym-IMPL-FLR-MEMBER-JOIN-1 the old revoke episode of a is closed before a gets a new participant record (g {:?})", f.g);
    f.ws_pub_select("a");
    let s = request(&mut f, "a", 5);
    assert_eq!(sent(&s, "a", Mt::Deny), 0, "S-sym-IMPL-FLR-MEMBER-JOIN-1 the re-joined former speaker's REQUEST is not denied as not in room");
}
