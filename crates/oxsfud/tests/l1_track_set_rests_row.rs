// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-PUB-TRACK-SET · 정:IMPL-SUB-TABLE · 연:RULE-SUB-RESTING-HAS-PT · model: claude-opus-5-5
//! 1층 제어 층 — 전이중에서 반이중으로 돌린 트랙(쉬는 트랙)은 구독자 배정표의 그 줄을 비운다(줄은 남고 트랙만 빠진다) · 같은 kind 의 새 트랙이 그 빈 줄을 받는다(새 줄 0 — 쉬는 트랙이 줄을 쥐거나 예약하지 않는다)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use oxsig::msg::{Duplex, Kind};
use serde_json::json;

use support::*;

fn video_rows(t: &oxsfud::sub::Table) -> Vec<(String, Option<String>)> {
    t.snapshot("l1").mlines.into_iter().filter(|m| m.kind == Kind::Video).map(|m| (m.mid, m.track_id)).collect()
}

#[test]
fn resting_a_full_track_empties_its_row_and_the_next_video_takes_that_row() {
    let _g = serial();
    let r = room(&tag("ts2"));
    let v = subscriber(&tag("ts2-v"));
    enter(&v.p, &r);
    let p = subscriber(&tag("ts2-p"));
    enter(&p.p, &r);
    let (sv, _) = published(&p.p, &r.id, Kind::Video, Duplex::Full);
    list(&sv, true);
    oxsfud::ops::media::resync_peer(&v.p, false);
    assert!(holds(&v.t, &sv.track_id, &r.id), "stand check — the subscriber loads the full video");
    let before = video_rows(&v.t);
    let mid = before.iter().find(|(_, t)| t.as_deref() == Some(sv.track_id.as_str())).map(|(m, _)| m.clone()).expect("stand check — the video sits on a row");
    let res = oxsfud::ops::media::track_set(&ctx_of(&p.p), &json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "track_id": sv.track_id, "duplex": "half" }));
    assert!(res.is_ok(), "stand check — TRACK_SET duplex half is accepted: {res:?}");
    oxsfud::ops::media::resync_peer(&v.p, false);
    let rested = video_rows(&v.t);
    assert!(!holds(&v.t, &sv.track_id, &r.id), "S-sym-IMPL-PUB-TRACK-SET-2 the resting track leaves the subscriber's table");
    assert_eq!(rested.len(), before.len(), "S-sym-IMPL-PUB-TRACK-SET-2 the row stays — only its track is emptied");
    assert_eq!(rested.iter().find(|(m, _)| *m == mid).map(|(_, t)| t.clone()), Some(None), "S-sym-IMPL-PUB-TRACK-SET-2 the resting track's row is empty (one emptied row)");
    let q = subscriber(&tag("ts2-q"));
    enter(&q.p, &r);
    let (qv, _) = published(&q.p, &r.id, Kind::Video, Duplex::Full);
    list(&qv, true);
    oxsfud::ops::media::resync_peer(&v.p, false);
    let after = video_rows(&v.t);
    assert_eq!(after.len(), before.len(), "S-sym-IMPL-PUB-TRACK-SET-2 the new video takes no new row — the resting track holds no reserved row");
    assert_eq!(after.iter().find(|(m, _)| *m == mid).and_then(|(_, t)| t.clone()), Some(qv.track_id.clone()), "S-sym-IMPL-PUB-TRACK-SET-2 a new track of the same kind takes the emptied row");
    assert!(breaks(&v).is_empty(), "lists equal the table: {:?}", breaks(&v));
}
