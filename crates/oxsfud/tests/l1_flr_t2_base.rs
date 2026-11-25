// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-FLR-RTP-OBSERVE · 연:RULE-FLR-TIMERS · model: claude-opus-5-5
//! 1층 발언권 시계 — 큐 승계로 발언권을 받은 사람의 `T2` 는 허가 시각이 아니라 그 사람의 첫 RTP 부터 센다: 승계 허가는 앞 화자의 첫 RTP 시각을 지우고 `T2` 를 걸지 않으며, 늦게 켠 마이크의 첫 RTP 가 들어온 때 그 시각 + `T2` 로 건다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;
use std::time::Duration;

use common::envelope::CallKind;
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::room::Room;
use oxsig::floor::timers;
use serde_json::json;

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn rx(r: &Arc<Room>, u: &str, t: Mt) {
    let m = Msg::new(t, None, 5, 0);
    floor::run(r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
}

fn clock(r: &Room) -> (u64, u64, Option<u64>, Option<u64>) {
    let c = r.exec.clock.lock().expect("clock");
    (c.grant_no, c.granted_at, c.first_rtp_at, c.timers.get(&("T2", None)).copied())
}

#[test]
fn a_succession_speaker_counts_t2_from_their_own_first_rtp() {
    let _g = serial();
    let r = create(&tag("t2b"));
    let (a, b) = (tag("t2b-a"), tag("t2b-b"));
    for u in [&a, &b] {
        floor::run(&r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
        floor::run(&r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
    }
    rx(&r, &a, Mt::Request);
    let (g1, _, _, _) = clock(&r);
    assert!(g1 >= 1, "stand check — the first speaker is granted (grant {g1})");
    floor::on_rtp(&r.id, &a, true, g1);
    let (_, _, a_first, a_t2) = clock(&r);
    assert!(a_first.is_some() && a_t2 == a_first.map(|f| f + timers::T2_MS), "stand check — the first speaker's T2 runs from their first RTP ({a_first:?} · {a_t2:?})");
    rx(&r, &b, Mt::Request);
    std::thread::sleep(Duration::from_millis(20));
    rx(&r, &a, Mt::Release);
    let (g2, granted, first, t2) = clock(&r);
    assert!(g2 == g1 + 1 && holder_is(&r, &b), "stand check — the queue hands the floor to the waiter (grant {g1} → {g2})");
    assert_eq!(first, None, "S-sym-IMPL-FLR-RTP-OBSERVE-1 the succession grant forgets the previous speaker's first RTP");
    assert_eq!(t2, None, "S-sym-IMPL-FLR-RTP-OBSERVE-1 T2 is not armed at the succession grant (grant time counts for nothing)");
    std::thread::sleep(Duration::from_millis(40));
    floor::on_rtp(&r.id, &b, true, g2);
    let (_, _, first, t2) = clock(&r);
    let first = first.expect("S-sym-IMPL-FLR-RTP-OBSERVE-1 the late microphone's first RTP is recorded");
    assert!(first >= granted + 40, "S-sym-IMPL-FLR-RTP-OBSERVE-1 the recorded first RTP is the late one, not the grant ({granted} → {first})");
    assert_eq!(t2, Some(first + timers::T2_MS), "S-sym-IMPL-FLR-RTP-OBSERVE-1 T2 runs from the successor's first RTP — first RTP + {} ms", timers::T2_MS);
}

fn holder_is(r: &Room, u: &str) -> bool {
    r.floor.lock().ok().and_then(|f| f.holder()).as_deref() == Some(u)
}
