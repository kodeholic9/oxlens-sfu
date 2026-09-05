// author: kodeholic (powered by Claude)
//! 정§11-1 상향 — 물리 트랙마다의 결손 추적과 재전송 요구.
//!
//! ★`BTreeMap` 이다. Generic NACK 은 PID + BLP(17개 묶음)로 나가므로 seq 정렬이 필요하다 —
//! 해시맵으로 바꾸면 묶기가 깨져 한 결손에 한 패킷씩 나간다.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// 정§11-1 — 이보다 큰 점프는 재전송으로 못 메운다. 키프레임(PLI) 영역이다.
pub const MAX_GAP: u16 = 128;
/// 재전송 요구 간격. ★RTT 실측을 반영하지 않는 고정값이다.
pub const RETRY_MS: u64 = 200;
/// 이만큼 지난 결손은 포기한다 — 늦게 와 봐야 jitter buffer 를 지났다.
pub const EXPIRE_MS: u64 = 1_000;
/// 추적 상한. 넘으면 등재를 멈춘다(메모리와 NACK 폭풍을 같이 막는다).
pub const MAX_TRACKED: usize = 128;

#[derive(Debug, Clone, Copy)]
struct Missing {
    since_ms: u64,
    /// ★`None` 이 "아직 안 물어봤다" 다. 0 을 표식으로 쓰면 시각 0 과 겹친다.
    asked_at_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct Inner {
    last_seq: Option<u16>,
    missing: BTreeMap<u16, Missing>,
}

/// 한 물리 트랙의 결손 장부.
#[derive(Debug, Default)]
pub struct GapTracker {
    inner: Mutex<Inner>,
}

impl GapTracker {
    /// ingress 한 패킷을 봤다. ★핫패스라 판정만 하고 아무것도 보내지 않는다.
    pub fn observe(&self, seq: u16, now_ms: u64) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.missing.remove(&seq);
        let Some(last) = g.last_seq else {
            g.last_seq = Some(seq);
            return;
        };
        let diff = seq.wrapping_sub(last) as i16;
        // 정§11-1 — 순서 역전은 결손 회수만 한다. ★last_seq 를 되돌리지 않는다:
        // 늦은 패킷이 창을 뒤로 끌면 이미 지난 구간을 다시 결손으로 등재한다.
        if diff <= 0 {
            return;
        }
        g.last_seq = Some(seq);
        if diff == 1 {
            return;
        }
        // 128 이상 점프는 등재하지 않는다 — 쏴 봐야 대역만 먹고 화면은 안 돌아온다.
        if (diff as u16) >= MAX_GAP {
            return;
        }
        for step in 1..diff as u16 {
            if g.missing.len() >= MAX_TRACKED {
                return;
            }
            let lost = last.wrapping_add(step);
            g.missing.entry(lost).or_insert(Missing { since_ms: now_ms, asked_at_ms: None });
        }
    }

    /// 지금 요구할 seq 들 — 오름차순이다(묶기의 전제).
    /// 만료된 것은 이 자리에서 걷는다: 장부를 훑는 곳이 여기 하나다.
    pub fn due(&self, now_ms: u64) -> Vec<u16> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.missing.retain(|_, m| now_ms.saturating_sub(m.since_ms) < EXPIRE_MS);
        let mut out = Vec::new();
        for (seq, m) in g.missing.iter_mut() {
            if m.asked_at_ms.is_some_and(|at| now_ms.saturating_sub(at) < RETRY_MS) {
                continue;
            }
            m.asked_at_ms = Some(now_ms);
            out.push(*seq);
        }
        out
    }

    pub fn tracked(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).missing.len()
    }
}

