// author: kodeholic (powered by Claude)
//! 규격 고정값 — 연§3·§8. 값의 정본은 연동규격서다. 여기는 그 좌표를 단 상수 하나씩.
//! `T2`(Stop talking)만 정책 키(`[floor] t2_stop_talking_secs`)라 여기 없다.

// 연§3-1 끊는 조건
pub const IDLE_TIMEOUT_MS: u64 = 30_000;
pub const RESPONSE_TIMEOUT_MS: u64 = 30_000;
pub const QUEUE_OVERFLOW: usize = 1_000;
// 연§3-2 흐름 제어
pub const WINDOW_DEFAULT: usize = 1;
pub const WINDOW_MAX: usize = 10;
// 연§6-1 BIND 전 10초
pub const BIND_TIMEOUT_MS: u64 = 10_000;
// 연§8-2 재접속 백오프 — 대기 44초, 지터는 3번째부터 0~1000ms
pub const BACKOFF_MS: [u64; 10] = [0, 300, 1_200, 2_700, 4_800, 7_000, 7_000, 7_000, 7_000, 7_000];
pub const BACKOFF_JITTER_FROM_INDEX: usize = 2;
pub const BACKOFF_JITTER_MAX_MS: u64 = 1_000;
/// 연§7-0-1 4 — 요청 재시도는 사다리 둘째~넷째 칸, 최대 3회.
pub const REQUEST_RETRY_MS: [u64; 3] = [300, 1_200, 2_700];
// 연§8-4 클라 (3GPP TS 24.380 이름 그대로)
pub const T101_MS: u64 = 500;
pub const C101: u8 = 3;
pub const T100_MS: u64 = 500;
pub const C100: u8 = 3;
pub const T104_MS: u64 = 500;
pub const C104: u8 = 3;
pub const T132_MS: u64 = 2_000;
pub const T_QUEUEPOS_MS: u64 = 30_000;
// 연§8-4 서버
pub const T1_MS: u64 = 4_000;
pub const T3_MS: u64 = 3_000;
pub const T7_MS: u64 = 1_000;
pub const C7: u8 = 10;
pub const T8_MS: u64 = 1_000;
pub const T20_MS: u64 = 1_000;
pub const C20: u8 = 3;
pub const T9_MS: u64 = 3_000;
// 연§8-3 미디어
pub const READY_KEYFRAME_SELF_RELEASE_MS: u64 = 5_000;
pub const SYNC_REQUIRED_COOLDOWN_MS: u64 = 30_000;
// 연§6-2 한계
pub const MAX_ROOMS_PER_SFU: usize = 100;
pub const MAX_TRACKS_PER_REQUEST: usize = 8;
pub const MAX_ACTIVE_TRACKS: usize = 16;
pub const MAX_RECV_MID: usize = 255;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sum_stays_inside_resume_window_math() {
        let wait: u64 = BACKOFF_MS.iter().sum();
        assert_eq!(wait, 44_000);
        assert_eq!(&BACKOFF_MS[1..4], &REQUEST_RETRY_MS);
        assert!(T101_MS * u64::from(C101) < 6_000);
    }
}
