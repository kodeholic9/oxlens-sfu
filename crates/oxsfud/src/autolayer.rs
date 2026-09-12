// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§10-2 · §10-3 · model: claude-opus-5

//! 자동 레이어 — ★★**판정은 순수 함수다**(시계도 신호도 전부 인자, 정§10-3).
//!
//! 비대칭이 이 판의 뼈대다 — ★**내릴 땐 빨리, 올릴 땐 의심하며.**
//! 내림은 네 사유(remb→loss→nack→drop) 중 하나가 ★**두 tick 연속** 서면 곧바로,
//! 올림은 깨끗한 창을 backoff 만큼 견딘 뒤 ★**프로브로 실증**하고서야 선다.
//!
//! ★★**v2 의 실증은 "그 판의 목표를 넘긴 실측" 하나다** — 프로브 중 아무 측정이나
//! 실증으로 읽으면 ★**승격이 널뛴다.** ★**못 잰 것은 `0` 이 아니라 「없음」**이다
//! (`0` 으로 채우면 *"쟀는데 0"* 과 못 갈라 그 판이 통째로 죽는다).
//!
//! ★**올림은 시도다** — 올려 보고 무너지면 내림과 backoff 배증이 잡는다. 다만
//! *"무너짐"* 과 *"용량 부족"* 은 갈라야 한다: 승격 직후 창(5s) 안의 재강등만 배증이고,
//! 유예(10s)가 지난 뒤의 대역 부족 강등은 배증이 아니다.

/// 정§10-3 의 수치 전량.
pub mod v {
    /// 레이어 명목 비트레이트.
    pub const H_BPS: u64 = 1_650_000;
    pub const L_BPS: u64 = 250_000;
    /// 강등 headroom — 수요의 이 배를 못 대면 못 버티는 것이다.
    pub const DEMOTE_HEADROOM: f64 = 1.2;
    /// 강등 손실 / 깨끗 손실.
    pub const DEMOTE_LOSS_PCT: f32 = 8.0;
    pub const CLEAN_LOSS_PCT: f32 = 2.0;
    /// 강등 NACK.
    pub const DEMOTE_NACK_PER_S: f64 = 20.0;
    /// 강등 연속 tick(tick 1,000ms — 판정 해상도 ≈2s).
    pub const DEMOTE_TICKS: u8 = 2;
    pub const TICK_MS: u64 = 1_000;
    /// 승격 backoff 초기 / 상한.
    pub const BACKOFF_INIT_MS: u64 = 15_000;
    pub const BACKOFF_CAP_MS: u64 = 60_000;
    /// 재강등 창 — 이 안의 강등만 *"무너짐"* 이다.
    pub const REDEMOTE_WINDOW_MS: u64 = 5_000;
    /// 승격 뒤 remb 유예.
    pub const PROMOTE_GRACE_MS: u64 = 10_000;
    /// 신호 신선도.
    pub const REMB_FRESH_MS: u64 = 5_000;
    pub const LOSS_FRESH_MS: u64 = 6_000;
    /// 전환 pending 만료.
    pub const PENDING_MS: u64 = 10_000;
    /// PLI 스로틀 — ★`l` 이 촘촘한 까닭은 강등 전환이 `l` 키프레임을 기다리기 때문이다.
    pub const PLI_THROTTLE_H_MS: u64 = 300;
    pub const PLI_THROTTLE_L_MS: u64 = 100;
    /// v2 프로브 — 지속 · 판정 창 · 청크 · 배율 · 상한.
    pub const PROBE_MS: u64 = 2_500;
    pub const PROBE_DECISION_MS: u64 = 4_000;
    pub const PROBE_CHUNK_MS: u64 = 20;
    pub const PROBE_FACTOR: f64 = 1.35;
    pub const PROBE_CAP_BPS: f64 = 3_000_000.0;
    /// 프로브 패딩 한 개의 몸집.
    pub const PROBE_PAD_BYTES: usize = 255;
    /// 신호 불신선 — 넘으면 v1 축으로 내려선다.
    pub const DISTRUST_MS: u64 = 1_000;

    /// ★**성립 조건을 빌드에 박는다**(정§10-3) — 유예가 재강등 창보다 짧아지면
    /// *"용량 부족"* 과 *"무너짐"* 이 갈리지 않아 backoff 가 배증하고 승격이 영영 안 온다.
    /// 값을 바꾸는 사람이 ★**시험이 아니라 컴파일에서** 먼저 걸린다.
    const _: () = assert!(PROMOTE_GRACE_MS > REDEMOTE_WINDOW_MS);
}

