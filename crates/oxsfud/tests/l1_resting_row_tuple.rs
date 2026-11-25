// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-TRK-FROM-PUB · 연:RULE-SUB-RESTING-HAS-PT · model: claude-opus-5-5
//! 1층 제어 층 — 배정표 줄은 실린 트랙이 없어도 m-line 그대로다: 화자 없는 슬롯 줄 · 쉬게 된 개인 트랙의 줄 모두 `pt` · `codec` · `fmtp` 를 싣고, 비운 줄은 실려 있을 때의 튜플 · 다운링크 SSRC · `stream_id` 를 그대로 갖는다(context 202611/20261124b 서버 규칙 갈래 · ㉣ 메인 결재 20261125).

mod support;

use oxsig::msg::{Duplex, Kind, MlineEntry};
use serde_json::json;

use support::*;

fn rows(t: &oxsfud::sub::Table) -> Vec<MlineEntry> {
    t.snapshot("l1").mlines
}

fn tuple(m: &MlineEntry) -> (u8, String, Option<String>, u32, String) {
    (m.pt, m.codec.clone(), m.fmtp.clone(), m.ssrc, m.stream_id.clone())
}

#[test]
fn every_row_carries_its_tuple_even_when_no_track_sits_on_it() {
    let _g = serial();
    let r = room(&tag("rht"));
    let v = subscriber(&tag("rht-v"));
    enter(&v.p, &r);
    let p = subscriber(&tag("rht-p"));
    enter(&p.p, &r);
    let (sv, _) = published(&p.p, &r.id, Kind::Video, Duplex::Full);
    list(&sv, true);
    oxsfud::ops::media::resync_peer(&v.p, false);
    let loaded = rows(&v.t);
    let row = loaded.iter().find(|m| m.track_id.as_deref() == Some(sv.track_id.as_str())).cloned().expect("stand check — the full video sits on a row");
    let slots = [oxsfud::trk::slot_track_id(&r.id, Kind::Audio), oxsfud::trk::slot_track_id(&r.id, Kind::Video)];
    let slot_rows: Vec<&MlineEntry> = loaded.iter().filter(|m| m.track_id.as_ref().is_some_and(|t| slots.contains(t))).collect();
    assert_eq!(slot_rows.len(), 2, "stand check — the room's two slot rows are loaded (no speaker yet)");
    for m in &slot_rows {
        assert!(m.pt != 0 && !m.codec.is_empty() && m.fmtp.is_some(), "S-sym-IMPL-TRK-FROM-PUB-1 a slot row with no speaker still carries pt · codec · fmtp: {m:?}");
    }
    assert!(row.pt != 0 && !row.codec.is_empty() && row.fmtp.is_some(), "stand check — the loaded row carries its tuple: {row:?}");
    let res = oxsfud::ops::media::track_set(&ctx_of(&p.p), &json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "track_id": sv.track_id, "duplex": "half" }));
    assert!(res.is_ok(), "stand check — the video rests (duplex half): {res:?}");
    oxsfud::ops::media::resync_peer(&v.p, false);
    let rested = rows(&v.t).into_iter().find(|m| m.mid == row.mid).expect("S-sym-IMPL-TRK-FROM-PUB-1 the resting track's row stays in the table");
    assert_eq!(rested.track_id, None, "stand check — the row is emptied");
    assert_eq!(tuple(&rested), tuple(&row), "S-sym-IMPL-TRK-FROM-PUB-1 the emptied row keeps pt · codec · fmtp · downlink SSRC · stream_id as it had them");
    assert!(rows(&v.t).iter().all(|m| m.pt != 0 && !m.codec.is_empty() && m.fmtp.is_some()), "S-sym-IMPL-TRK-FROM-PUB-1 every row of the table carries pt · codec · fmtp: {:?}", rows(&v.t));
}
