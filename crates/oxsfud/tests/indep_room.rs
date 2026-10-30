// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-PERMISSION · model: claude-opus-5-5
//! 맹검 독립 1층 — 수명 판정은 등록 기준(투명도 사람) · 권한 갱신은 멱등 · 오버라이드 표는 방이 들고 퇴장 · 재입장을 넘어 산다.

use oxsfud::flr::core::Floor;
use oxsfud::room::{Member, PERMANENT, Room, RoomInit};
use oxsig::msg::{MediaCodecs, Permission};
use std::sync::Arc;

fn room(unused: u32, departure: u32) -> Arc<Room> {
    Room::new(RoomInit {
        id: "r1".into(),
        name: "r1".into(),
        capacity: 10,
        unused_ttl: unused,
        departure_ttl: departure,
        media_codecs: MediaCodecs { audio: Some("opus".into()), video: Some("H264".into()) },
        epoch: "ep-1".into(),
        floor: Floor::new(10, 30),
    })
}

fn member(user: &str, hidden: bool) -> Member {
    Member { user: user.into(), role: None, select: false, participant_type: if hidden { 2 } else { 0 }, hidden, metadata: None, permission: Permission::ALL }
}

const DOWN: Permission = Permission { publish_audio: false, publish_video: true, publish_screen: true, floor_request: false };

#[test]
fn hidden_member_keeps_room_alive() {
    let r = room(0, 0);
    r.join(member("rec-bot", true), false, None);
    assert_eq!(r.user_count(), 0, "S-sym-IMPL-ROOM-LIFE-1 hidden member is not visible in user_count");
    assert_eq!(r.registered(), 1, "S-sym-IMPL-ROOM-LIFE-1 hidden member is registered");
    let base = r.created_at;
    for dt in [1u64, 5_000, 60_000, 10_000_000, 10_000_001] {
        assert!(!r.ttl_due(base + dt), "S-sym-IMPL-ROOM-LIFE-1 room with only a hidden recorder must not expire · +{dt}ms");
    }
    r.remove("rec-bot");
    let t1 = base + 20_000_000;
    let first = r.ttl_due(t1);
    let second = r.ttl_due(t1 + 1);
    assert!(!first, "S-sym-IMPL-ROOM-LIFE-1 first sweep after the last leave only records the empty time");
    assert!(second, "S-sym-IMPL-ROOM-LIFE-1 control: departure TTL 0 expires on the following sweep once really empty");
}

#[test]
fn permanent_room_never_expires() {
    let r = room(PERMANENT, PERMANENT);
    let base = r.created_at;
    for dt in [0u64, 1, 1 << 40] {
        assert!(!r.ttl_due(base + dt), "S-sym-IMPL-ROOM-LIFE-1 permanent marker never expires (unused) · +{dt}");
    }
    r.join(member("a", false), false, None);
    r.remove("a");
    for dt in [10u64, 20, 1 << 41] {
        assert!(!r.ttl_due(base + dt), "S-sym-IMPL-ROOM-LIFE-1 permanent marker never expires (departure) · +{dt}");
    }
}

#[test]
fn same_permission_twice_is_a_no_op() {
    let r = room(PERMANENT, PERMANENT);
    r.join(member("a", false), false, None);
    r.join(member("b", false), false, None);
    let s0 = r.version().seq;
    let first = r.set_permission("a", DOWN);
    assert!(first.is_some(), "S-sym-IMPL-ROOM-PERMISSION-2 member permission update accepted");
    let s1 = r.version().seq;
    let again = r.set_permission("a", DOWN);
    let s2 = r.version().seq;
    assert_eq!(s2, s1, "S-sym-IMPL-ROOM-PERMISSION-2 re-applying the same bits does not bump seq ({s0} → {s1} → {s2})");
    if let Some((old, new, _)) = again {
        assert_eq!(old, new, "S-sym-IMPL-ROOM-PERMISSION-2 re-applying the same bits reports no change");
    }
    assert_eq!(r.member("a").map(|m| m.permission), Some(DOWN), "S-sym-IMPL-ROOM-PERMISSION-2 bits stay applied");
}

#[test]
fn override_outlives_membership() {
    let r = room(PERMANENT, PERMANENT);
    r.join(member("a", false), false, Some(Permission::ALL));
    assert!(r.set_permission("a", DOWN).is_some(), "S-sym-IMPL-ROOM-PERMISSION-3 bits lowered for a member");
    r.remove("a");
    assert!(!r.has("a"), "S-sym-IMPL-ROOM-PERMISSION-3 left the room");
    r.join(member("a", false), false, Some(Permission::ALL));
    assert_eq!(r.member("a").map(|m| m.permission), Some(DOWN), "S-sym-IMPL-ROOM-PERMISSION-3 leaving and re-joining does not lift the lowered bits");
    assert_eq!(r.permission_for("a", Some(Permission::ALL)), DOWN, "S-sym-IMPL-ROOM-PERMISSION-3 the room's override wins over the token value");
    assert!(r.set_permission("a", Permission::ALL).is_some(), "S-sym-IMPL-ROOM-PERMISSION-3 bits raised back to default");
    assert!(!r.st.lock().map(|s| s.overrides.contains_key("a")).unwrap_or(true), "S-sym-IMPL-ROOM-PERMISSION-3 default bits are not stored as an override");
}