/// 공간 단. ★인덱스가 계약이다(낮은 화질이 `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Low = 0,
    High = 1,
}

impl Layer {
    pub fn rid(&self) -> &'static str {
        match self {
            Layer::Low => "l",
            Layer::High => "h",
        }
    }

    pub fn spatial(&self) -> u8 {
        *self as u8
    }

    /// 그 단의 PLI 스로틀 — ★**단마다 버킷이 다르다**(정§10-3).
    pub fn pli_throttle_ms(&self) -> u64 {
        match self {
            Layer::Low => v::PLI_THROTTLE_L_MS,
            Layer::High => v::PLI_THROTTLE_H_MS,
        }
    }
}

/// 상한 결합 — ★**수동 상한과 자동 cap 은 `min`** 이다(정§10-1).
pub fn apply_cap(desired: Layer, cap: Layer) -> Layer {
    desired.min(cap)
}

/// tick 한 번의 판단 입력 ★전부.
#[derive(Debug, Clone, PartialEq)]
pub struct Signals {
    pub now_ms: u64,
    /// 가용 대역 추정 — v1 은 REMB, v2 는 send-side 추정이 이 자리를 쓴다.
    /// ★`None` = 신호 없음(못 잰 것을 `0` 으로 채우지 않는다).
    pub remb_bps: Option<u64>,
    /// 최근 손실률(%). 신선한 것만 `Some`.
    pub loss_pct: Option<f32>,
    /// 손실이 연속으로 나빴던 횟수.
    pub loss_bad_streak: u8,
    /// NACK 율(Δ/s).
    pub nack_per_s: f64,
    /// 우리가 egress 에서 버린 수(tick 사이 Δ).
    pub drop_delta: u64,
    /// 지금 단 기준 수요.
    pub demand_bps: u64,
    /// 전부 High 로 올렸을 때의 수요 — ★프로브 판정의 문턱이다.
    pub demand_high_bps: u64,
    /// 구독자가 High 를 원하는가(수동 상한 천장 안인가).
    pub want_high: bool,
    /// 프로브를 쏠 수 있는가 — v2 신호가 살아 있는 전송로만 참이다.
    pub probe_capable: bool,
}

/// 판단 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// High→Low. 사유는 관측용.
    Demote(&'static str),
    /// Low→High.
    Promote,
    /// 프로브 개시 — ★승격 전에 실측으로 묻는다.
    Probe,
    Hold,
}

/// 판정이 기억해야 하는 것 전부.
#[derive(Debug, Clone)]
pub struct Policy {
    /// 지금 자동 cap. ★High 가 무제약(기본)이다.
    pub cap: Layer,
    demote_streak: u8,
    /// 깨끗한 창의 시작 — ★`None` = 아직 안 깨끗하다.
    clean_since_ms: Option<u64>,
    /// 지금 승격에 요구되는 깨끗 창 길이.
    pub backoff_ms: u64,
    /// 마지막 승격 시각 — ★`None` = 승격한 적 없다(`0` 을 부재 표식으로 쓰지 않는다).
    pub promoted_at: Option<u64>,
    /// 프로브 판정 창 만료 — ★`None` = 프로브 중이 아니다.
    probing_until_ms: Option<u64>,
    /// 관측 — 마지막 강등 사유·시각.
    pub last_demote: Option<(&'static str, u64)>,
}

impl Default for Policy {
    fn default() -> Self {
        Self::new()
    }
}

impl Policy {
    pub fn new() -> Self {
        Self {
            cap: Layer::High,
            demote_streak: 0,
            clean_since_ms: None,
            backoff_ms: v::BACKOFF_INIT_MS,
            promoted_at: None,
            probing_until_ms: None,
            last_demote: None,
        }
    }

    pub fn probing(&self) -> bool {
        self.probing_until_ms.is_some()
    }
}

/// 추정이 수요의 headroom 배를 못 대는가 — 강등 remb 사유.
pub fn below_headroom(s: &Signals) -> bool {
    matches!(s.remb_bps, Some(bps) if s.demand_bps > 0
        && (bps as f64) < v::DEMOTE_HEADROOM * s.demand_bps as f64)
}

/// 강등 사유 ★**넷을 이 순서로** 본다(정§10-3).
///
/// `suppress_remb` = 승격 직후 유예 중 — ★**remb 사유만 침묵**한다. loss·nack·drop 은
/// 그대로 산다(진짜 붕괴는 손실이 정직하게 잡는다. 유예가 그것까지 덮으면 무너진 판을
/// 10초 동안 붙들고 있게 된다).
fn demote_cause(s: &Signals, suppress_remb: bool) -> Option<&'static str> {
    if !suppress_remb && below_headroom(s) {
        return Some("remb");
    }
    if s.loss_bad_streak >= 2 {
        return Some("loss");
    }
    if s.nack_per_s > v::DEMOTE_NACK_PER_S {
        return Some("nack");
    }
    if s.drop_delta > 0 {
        return Some("drop");
    }
    None
}