/// RFC 4585 §6.2.1 Generic NACK — `PID` 와 그 뒤 16개를 `BLP` 비트로 묶는다.
/// ★오름차순 입력을 전제한다. 안 그러면 묶이지 않고 한 결손에 한 항목이 나간다.
pub fn pack(seqs: &[u16]) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for &seq in seqs {
        if let Some((pid, blp)) = out.last_mut() {
            let offset = seq.wrapping_sub(*pid);
            if (1..=16).contains(&offset) {
                *blp |= 1 << (offset - 1);
                continue;
            }
        }
        out.push((seq, 0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hole_is_registered_and_a_run_is_not() {
        let g = GapTracker::default();
        g.observe(10, 0);
        g.observe(11, 0);
        assert_eq!(g.tracked(), 0, "이어지면 결손이 아니다");
        g.observe(15, 0);
        assert_eq!(g.due(0), vec![12, 13, 14], "빠진 것만, 오름차순으로");
    }

    #[test]
    fn a_late_packet_collects_its_hole_and_does_not_drag_the_window_back() {
        let g = GapTracker::default();
        g.observe(10, 0);
        g.observe(14, 0);
        assert_eq!(g.tracked(), 3);
        g.observe(12, 0);
        assert_eq!(g.due(0), vec![11, 13], "늦게 온 것은 장부에서 걷는다");

        g.observe(11, 0);
        g.observe(20, 0);
        assert_eq!(
            g.due(0).first().copied(),
            Some(15),
            "★last_seq 를 되돌렸으면 11~13 을 다시 결손으로 등재한다"
        );
    }

    #[test]
    fn a_big_jump_is_a_keyframe_problem_not_a_retransmit_one() {
        let g = GapTracker::default();
        g.observe(1, 0);
        g.observe(1 + MAX_GAP, 0);
        assert_eq!(g.tracked(), 0, "★쏴 봐야 대역만 먹고 화면은 안 돌아온다");
        assert_eq!(g.due(0), Vec::<u16>::new());
    }

    #[test]
    fn asking_twice_waits_for_the_retry_window() {
        let g = GapTracker::default();
        g.observe(1, 0);
        g.observe(4, 0);
        assert_eq!(g.due(0), vec![2, 3]);
        assert_eq!(g.due(RETRY_MS - 1), Vec::<u16>::new(), "간격 안에는 다시 안 묻는다");
        assert_eq!(g.due(RETRY_MS), vec![2, 3]);
    }

    #[test]
    fn a_hole_is_given_up_after_the_window() {
        let g = GapTracker::default();
        g.observe(1, 0);
        g.observe(4, 0);
        assert_eq!(g.due(EXPIRE_MS), Vec::<u16>::new(), "지났으면 포기한다");
        assert_eq!(g.tracked(), 0);
    }

    #[test]
    fn tracking_stops_at_the_ceiling() {
        let g = GapTracker::default();
        g.observe(0, 0);
        g.observe(100, 0);
        assert_eq!(g.tracked(), 99);
        // 한 번 더 벌어지면 상한에 닿는다 — 메모리와 NACK 폭풍을 같이 막는 자리다.
        g.observe(200, 0);
        assert_eq!(g.tracked(), MAX_TRACKED, "★넘겨 담으면 결손 하나에 대역이 계속 나간다");
        g.observe(300, 0);
        assert_eq!(g.tracked(), MAX_TRACKED);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let g = GapTracker::default();
        g.observe(u16::MAX - 1, 0);
        g.observe(2, 0);
        assert_eq!(g.due(0), vec![0, 1, u16::MAX], "감긴 구간도 오름차순 장부다");
    }

    #[test]
    fn nack_packs_a_run_into_one_entry() {
        assert_eq!(pack(&[12, 13, 14]), vec![(12, 0b11)]);
        assert_eq!(pack(&[12, 28]), vec![(12, 1 << 15)], "★BLP 는 PID+1~PID+16 이라 28 까지 한 묶음이다");
        assert_eq!(pack(&[12, 29]), vec![(12, 0), (29, 0)], "17 떨어지면 새 묶음이다");
        assert_eq!(pack(&[]), vec![]);
    }
}
