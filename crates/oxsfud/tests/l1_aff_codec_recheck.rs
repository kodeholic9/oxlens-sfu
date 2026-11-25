// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · model: claude-opus-5-5
//! 1층 제어 층 — 발언 방 재검사: 든 트랙의 코덱이 새 방과 다르면 `pub_select` 를 `1006` 으로 거절하고 아무것도 옮기지 않는다(그 방 트랙 목록에 안 들어간다) · `ROOM_JOIN{select:true}` 의 묵시 `pub_select` 가 코덱 · 권한 비트에 걸리면 입장은 되고 `pub_room` 은 그 방이 아니다 — 맞으면 그 방이다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use common::envelope::Claims;
use oxsfud::flr::core::Floor;
use oxsfud::room::{PERMANENT, Room, RoomInit};
use oxsig::msg::{Duplex, Kind, MediaCodecs, Permission};
use serde_json::{Value, json};

use support::*;

fn vp8_room(id: &str) -> Arc<Room> {
    let s = sfu();
    let (r, fresh) = s.rooms.insert(Room::new(RoomInit {
        id: id.into(),
        name: id.into(),
        capacity: 1_000,
        unused_ttl: PERMANENT,
        departure_ttl: PERMANENT,
        media_codecs: MediaCodecs { audio: None, video: Some("VP8".into()) },
        epoch: s.epoch.clone(),
        floor: Floor::new(10, 30),
        record: false,
    }));
    assert!(fresh, "stand check — room {id} is new");
    r
}

fn mine(r: &Room, user: &str) -> usize {
    oxsfud::ops::rooms::tracks_of(r).iter().filter(|t| t.user_id.as_deref() == Some(user)).count()
}

fn pub_room_of(res: &Result<Value, oxsig::Failure>) -> Option<String> {
    res.as_ref().ok().and_then(|v| v["affiliation"]["pub_room"].as_str().map(String::from))
}

#[test]
fn a_pub_select_into_a_room_of_another_codec_is_refused_1006_and_moves_nothing() {
    let _g = serial();
    let home = room(&tag("cr-home"));
    let vp8 = vp8_room(&tag("cr-vp8"));
    let u = subscriber(&tag("cr-u"));
    enter(&u.p, &home);
    enter(&u.p, &vp8);
    let (v, _) = published(&u.p, &home.id, Kind::Video, Duplex::Full);
    list(&v, true);
    oxsfud::ops::media::move_pub(&u.p, Some(&home), None).expect("stand check — the home room is the speaking room");
    assert_eq!(mine(&home, &u.p.user), 1, "stand check — the H264 video is listed in the home room");
    let f = oxsfud::ops::media::move_pub(&u.p, Some(&vp8), None).expect_err("S-sym-IMPL-AFF-PUB-ROOM-1 CP-OP-AFFILIATION⑦ an H264 track cannot speak into a VP8 room — the whole move fails");
    assert_eq!(f.code, 1006, "S-sym-IMPL-AFF-PUB-ROOM-1 CP-OP-AFFILIATION⑦ the codec recheck refuses with CODEC_MISMATCH: {f:?}");
    assert_eq!(oxsfud::aff::pub_room(&u.p).as_deref(), Some(home.id.as_str()), "S-sym-IMPL-AFF-PUB-ROOM-1 CP-OP-AFFILIATION⑦ the refusal moves nothing — the speaking room stays");
    assert_eq!(mine(&vp8, &u.p.user), 0, "S-sym-IMPL-AFF-PUB-ROOM-1 the H264 track never lands in the VP8 room (no packets of another codec flow there)");
    assert_eq!(mine(&home, &u.p.user), 1, "S-sym-IMPL-AFF-PUB-ROOM-1 the track stays listed in the home room");
}

#[test]
fn a_join_with_select_into_a_room_that_refuses_the_tracks_joins_without_the_speaking_room() {
    let _g = serial();
    let home = room(&tag("cm-home"));
    let vp8 = vp8_room(&tag("cm-vp8"));
    let same = room(&tag("cm-same"));
    let u = subscriber(&tag("cm-u"));
    enter(&u.p, &home);
    let (v, _) = published(&u.p, &home.id, Kind::Video, Duplex::Full);
    list(&v, true);
    oxsfud::ops::media::move_pub(&u.p, Some(&home), None).expect("stand check — speaking in the home room");
    oxsfud::ops::media::move_pub(&u.p, None, None).expect("stand check — pub_deselect");
    assert_eq!(oxsfud::aff::pub_room(&u.p), None, "stand check — no speaking room after pub_deselect");
    let res = oxsfud::ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": vp8.id, "select": true }));
    assert!(res.is_ok(), "S-sym-IMPL-AFF-PUB-MOVE-1① the join itself succeeds even when the implicit pub_select is refused: {res:?}");
    assert!(vp8.has(&u.p.user), "S-sym-IMPL-AFF-PUB-MOVE-1① the user is in the VP8 room roster");
    assert_ne!(pub_room_of(&res).as_deref(), Some(vp8.id.as_str()), "S-sym-IMPL-AFF-PUB-MOVE-1② the response affiliation.pub_room is not the room whose codec refused the tracks");
    assert_eq!(mine(&vp8, &u.p.user), 0, "S-sym-IMPL-AFF-PUB-MOVE-1③ the tracks do not follow into the VP8 room — no TRACK_EVENT add there");
    let mut c = ctx_of(&u.p);
    c.claims = Some(Claims { participant_type: 0, hidden: false, metadata: None, permission: Some(Permission { publish_video: false, ..Permission::ALL }) });
    let bits = room(&tag("cm-bits"));
    let res = oxsfud::ops::rooms::join(&c, &json!({ "room_id": bits.id, "select": true }));
    assert!(res.is_ok() && bits.has(&u.p.user), "S-sym-IMPL-AFF-PUB-MOVE-1① the join succeeds when the room's bits block the video: {res:?}");
    assert_ne!(pub_room_of(&res).as_deref(), Some(bits.id.as_str()), "S-sym-IMPL-AFF-PUB-MOVE-1② a blocked bit keeps the speaking room off that room");
    assert_eq!(mine(&bits, &u.p.user), 0, "S-sym-IMPL-AFF-PUB-MOVE-1③ the tracks do not follow into the room that blocks them");
    let res = oxsfud::ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": same.id, "select": true }));
    assert_eq!(pub_room_of(&res).as_deref(), Some(same.id.as_str()), "S-sym-IMPL-AFF-PUB-MOVE-1② control — codec and bits match, so pub_room is that room: {res:?}");
    assert_eq!(mine(&same, &u.p.user), 1, "S-sym-IMPL-AFF-PUB-MOVE-1③ control — the full-duplex video follows into the matching room");
}
