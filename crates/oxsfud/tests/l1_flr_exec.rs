// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-MEDIA-BIND · model: claude-opus-5-5
//! 1층 발언권 집행 판단 — 효과를 적용하고(해제보다 늦게 닿은 결합은 버린다) 적용된 결합의 영상에만 PLI burst 를 청한다. 판단은 `plan_exec` 하나이고 집행(`execute`)은 그 목록을 내보내기만 한다(context 202611/20261124b 본게임 차례 1).

mod support;

use oxsfud::flr::bind::Effect;
use oxsfud::link::{Role, Transport};
use oxsfud::ops::floor::{Exec, plan_exec};
use oxsfud::r#pub::PubTrack;
use oxsig::msg::{Duplex, Kind};

use support::*;

fn video_of(user: &str, room: &str) -> std::sync::Arc<oxsfud::r#pub::PubStream> {
    let st = stream_as(user, room, Kind::Video, Duplex::Half);
    let tr = Transport::new(sfu().next_id(), user, &st.session, Role::Publish, 0, false);
    let t = PubTrack::new(st.clone(), st.ssrc, None, None, &tr);
    st.tracks.lock().expect("tracks").push(t);
    st
}

fn bind(user: &str, g: u64, v: &std::sync::Arc<oxsfud::r#pub::PubStream>) -> Exec {
    Exec::Bind { effect: Effect::Bind { user: user.into(), g }, user: user.into(), g, videos: vec![v.clone()], video_ok: true }
}

#[test]
fn a_bind_that_lands_after_its_release_is_dropped_and_asks_no_pli_burst() {
    let _g = serial();
    let fresh = room(&tag("exec-fresh"));
    let va = video_of(&tag("exec-a"), &fresh.id);
    let plan = plan_exec(&fresh, vec![bind("a", 0, &va)]);
    assert_eq!((plan.applied.len(), plan.burst.len()), (1, 1), "positive control — a fresh bind applies and asks a burst for its video");

    let late = room(&tag("exec-late"));
    let vb = video_of(&tag("exec-b"), &late.id);
    let rel = plan_exec(&late, vec![Exec::Release { effect: Effect::Release { g: 0 } }]);
    assert!(rel.burst.is_empty(), "a release asks no burst");
    let plan = plan_exec(&late, vec![bind("b", 0, &vb)]);
    assert!(plan.burst.is_empty(), "S-sym-IMPL-FLR-MEDIA-BIND-4② the dropped bind asks no PLI burst");
    assert!(plan.applied.is_empty(), "S-sym-IMPL-FLR-MEDIA-BIND-4① the bind that lands after its own release is dropped — the returned speaker is not revived");
}
