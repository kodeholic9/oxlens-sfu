// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-STALL · 정:IMPL-FLR-RTP-OBSERVE · model: claude-opus-5-5
//! 1층 — 정체 감지가 슬롯 줄 하나를 볼 때 화자 · 마지막 RTP 는 일화 하나에서 읽고, 판정 쪽 시계의 허가 시각은 그 일화의 허가 번호와 같을 때만 쓴다(판정이 집행보다 큐 한 단계 앞선 사이에는 표본을 건너뛴다 · context 202611/20261112d R9 · RTP 관측 귀속 R8 과 같은 꼴).

use oxsfud::ops::timers::slot_start;

#[test]
fn the_slot_window_starts_at_the_grant_of_the_same_episode() {
    assert_eq!(slot_start(7, Some((7, 5_000)), 3_000), Some(5_000), "정:IMPL-ROOM-STALL the window starts at the grant when the clock and the episode agree");
    assert_eq!(slot_start(7, Some((7, 2_000)), 3_000), Some(3_000), "정:IMPL-ROOM-STALL and never before the subscriber was ready");
}

#[test]
fn a_clock_ahead_of_the_slot_episode_skips_the_sample() {
    assert_eq!(slot_start(7, Some((8, 9_000)), 3_000), None, "정:IMPL-ROOM-STALL S-sym-IMPL-ROOM-STALL-4 the decision is one grant ahead of the execution — the speaker and the grant time belong to different grants, so this sample is skipped");
    assert_eq!(slot_start(7, None, 3_000), None, "정:IMPL-ROOM-STALL no floor clock no slot sample");
}
