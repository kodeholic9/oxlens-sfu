// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-PERMISSION · 정:IMPL-ROOM-STALL · model: claude-opus-5-5
//! 독립 1층 시험(B′ 맹검) — 방 수명(등록 기준 · TTL 둘 · 영구 표지) · 파생값(명단에서) · 권한 오버라이드 표 · 정체 쿨다운(방, 사람).

use oxsfud::flr::Floor;
use oxsfud::fwd::hot::{RoomHot, Slot};
use oxsfud::room::{permission_bits_changed, Member, Room, Sweep, PERMANENT, T_STALL_MS};
use oxsfud::trk::Item;
use oxsig::types::{Kind, MediaCodecs, MemberInfo, Permission};
use std::sync::Arc;

fn room(unused: u32, departure: u32) -> Room<Item> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let hot = Arc::new(RoomHot { id: Arc::from("r"), audio: Slot::new(Kind::Audio, "r"), video: Slot::new(Kind::Video, "r"), audio_codec: Arc::from("opus"), video_codec: Arc::from("H264"), floor_tx: tx });
    Room::new("r", "r", 10, &MediaCodecs::default(), unused, departure, "ep1", hot, Floor::new(8, 30), 1000)
}

fn member(u: &str, hidden: bool, ptype: u8) -> Member {
    Member { info: MemberInfo { user_id: u.into(), role: None, select: false, participant_type: ptype, metadata: None, permission: None }, hidden, session: format!("s-{u}") }
}

fn perm(a: bool, v: bool, s: bool, f: bool) -> Permission {
    Permission { publish_audio: a, publish_video: v, publish_screen: s, floor_request: f }
}

#[test]
fn derived_values_come_from_roster() {
    let mut r = room(PERMANENT, PERMANENT);
    r.members.insert("rec".into(), member("rec", true, 1));
    assert!(r.rec(), "S-sym-IMPL-ROOM-DERIVED-2 투명 녹화 봇만 있어도 rec = true");
    assert_eq!(r.user_count(), 0, "S-sym-IMPL-ROOM-DERIVED-2 user_count = 0(기준이 다르다)");
    assert_eq!(r.registered(), 1, "S-sym-IMPL-ROOM-LIFE-1 등록 수는 투명도 센다");
    r.members.insert("a".into(), member("a", false, 0));
    assert_eq!(r.user_count(), 1, "S-sym-IMPL-ROOM-DERIVED-2 보이는 사람만 user_count");
    r.members.remove("rec");
    assert!(!r.rec(), "S-sym-IMPL-ROOM-DERIVED-1 녹화 봇 퇴장 → rec false(플래그 잔존 0)");
}

#[test]
fn hidden_only_room_survives_ttl() {
    let mut r = room(1, 1);
    r.members.insert("rec".into(), member("rec", true, 1));
    r.ever_joined = true;
    for t in [5_000u64, 10_000, 60_000, 600_000] {
        assert_ne!(r.sweep(t), Sweep::Expire, "S-sym-IMPL-ROOM-LIFE-1 투명 봇만 남은 방 + TTL 경과 → 생존(등록 기준) t={t}");
    }
}

