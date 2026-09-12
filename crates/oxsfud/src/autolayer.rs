// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§10-2 · §10-3 · model: claude-opus-5

//! 자동 레이어 — ★★**v2 의 실증은 "그 판의 목표를 넘긴 실측" 하나다.**
//!
//! 프로브 중 아무 측정이나 실증으로 읽으면 ★**승격이 널뛴다.**
//! ★**못 잰 것은 `0` 이 아니라 「없음」**이다 — `0` 으로 채우면 *"쟀는데 0"* 과 못 가른다.
//! ★**인코더가 층 명목값만큼 안 내는 판**이 있으므로 판정은 명목이 아니라 ★**실수신 속도**로 묻는다.

/// 정§10-2 값.
pub mod v {
    /// 프로브 지속.
    pub const PROBE_MS: u64 = 2_500;
    /// 판정 창.
    pub const DECIDE_MS: u64 = 4_000;
    /// 목표 배율.
    pub const FACTOR: f64 = 1.35;
    /// 상한.
    pub const CAP_BPS: u64 = 3_000_000;
    /// ★**승격 유예 > 재강등 창**이어야 둘이 구분된다.
    pub const PROMOTE_GRACE_MS: u64 = 10_000;
    pub const DEMOTE_WINDOW_MS: u64 = 5_000;
    /// 신호 불신선 — 넘으면 v1 로 내려선다.
    pub const DISTRUST_MS: u64 = 1_000;

    /// ★**성립 조건을 빌드에 박는다**(정§10-2) — 유예가 재강등 창보다 짧아지면
    /// *"용량 부족"* 과 *"무너짐"* 이 갈리지 않아 backoff 가 배증하고 승격이 영영 안 온다.
    /// 값을 바꾸는 사람이 ★**시험이 아니라 컴파일에서** 먼저 걸린다.
    const _: () = assert!(PROMOTE_GRACE_MS > DEMOTE_WINDOW_MS);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 아무것도 안 한다.
    Hold,
    /// 한 단 올린다.
    Promote,
    /// 한 단 내린다.
    Demote,
    /// ★신호를 못 믿는다 — v1 축으로 내려선다.
    FallbackV1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    /// ★**실수신 속도(측정값).** ★`None` = 아직 안 쟀다 — 그 상태로는 안 올린다.
    pub measured_bps: Option<u64>,
    /// 지금 층의 명목 속도.
    pub nominal_bps: u64,
    /// 마지막 측정 시각.
    pub measured_at: Option<u64>,
    /// 마지막 승격 시각.
    pub promoted_at: Option<u64>,
    /// 최근 창에서 무너졌나(손실·정체).
    pub collapsed: bool,
}

/// ★**프로브가 도는 동안 목표를 넘긴 실측만 실증이다.**
pub fn proven(s: &Signals) -> bool {
    match s.measured_bps {
        // ★못 잰 것을 0 으로 채우면 "쟀는데 0" 과 못 갈라 그 판이 통째로 죽는다.
        None => false,
        Some(bps) => {
            let target = (s.nominal_bps as f64 * v::FACTOR) as u64;
            bps >= target.min(v::CAP_BPS)
        }
    }
}

/// 한 판정.
pub fn decide(s: &Signals, now: u64) -> Decision {
    // ★신호가 낡으면 v1 로 내려선다 — 모르는 채로 올리지 않는다.
    match s.measured_at {
        Some(t) if now.saturating_sub(t) > v::DISTRUST_MS => return Decision::FallbackV1,
        None => return Decision::FallbackV1,
        _ => {}
    }
    if s.collapsed {
        // ★승격 유예 안의 무너짐은 "용량 부족"이지 "무너짐"이 아니다 —
        //   유예가 재강등 창보다 길어야 그 둘이 갈린다.
        if let Some(p) = s.promoted_at
            && now.saturating_sub(p) < v::PROMOTE_GRACE_MS
        {
            return Decision::Hold;
        }
        return Decision::Demote;
    }
    if proven(s) { Decision::Promote } else { Decision::Hold }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig() -> Signals {
        Signals {
            measured_bps: None,
            nominal_bps: 1_000_000,
            measured_at: Some(1_000),
            promoted_at: None,
            collapsed: false,
        }
    }

    #[test]
    fn 못_잰_것으로는_안_올린다() {
        // ★0 으로 채우면 "쟀는데 0" 과 못 가른다.
        let s = sig();
        assert!(!proven(&s));
        assert_eq!(decide(&s, 1_000), Decision::Hold);
    }

    #[test]
    fn 목표를_넘겨야_실증이다() {
        let mut s = sig();
        s.measured_bps = Some(1_200_000); // 1.2배 — 목표 1.35배에 못 미친다
        assert!(!proven(&s));
        assert_eq!(decide(&s, 1_000), Decision::Hold);
        s.measured_bps = Some(1_350_000);
        assert!(proven(&s));
        assert_eq!(decide(&s, 1_000), Decision::Promote);
    }

    #[test]
    fn 명목이_아니라_실수신으로_묻는다() {
        // ★인코더가 층 명목값만큼 안 내는 판이 있다.
        let mut s = sig();
        s.nominal_bps = 3_000_000;
        s.measured_bps = Some(1_000_000);
        assert!(!proven(&s), "명목이 커도 실측이 낮으면 실증이 아니다");
    }

    #[test]
    fn 상한을_넘겨_요구하지_않는다() {
        let mut s = sig();
        s.nominal_bps = 4_000_000; // ×1.35 = 5.4M — 상한 3M 를 넘는다
        s.measured_bps = Some(3_000_000);
        assert!(proven(&s), "★목표는 상한으로 잘린다");
    }

    #[test]
    fn 신호가_낡으면_v1_로_내려선다() {
        let mut s = sig();
        s.measured_bps = Some(9_000_000);
        assert_eq!(decide(&s, 1_000 + v::DISTRUST_MS), Decision::Promote);
        assert_eq!(decide(&s, 1_000 + v::DISTRUST_MS + 1), Decision::FallbackV1);
        s.measured_at = None;
        assert_eq!(decide(&s, 0), Decision::FallbackV1);
    }

    #[test]
    fn 승격_유예_안의_무너짐은_강등이_아니다() {
        // ★유예 > 재강등 창은 `v` 모듈이 컴파일 시각에 박는다(여기서는 결과만 본다).
        let mut s = sig();
        s.measured_bps = Some(1_000);
        s.collapsed = true;
        s.promoted_at = Some(1_000);
        s.measured_at = Some(6_000);
        assert_eq!(decide(&s, 6_000), Decision::Hold, "★유예 안");
        s.measured_at = Some(11_001);
        assert_eq!(decide(&s, 11_001), Decision::Demote, "★유예 밖");
    }
}
