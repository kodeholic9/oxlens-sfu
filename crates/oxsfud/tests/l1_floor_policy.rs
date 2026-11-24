// author: kodeholic (powered by Claude)
// spec: v1.6 · 정책:floor · 정:RULE-FLR · model: claude-opus-5-5
//! 1층 발언권 정책 값 — 발언권 상태기(`Floor::new(queue_max, t2_stop_talking_secs)`)가 정책 값을 그대로 따르는지 값을 바꿔 잰다(context 202611/20261124b 운영 · 정책 갈래).

use oxsfud::flr::core::{Floor, Msg, Mt, Step};
use oxsig::floor::deny;

fn room(qmax: usize, t2: u32, users: &[&str]) -> Floor {
    let mut f = Floor::new(qmax, t2);
    for u in users {
        f.ws_join(u, true);
        f.ws_pub_select(u);
    }
    f
}

fn request(f: &mut Floor, u: &str) -> Step {
    f.server_rx(u, &Msg::new(Mt::Request, None, 5, 0), true, 0)
}

fn denied_with(s: &Step, to: &str) -> Option<u32> {
    s.out.iter().find(|o| o.to == to && o.m.t == Mt::Deny).map(|o| o.m.n)
}

#[test]
fn the_queue_takes_exactly_queue_max_then_denies_with_seven() {
    for qmax in [1usize, 2, 5] {
        let users: Vec<String> = (0..qmax + 3).map(|i| format!("u{i}")).collect();
        let refs: Vec<&str> = users.iter().map(String::as_str).collect();
        let mut f = room(qmax, 30, &refs);
        request(&mut f, "u0");
        for i in 1..=qmax {
            let s = request(&mut f, &users[i]);
            assert!(denied_with(&s, &users[i]).is_none(), "positive control — queue_max={qmax}: waiter {i} is taken into the queue");
            assert!(s.out.iter().any(|o| o.to == users[i] && o.m.t == Mt::Qinfo), "positive control — waiter {i} gets its position");
        }
        let over = request(&mut f, &users[qmax + 1]);
        assert_eq!(denied_with(&over, &users[qmax + 1]), Some(u32::from(deny::QUEUE_FULL)), "PL-KEY-FLOOR-QUEUE-MAX — queue_max={qmax}: the request after {qmax} waiters is DENY(7)");
        assert_eq!(f.queue.len(), qmax, "PL-KEY-FLOOR-QUEUE-MAX — queue_max={qmax}: the queue stays at the limit");
    }
}

#[test]
fn the_granted_message_carries_the_stop_talking_value() {
    for t2 in [5u32, 7, 600] {
        let mut f = room(10, t2, &["a", "b"]);
        let s = request(&mut f, "a");
        let granted = s.out.iter().find(|o| o.to == "a" && o.m.t == Mt::Granted).expect("GRANTED to the speaker");
        assert_eq!(granted.m.d, t2, "t2_stop_talking_secs={t2}: the speaker is told this duration in GRANTED");
    }
}