/// ★**순수 함수** — 시계도 신호도 인자다(정§10-3 · §2-3 계약 6).
pub fn policy_tick(st: &mut Policy, s: &Signals) -> Decision {
    match st.cap {
        Layer::High => {
            let in_grace = matches!(st.promoted_at,
                Some(p) if s.now_ms.saturating_sub(p) < v::PROMOTE_GRACE_MS);
            match demote_cause(s, in_grace) {
                Some(cause) => {
                    st.demote_streak = st.demote_streak.saturating_add(1);
                    if st.demote_streak < v::DEMOTE_TICKS {
                        return Decision::Hold;
                    }
                    st.cap = Layer::Low;
                    st.demote_streak = 0;
                    st.clean_since_ms = None;
                    st.last_demote = Some((cause, s.now_ms));
                    // ★승격 직후 창 안이면 *"무너짐"* — backoff 를 배증한다.
                    //   유예 밖 remb 강등은 *"용량 부족"* 이라 배증이 없다.
                    if let Some(p) = st.promoted_at
                        && s.now_ms.saturating_sub(p) < v::REDEMOTE_WINDOW_MS
                    {
                        st.backoff_ms = (st.backoff_ms * 2).min(v::BACKOFF_CAP_MS);
                    }
                    Decision::Demote(cause)
                }
                None => {
                    st.demote_streak = 0;
                    // 승격이 창을 살아남았다 → backoff 를 원래대로 돌린다.
                    if let Some(p) = st.promoted_at
                        && s.now_ms.saturating_sub(p) >= v::REDEMOTE_WINDOW_MS
                        && st.backoff_ms > v::BACKOFF_INIT_MS
                    {
                        st.backoff_ms = v::BACKOFF_INIT_MS;
                    }
                    Decision::Hold
                }
            }
        }
        Layer::Low => {
            // ★깨끗 판정에 remb 를 넣지 않는다 — `l` 만 받는 동안 추정은 낮게 유지되므로
            //   그것을 승격 관문으로 삼으면 ★**승격이 영영 안 온다.** 실증은 프로브 몫이다.
            let loss_clean = s.loss_pct.is_none_or(|p| p < v::CLEAN_LOSS_PCT);
            let clean = loss_clean && s.nack_per_s < 1.0 && s.drop_delta == 0;

            if let Some(until) = st.probing_until_ms {
                if !clean {
                    // 프로브 중에 더러워졌다 — 실패로 닫는다(쏘는 것을 멈추는 건 실행기 몫).
                    st.probing_until_ms = None;
                    st.clean_since_ms = None;
                    return Decision::Hold;
                }
                // ★**목표를 넘긴 실측만 실증이다.**
                let proven = matches!(s.remb_bps, Some(e) if s.demand_high_bps > 0
                    && (e as f64) >= v::DEMOTE_HEADROOM * s.demand_high_bps as f64);
                if proven {
                    st.probing_until_ms = None;
                    st.cap = Layer::High;
                    st.clean_since_ms = None;
                    st.demote_streak = 0;
                    st.promoted_at = Some(s.now_ms);
                    return Decision::Promote;
                }
                if s.now_ms >= until {
                    // ★실패는 *"용량 부족 확인"* 이다 — 배증 없이 깨끗 창만 다시 센다.
                    st.probing_until_ms = None;
                    st.clean_since_ms = None;
                }
                return Decision::Hold;
            }

            if !(s.want_high && clean) {
                st.clean_since_ms = None;
                return Decision::Hold;
            }
            let since = *st.clean_since_ms.get_or_insert(s.now_ms);
            if s.now_ms.saturating_sub(since) < st.backoff_ms {
                return Decision::Hold;
            }
            if s.probe_capable {
                st.probing_until_ms = Some(s.now_ms + v::PROBE_DECISION_MS);
                return Decision::Probe;
            }
            // v1 폴백 — 프로브가 없으면 ★올림은 시도다.
            st.cap = Layer::High;
            st.clean_since_ms = None;
            st.demote_streak = 0;
            st.promoted_at = Some(s.now_ms);
            Decision::Promote
        }
    }
}

