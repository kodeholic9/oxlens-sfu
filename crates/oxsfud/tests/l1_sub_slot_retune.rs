// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-TABLE · 연:RULE-SUB-FILL · 연:RULE-ROOM-BUDGET-SPLIT · model: claude-opus-5-5
//! 1층 구독 표 — 슬롯 줄도 채우는 순서 셋(①같은 kind · 같은 튜플의 빈 줄 ②새 줄(슬롯 몫 안) ③몫이 찼을 때만 다른 튜플의 빈 슬롯 줄의 튜플을 방 코덱으로 바꾼다): H264 방 20 을 든 채 하나 나가고 VP8 방에 들면 `4005` 없이 들고 줄은 40 그대로 · 몫이 남으면 새 줄이 먼저다(context 202611/20261124b 이슈 I3).

mod support;

use std::sync::Arc;

use common::envelope::CallKind;
use oxsfud::ops;
use oxsfud::room::Room;
use oxsig::msg::{Kind, MlineEntry};
use serde_json::{Value, json};

use support::*;

fn create(id: &str, video: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id, "media_codecs": { "video": video } });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn join(u: &Sub, r: &Room) -> Result<Value, oxsig::Failure> {
    ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": r.id }))
}

fn leave(u: &Sub, r: &Room) {
    assert!(ops::rooms::leave(&ctx_of(&u.p), &json!({ "room_id": r.id })).is_ok(), "stand check — leave {}", r.id);
}

fn lines(u: &Sub) -> Vec<MlineEntry> {
    u.t.snapshot("l1").mlines
}

fn video_of(u: &Sub, room: &str) -> Option<MlineEntry> {
    lines(u).into_iter().find(|m| m.kind == Kind::Video && m.room_id.as_deref() == Some(room))
}

#[test]
fn a_vp8_room_after_twenty_h264_rooms_less_one_retunes_the_free_slot_row() {
    let _g = serial();
    assert_eq!(sfu().policy.media.slot_mid_reserve, 40, "stand check — the policy default slot share is 40");
    let u = subscriber(&tag("sret-u"));
    let rooms: Vec<Arc<Room>> = (0..20).map(|i| create(&tag(&format!("sret-h{i}")), "H264")).collect();
    for r in &rooms {
        assert!(join(&u, r).is_ok(), "stand check — H264 room {} joins inside the slot share", r.id);
    }
    assert_eq!(lines(&u).len(), 40, "stand check — twenty slot rooms load forty slot rows");
    let freed = video_of(&u, &rooms[7].id).expect("stand check — the room to leave has a video slot row");
    leave(&u, &rooms[7]);
    let arev = u.t.arev();
    let vp8 = create(&tag("sret-v"), "VP8");
    let got = join(&u, &vp8);
    assert!(got.is_ok(), "정:IMPL-SUB-TABLE I3 with the slot share full and a free H264 slot row, a VP8 room joins without 4005: {got:?}");
    let all = lines(&u);
    assert_eq!(all.len(), 40, "정:IMPL-SUB-TABLE I3 no new row past the share — the free slot rows are reused (40 stays 40)");
    let row = video_of(&u, &vp8.id).expect("정:IMPL-SUB-TABLE I3 the VP8 room's video is loaded on a slot row");
    assert_eq!(row.mid, freed.mid, "정:IMPL-SUB-TABLE I3 the VP8 video takes the freed slot row (mid {}), got mid {}", freed.mid, row.mid);
    assert!(row.codec.eq_ignore_ascii_case("VP8") && row.pt != freed.pt, "정:IMPL-SUB-TABLE I3 the freed row's tuple is retuned to the room codec VP8 with another PT: {row:?} was {freed:?}");
    assert!(u.t.arev() > arev, "정:IMPL-SUB-TABLE I3 the retune is a table change (arev rises)");
    let h264 = all.iter().filter(|m| m.kind == Kind::Video && m.codec.eq_ignore_ascii_case("H264")).count();
    assert_eq!(h264, 19, "정:IMPL-SUB-TABLE I3 the other nineteen H264 video slot rows are untouched");
}

#[test]
fn with_share_left_a_new_slot_row_comes_before_retuning_a_free_one() {
    let _g = serial();
    let u = subscriber_slots(&tag("sret-n"), &tag("sid-sret-n"), 10);
    let a = create(&tag("sret-na"), "H264");
    let b = create(&tag("sret-nb"), "H264");
    assert!(join(&u, &a).is_ok() && join(&u, &b).is_ok(), "stand check — two H264 rooms join");
    let freed = video_of(&u, &b.id).expect("stand check — the room to leave has a video slot row");
    leave(&u, &b);
    let vp8 = create(&tag("sret-nv"), "VP8");
    assert!(join(&u, &vp8).is_ok(), "stand check — the VP8 room joins with share left");
    let all = lines(&u);
    let row = video_of(&u, &vp8.id).expect("stand check — the VP8 video is loaded");
    assert_ne!(row.mid, freed.mid, "정:IMPL-SUB-TABLE ② before ③ — with share left the VP8 video gets a new row, not the freed H264 row: {row:?}");
    let kept = all.iter().find(|m| m.mid == freed.mid).expect("stand check — the freed row stays in the table");
    assert!(kept.codec.eq_ignore_ascii_case("H264") && kept.track_id.is_none(), "정:IMPL-SUB-TABLE ② before ③ — the freed H264 row is left empty and unretuned: {kept:?}");
    assert_eq!(all.len(), 5, "정:IMPL-SUB-TABLE ① same tuple first — the freed audio row is reused (same opus tuple) and only the video row is new (4 + 1)");
}

#[test]
fn with_the_share_full_and_no_free_slot_row_the_join_is_still_mid_limit() {
    let _g = serial();
    let u = subscriber_slots(&tag("sret-f"), &tag("sid-sret-f"), 4);
    let a = create(&tag("sret-fa"), "H264");
    let b = create(&tag("sret-fb"), "H264");
    assert!(join(&u, &a).is_ok() && join(&u, &b).is_ok(), "stand check — two H264 rooms fill a share of four");
    let vp8 = create(&tag("sret-fv"), "VP8");
    let f = join(&u, &vp8).expect_err("정:IMPL-SUB-TABLE ③ needs a free row — none is free, the join is refused");
    assert_eq!(f.code, 4005, "정:IMPL-SUB-TABLE ③ with no free slot row the share still refuses with 4005: {f:?}");
    assert_eq!(f.details, Some(json!({ "limit": 4, "current": 4 })), "정:IMPL-SUB-TABLE ③ details are slot row counts: {f:?}");
    assert!(lines(&u).iter().all(|m| m.codec.eq_ignore_ascii_case("opus") || m.codec.eq_ignore_ascii_case("H264")), "정:IMPL-SUB-TABLE ③ a refused join retunes nothing");
}
