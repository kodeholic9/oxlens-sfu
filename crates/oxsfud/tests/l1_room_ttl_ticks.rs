// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-LIFE · model: claude-opus-5-5
//! 1층 — 떠나간 방의 TTL 은 쓸기 횟수로 센다. 빈 것을 처음 본 쓸기 뒤 ⌈TTL ÷ 쓸기 주기⌉ 번째 쓸기에서 거둔다 — 시각으로 견주면 TTL 이 주기의 배수일 때 쓸기가 몇 ms 일찍 오면 한 주기 더 밀린다(실측 14.55초 · TTL 5초 · 쓸기 5초 · context 202611/20261110e D1). 거두는 것은 TTL 보다 일찍이 아니다.

use oxsfud::flr::core::Floor;
use oxsfud::room::{Member, Room, RoomInit};
use oxsig::msg::{MediaCodecs, Permission};
use std::sync::Arc;

const SWEEP: u64 = 5_000;

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

fn member(user: &str) -> Member {
    Member { user: user.into(), role: None, select: false, participant_type: 0, hidden: false, metadata: None, permission: Permission::ALL }
}

fn emptied(departure: u32) -> (Arc<Room>, u64) {
    let r = room(3_600, departure);
    r.join(member("a"), false, None);
    r.remove("a");
    let t0 = r.created_at + 1_000_000;
    assert!(!r.ttl_due(t0, SWEEP), "S-sym-IMPL-ROOM-LIFE-1 the sweep that first sees the room empty only records it");
    (r, t0)
}

#[test]
fn departure_ttl_equal_to_sweep_period_expires_on_the_next_sweep_even_if_it_comes_early() {
    let (r, t0) = emptied(5);
    assert!(r.ttl_due(t0 + SWEEP - 1, SWEEP), "S-sym-IMPL-ROOM-LIFE-2 one sweep later (4,999ms — the interval came 1ms early) is due · not one more period");
}

#[test]
fn departure_ttl_is_rounded_up_to_whole_sweeps_and_never_expires_early() {
    let (r, t0) = emptied(7);
    assert!(!r.ttl_due(t0 + SWEEP, SWEEP), "S-sym-IMPL-ROOM-LIFE-2 5s < 7s TTL — not yet");
    assert!(r.ttl_due(t0 + 2 * SWEEP, SWEEP), "S-sym-IMPL-ROOM-LIFE-2 2 sweeps = 10s ≥ 7s");
}

#[test]
fn rejoin_resets_the_sweep_count() {
    let (r, t0) = emptied(10);
    assert!(!r.ttl_due(t0 + SWEEP, SWEEP));
    r.join(member("a"), false, None);
    r.remove("a");
    assert!(!r.ttl_due(t0 + 2 * SWEEP, SWEEP), "S-sym-IMPL-ROOM-LIFE-1 first sweep after the second departure only records");
    assert!(!r.ttl_due(t0 + 3 * SWEEP, SWEEP), "S-sym-IMPL-ROOM-LIFE-1 count restarted — 1 sweep < 10s");
    assert!(r.ttl_due(t0 + 4 * SWEEP, SWEEP));
}

#[test]
fn unused_ttl_still_compares_with_the_creation_time() {
    let r = room(7, 5);
    let c = r.created_at;
    assert!(!r.ttl_due(c + 6_999, SWEEP), "S-sym-IMPL-ROOM-LIFE-1 unused is measured from created_at — not yet");
    assert!(r.ttl_due(c + 7_000, SWEEP));
}
