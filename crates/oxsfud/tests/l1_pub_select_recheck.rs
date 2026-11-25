// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! 1층 제어 층 — `pub_select` 비트 재검사: 발행 중인 사람이 발언 방을 옮길 때 새 방의 권한 비트가 그 발행을 막으면 `2006` 으로 거절하고 `details.denied` 에 막힌 비트 이름을 싣는다 · 발언 방은 그대로다(context 202611/20261124b 서버 규칙 갈래 · SV-CODE-2006 소비처 넷 가운데 하나).

mod support;

use oxsig::msg::{Duplex, Kind, Permission};

use support::*;

#[test]
fn a_pub_select_into_a_room_without_the_bit_is_refused_2006_with_the_denied_bits() {
    let _g = serial();
    let home = room(&tag("ps-home"));
    let next = room(&tag("ps-next"));
    let u = subscriber(&tag("ps-u"));
    enter(&u.p, &home);
    enter(&u.p, &next);
    let _v = published(&u.p, &home.id, Kind::Video, Duplex::Half);
    let _a = published(&u.p, &home.id, Kind::Audio, Duplex::Half);
    oxsfud::ops::media::move_pub(&u.p, Some(&home), None).expect("stand check — the home room is the speaking room");
    assert_eq!(oxsfud::aff::pub_room(&u.p).as_deref(), Some(home.id.as_str()), "stand check — speaking in the home room");
    let down = Permission { publish_video: false, ..Permission::ALL };
    assert!(next.set_permission(&u.p.user, down).is_some(), "stand check — the next room lowers the video bit");
    let f = oxsfud::ops::media::move_pub(&u.p, Some(&next), None).expect_err("SV-CODE-2006 pub_select into a room without the video bit is refused");
    assert_eq!(f.code, 2006, "SV-CODE-2006 the pub_select bit recheck refuses with NOT_AUTHORIZED: {f:?}");
    assert_eq!(f.details, Some(serde_json::json!({ "denied": ["publish_video"] })), "SV-CODE-2006 the refusal names the blocked bits in details.denied: {f:?}");
    assert_eq!(oxsfud::aff::pub_room(&u.p).as_deref(), Some(home.id.as_str()), "SV-CODE-2006 a refused pub_select leaves the speaking room as it was");
    oxsfud::ops::media::move_pub(&u.p, Some(&home), None).expect("stand check — staying home is still fine");
}