/// 프로브가 쏠 속도 — ★**수요의 배율**이되 상한으로 자른다.
pub fn probe_rate_bps(demand_high_bps: u64) -> f64 {
    (demand_high_bps as f64 * v::PROBE_FACTOR).min(v::PROBE_CAP_BPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(now: u64) -> Signals {
        Signals {
            now_ms: now,
            remb_bps: Some(10_000_000),
            loss_pct: Some(0.0),
            loss_bad_streak: 0,
            nack_per_s: 0.0,
            drop_delta: 0,
            demand_bps: v::H_BPS,
            demand_high_bps: v::H_BPS,
            want_high: true,
            probe_capable: true,
        }
    }

    #[test]
    fn 한_tick_으로는_안_내린다() {
        // ★한 수 튀었다고 내리면 단이 널뛴다 — 두 tick 연속이 계약이다.
        let mut st = Policy::new();
        let mut s = sig(1_000);
        s.remb_bps = Some(100_000);
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        assert_eq!(policy_tick(&mut st, &sig(2_000)), Decision::Hold, "깨끗하면 연속이 끊긴다");
        s.now_ms = 3_000;
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        s.now_ms = 4_000;
        assert_eq!(policy_tick(&mut st, &s), Decision::Demote("remb"));
        assert_eq!(st.cap, Layer::Low);
    }

    #[test]
    fn 사유는_이_순서로_본다() {
        let mut s = sig(0);
        s.remb_bps = Some(100_000);
        s.loss_bad_streak = 2;
        s.nack_per_s = 100.0;
        s.drop_delta = 5;
        assert_eq!(demote_cause(&s, false), Some("remb"));
        s.remb_bps = Some(10_000_000);
        assert_eq!(demote_cause(&s, false), Some("loss"));
        s.loss_bad_streak = 0;
        assert_eq!(demote_cause(&s, false), Some("nack"));
        s.nack_per_s = 0.0;
        assert_eq!(demote_cause(&s, false), Some("drop"));
        s.drop_delta = 0;
        assert_eq!(demote_cause(&s, false), None);
    }

    #[test]
    fn 유예는_remb_만_덮는다() {
        // ★진짜 붕괴는 손실이 정직하게 잡는다 — 유예가 그것까지 덮으면 안 된다.
        let mut s = sig(0);
        s.remb_bps = Some(100_000);
        assert_eq!(demote_cause(&s, true), None, "★remb 만 침묵");
        s.loss_bad_streak = 2;
        assert_eq!(demote_cause(&s, true), Some("loss"));
    }

    #[test]
    fn 올림은_깨끗한_창을_견디고_프로브로_묻는다() {
        let mut st = Policy::new();
        st.cap = Layer::Low;
        // 깨끗 창이 backoff 에 못 미치는 동안은 아무 일 없다.
        assert_eq!(policy_tick(&mut st, &sig(0)), Decision::Hold);
        assert_eq!(policy_tick(&mut st, &sig(v::BACKOFF_INIT_MS - 1)), Decision::Hold);
        assert_eq!(policy_tick(&mut st, &sig(v::BACKOFF_INIT_MS)), Decision::Probe);
        assert!(st.probing());
        // 실측이 문턱을 넘으면 승격.
        let ok = policy_tick(&mut st, &sig(v::BACKOFF_INIT_MS + 1_000));
        assert_eq!(ok, Decision::Promote);
        assert_eq!(st.cap, Layer::High);
    }

    #[test]
    fn 실증_없이는_안_올린다() {
        let mut st = Policy::new();
        st.cap = Layer::Low;
        // 깨끗 창을 먼저 연다 — 창이 backoff 를 채워야 프로브가 선다.
        policy_tick(&mut st, &sig(0));
        let mut s = sig(v::BACKOFF_INIT_MS);
        assert_eq!(policy_tick(&mut st, &s), Decision::Probe);
        // ★못 잰 것은 실증이 아니다 — `0` 으로 채웠다면 여기서 "쟀는데 0" 과 못 가른다.
        s.remb_bps = None;
        s.now_ms += 1_000;
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        // 목표(수요 ×1.2)에 못 미치는 실측도 실증이 아니다.
        s.remb_bps = Some((v::H_BPS as f64 * 1.19) as u64);
        s.now_ms += 1_000;
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        assert_eq!(st.cap, Layer::Low);
        // 판정 창이 지나면 프로브가 닫히고 깨끗 창을 다시 센다 — ★배증은 없다.
        s.now_ms = v::BACKOFF_INIT_MS + v::PROBE_DECISION_MS;
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        assert!(!st.probing());
        assert_eq!(st.backoff_ms, v::BACKOFF_INIT_MS, "★실패는 용량 부족 확인이지 무너짐이 아니다");
    }

    #[test]
    fn 프로브가_없으면_올림은_시도다() {
        let mut st = Policy::new();
        st.cap = Layer::Low;
        let mut zero = sig(0);
        zero.probe_capable = false;
        policy_tick(&mut st, &zero);
        let mut s = sig(v::BACKOFF_INIT_MS);
        s.probe_capable = false;
        assert_eq!(policy_tick(&mut st, &s), Decision::Promote);
        assert_eq!(st.cap, Layer::High);
    }

    #[test]
    fn 무너짐은_배증이고_용량_부족은_아니다() {
        let mut st = Policy::new();
        st.cap = Layer::Low;
        policy_tick(&mut st, &sig(0));
        assert_eq!(policy_tick(&mut st, &sig(v::BACKOFF_INIT_MS)), Decision::Probe);
        assert_eq!(policy_tick(&mut st, &sig(v::BACKOFF_INIT_MS)), Decision::Promote);
        let promoted = v::BACKOFF_INIT_MS;

        // 창(5s) 안에 손실로 무너진다 → 배증.
        let mut bad = sig(promoted + 1_000);
        bad.loss_bad_streak = 2;
        assert_eq!(policy_tick(&mut st, &bad), Decision::Hold);
        bad.now_ms += 1_000;
        assert_eq!(policy_tick(&mut st, &bad), Decision::Demote("loss"));
        assert_eq!(st.backoff_ms, v::BACKOFF_INIT_MS * 2);

        // 다시 승격시킨 뒤, 이번엔 유예(10s) 밖에서 remb 로 내려간다 → 배증 없음.
        let t = promoted + 2_000;
        let mut st2 = Policy::new();
        st2.promoted_at = Some(t);
        let mut poor = sig(t + v::PROMOTE_GRACE_MS + 1);
        poor.remb_bps = Some(100_000);
        assert_eq!(policy_tick(&mut st2, &poor), Decision::Hold);
        poor.now_ms += 1_000;
        assert_eq!(policy_tick(&mut st2, &poor), Decision::Demote("remb"));
        assert_eq!(st2.backoff_ms, v::BACKOFF_INIT_MS, "★용량 부족은 배증이 아니다");
    }

    #[test]
    fn 유예_안의_remb_로는_안_내려간다() {
        let mut st = Policy::new();
        st.promoted_at = Some(1_000);
        let mut s = sig(1_000);
        s.remb_bps = Some(100_000);
        for k in 0..5 {
            s.now_ms = 1_000 + k * 1_000;
            assert_eq!(policy_tick(&mut st, &s), Decision::Hold, "k={k}");
        }
        assert_eq!(st.cap, Layer::High);
        // 유예가 끝나면 두 tick 만에 내려간다.
        s.now_ms = 1_000 + v::PROMOTE_GRACE_MS;
        assert_eq!(policy_tick(&mut st, &s), Decision::Hold);
        s.now_ms += 1_000;
        assert_eq!(policy_tick(&mut st, &s), Decision::Demote("remb"));
    }

    #[test]
    fn 살아남으면_backoff_가_돌아온다() {
        let mut st = Policy::new();
        st.backoff_ms = v::BACKOFF_CAP_MS;
        st.promoted_at = Some(0);
        policy_tick(&mut st, &sig(v::REDEMOTE_WINDOW_MS));
        assert_eq!(st.backoff_ms, v::BACKOFF_INIT_MS);
    }

    #[test]
    fn 상한은_min_으로_결합한다() {
        assert_eq!(apply_cap(Layer::High, Layer::Low), Layer::Low);
        assert_eq!(apply_cap(Layer::Low, Layer::High), Layer::Low, "★자동이 올려도 수동을 못 넘는다");
        assert_eq!(apply_cap(Layer::High, Layer::High), Layer::High);
    }

    #[test]
    fn 프로브_속도는_상한으로_잘린다() {
        assert_eq!(probe_rate_bps(1_000_000), 1_350_000.0);
        assert_eq!(probe_rate_bps(4_000_000), v::PROBE_CAP_BPS);
    }
}