#[test]
fn unused_ttl_counts_from_creation_and_departure_marks_first() {
    let mut r = room(10, 2);
    assert_eq!(r.sweep(5_000), Sweep::Keep, "S-sym-IMPL-ROOM-LIFE-1 unused 10s 전 생존");
    assert_eq!(r.sweep(10_999), Sweep::Keep, "S-sym-IMPL-ROOM-LIFE-1 생성 1s · unused 10s — 10.999s 생존");
    assert_eq!(r.sweep(11_000), Sweep::Expire, "S-sym-IMPL-ROOM-LIFE-1 아무도 안 들어옴 — 생성 시각부터 ≥ ttl 이면 만료");
    let mut r = room(10, 2);
    r.members.insert("a".into(), member("a", false, 0));
    r.ever_joined = true;
    r.empty_since = 0;
    assert_eq!(r.sweep(1_000), Sweep::Keep, "S-sym-IMPL-ROOM-LIFE-1 사람 있으면 생존");
    r.members.remove("a");
    assert_eq!(r.sweep(20_000), Sweep::MarkEmpty, "S-sym-IMPL-ROOM-LIFE-1 방금 비었다 — 그 tick 엔 만료 판정 0");
    assert_eq!(r.sweep(21_000), Sweep::Keep, "S-sym-IMPL-ROOM-LIFE-1 departure 2s 전");
    assert_eq!(r.sweep(22_000), Sweep::Expire, "S-sym-IMPL-ROOM-LIFE-1 departure ≥ ttl 만료");
    let mut r = room(PERMANENT, PERMANENT);
    assert_ne!(r.sweep(u32::MAX as u64 * 2000), Sweep::Expire, "S-sym-IMPL-ROOM-LIFE-1 영구 표지는 거두지 않는다");
    let mut r = room(0, 0);
    assert_eq!(r.sweep(5_000), Sweep::Expire, "S-sym-IMPL-ROOM-LIFE-1 0초 TTL 은 다음 sweep 에 거둔다");
}

#[test]
fn version_seq_bumps_and_keeps_epoch() {
    let mut r = room(PERMANENT, PERMANENT);
    let v0 = r.version();
    let v1 = r.bump();
    assert_eq!((v1.epoch.as_str(), v1.seq), ("ep1", v0.seq + 1), "S-sym-IMPL-ROOM-VERSION-ISSUE-2 seq +1 · epoch 그대로");
    assert_eq!(r.version(), v1, "S-sym-IMPL-ROOM-VERSION-ISSUE-2 판번호 조회 = 마지막 발급");
}

#[test]
fn override_table_wins_over_token_and_all_true_clears() {
    let mut r = room(PERMANENT, PERMANENT);
    let token = perm(true, true, true, true);
    assert_eq!(r.permission_of("a", token), token, "S-sym-IMPL-ROOM-PERMISSION-3 표에 없으면 토큰값");
    let down = perm(false, true, true, false);
    r.set_override("a", down);
    assert_eq!(r.permission_of("a", token), down, "S-sym-IMPL-ROOM-PERMISSION-3 재입장 때 오버라이드 값(토큰값 0)");
    r.set_override("a", perm(true, true, true, true));
    assert!(!r.overrides.contains_key("a"), "S-sym-IMPL-ROOM-PERMISSION-3 넷 다 true 면 표 항목 삭제");
    assert_eq!(r.permission_of("a", perm(false, false, false, false)), perm(false, false, false, false), "S-sym-IMPL-ROOM-PERMISSION-3 삭제 뒤는 토큰값");
}

#[test]
fn same_permission_is_idempotent() {
    let p = perm(true, false, true, true);
    assert!(permission_bits_changed(&p, &p).is_empty(), "S-sym-IMPL-ROOM-PERMISSION-2 같은 값이면 바뀐 비트 0(seq 0 · 통지 0)");
    let q = perm(false, false, true, true);
    assert_eq!(permission_bits_changed(&p, &q), vec!["publish_audio"], "S-sym-IMPL-ROOM-PERMISSION-1 바뀐 비트만");
}

#[test]
fn stall_cooldown_per_room_and_person() {
    let mut r = room(PERMANENT, PERMANENT);
    assert!(r.stall_allowed("a", 1_000), "S-sym-IMPL-ROOM-STALL-1 첫 정체 통지");
    assert!(!r.stall_allowed("a", 1_000 + T_STALL_MS - 1), "S-sym-IMPL-ROOM-STALL-1 쿨다운 안 재통지 0");
    assert!(r.stall_allowed("b", 1_001), "S-sym-IMPL-ROOM-STALL-1 쿨다운은 (방, 사람)마다 — 다른 사람은 막지 않는다");
    assert!(r.stall_allowed("a", 1_000 + T_STALL_MS), "S-sym-IMPL-ROOM-STALL-1 쿨다운 뒤 다시");
}
