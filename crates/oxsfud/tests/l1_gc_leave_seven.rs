// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-SUB-TABLE · 연:RULE-TRK-SLOT-LIFE · model: claude-opus-5-5
//! 1층 제어 층 — 퇴장 정리 일곱 단계의 ⑥(트랙 정리): 개인 퇴장은 그 사람의 전이중 트랙만 빼고 슬롯 트랙은 건드리지 않는다 — 남은 구독자의 배정표에 슬롯 줄(audio · video)이 그대로 남는다(context 202611/20261124b 서버 규칙 갈래).

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
        assert!(holds(&rest.t, s, &x.id), "S-sym-RULE-GC-LEAVE-SEVEN-2 a personal leave keeps the slot track {s} in the remaining subscriber's table");
    }
    assert!(breaks(&rest).is_empty(), "lists equal the table after the leave: {:?}", breaks(&rest));
}
