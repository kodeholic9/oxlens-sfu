// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-SUB-TABLE · 연:RULE-TRK-SLOT-LIFE · model: claude-opus-5-5
//! 1층 제어 층 — 퇴장 정리 일곱 단계의 ⑥(트랙 정리): 개인 퇴장은 그 사람의 전이중 트랙만 빼고 슬롯 트랙은 건드리지 않는다 — 남은 구독자의 배정표에 슬롯 줄(audio · video)이 그대로 남는다 · 비운 줄은 표와 줄 목록이 함께 비고 재입장한 사람의 영상이 같은 kind 의 빈 줄에 다시 실린다(m-line 수 그대로)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use oxsfud::gc::Why;
use oxsig::msg::{Duplex, Kind};

use support::*;

#[test]
fn a_personal_leave_keeps_the_slot_tracks_for_the_rest() {
    let _g = serial();
    let x = room(&tag("g72-x"));
    let rest = subscriber(&tag("g72-r"));
    enter(&rest.p, &x);
    let spk = subscriber(&tag("g72-s"));
    enter(&spk.p, &x);
    let _half = published(&spk.p, &x.id, Kind::Video, Duplex::Half);
    let _full = published(&spk.p, &x.id, Kind::Audio, Duplex::Full);
    oxsfud::ops::media::resync_peer(&rest.p, false);
    let slots = [oxsfud::trk::slot_track_id(&x.id, Kind::Audio), oxsfud::trk::slot_track_id(&x.id, Kind::Video)];
    for s in &slots {
        assert!(holds(&rest.t, s, &x.id), "stand check — the remaining subscriber holds the slot track {s} before the leave");
    }
    assert!(oxsfud::gc::leave(&spk.p, &x.id, Why::Leave, false), "stand check — the speaker leaves");
    for s in &slots {
        assert!(holds(&rest.t, s, &x.id), "S-sym-RULE-GC-LEAVE-SEVEN-2① a personal leave keeps the slot track {s} in the remaining subscriber's table");
    }
    assert!(breaks(&rest).is_empty(), "lists equal the table after the leave: {:?}", breaks(&rest));
}

#[test]
fn a_rejoin_reloads_the_video_on_the_emptied_row() {
    let _g = serial();
    let x = room(&tag("g73-x"));
    let sub = subscriber(&tag("g73-s"));
    enter(&sub.p, &x);
    let user = tag("g73-u");
    let u = subscriber(&user);
    enter(&u.p, &x);
    let (v1, _) = published(&u.p, &x.id, Kind::Video, Duplex::Full);
    list(&v1, true);
    oxsfud::ops::media::resync_peer(&sub.p, false);
    assert!(holds(&sub.t, &v1.track_id, &x.id), "stand check — the subscriber loads the video before the leave");
    let rows0 = sub.t.rows().len();
    assert!(oxsfud::gc::leave(&u.p, &x.id, Why::Leave, false), "stand check — the publisher leaves");
    assert!(!holds(&sub.t, &v1.track_id, &x.id), "S-sym-IMPL-SUB-TABLE-2① the left track is off the table");
    assert!(breaks(&sub).is_empty(), "S-sym-IMPL-SUB-TABLE-2① S-sym-RULE-GC-LEAVE-SEVEN-3① after emptying the row the table and the lists agree: {:?}", breaks(&sub));
    let back = subscriber(&user);
    enter(&back.p, &x);
    let (v2, _) = published(&back.p, &x.id, Kind::Video, Duplex::Full);
    list(&v2, true);
    oxsfud::ops::media::resync_peer(&sub.p, false);
    assert!(holds(&sub.t, &v2.track_id, &x.id), "S-sym-IMPL-SUB-TABLE-2② S-sym-RULE-GC-LEAVE-SEVEN-3① the rejoined publisher's video is loaded again");
    assert_eq!(sub.t.rows().len(), rows0, "S-sym-IMPL-SUB-TABLE-2① the emptied video row is reused — no extra m-line");
    assert!(breaks(&sub).is_empty(), "S-sym-IMPL-SUB-TABLE-2① lists equal the table after the rejoin: {:?}", breaks(&sub));
}
