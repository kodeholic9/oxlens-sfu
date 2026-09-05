// author: kodeholic (powered by Claude)
//! 정§10-3 — 레이어 자동 판단. ★규격이 순수 함수로 못박은 자리다(정§2-3 계약 6).
//! 시계도 신호도 인자로 받는다 — 그래야 전이 전량을 시계만 돌려 잰다.

/// 정§10-3 수치 전량.
pub const LAYER_H_BPS: u64 = 1_650_000;
pub const LAYER_L_BPS: u64 = 250_000;
/// 강등 headroom — 수요의 이 배를 못 받치면 못 버티는 것으로 본다.
pub const HEADROOM: f64 = 1.2;
pub const DEMOTE_LOSS: f64 = 8.0;
pub const CLEAN_LOSS: f64 = 2.0;
pub const DEMOTE_NACK_PER_SEC: f64 = 20.0;
/// ★2연속 성립해야 강등한다 — 한 번의 튐으로 화질을 깎지 않는다.
pub const DEMOTE_STREAK: u8 = 2;
pub const BACKOFF_INIT_MS: u64 = 15_000;
pub const BACKOFF_CAP_MS: u64 = 60_000;
/// 승격 뒤 이 안에 다시 강등되면 "무너짐" — backoff 를 배증한다.
pub const COLLAPSE_MS: u64 = 5_000;
/// 승격 뒤 이만큼은 remb 강등을 건너뛴다. ★이 값이 재강등 창보다 커야 둘이 구분된다.
pub const REMB_GRACE_MS: u64 = 10_000;
pub const REMB_FRESH_MS: u64 = 5_000;
pub const LOSS_FRESH_MS: u64 = 6_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    L,
    H,
}

impl Layer {
    pub fn bps(self) -> u64 {
        match self {
            Layer::H => LAYER_H_BPS,
            Layer::L => LAYER_L_BPS,
        }
    }
}

/// 판정에 쓰는 사실 전부. ★없는 값은 `None` 이다 — 0 으로 채우면 "쟀는데 0" 과 못 가른다.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    /// (bps, 잰 시각). 오래됐으면 판정에 안 쓴다.
    pub remb: Option<(u64, u64)>,
    /// (손실률 %, 잰 시각).
    pub loss: Option<(f64, u64)>,
    pub nack_per_sec: f64,
    pub drops: u64,
}

/// 왜 내렸나 — 계수의 이름이자 로그의 이름이다. ★순서가 계약이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Remb,
    Loss,
    Nack,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Hold,
    Demote(Cause),
    Promote,
}

#[derive(Debug, Clone, Copy)]
pub struct State {
    pub layer: Layer,
    streak: u8,
    streak_cause: Option<Cause>,
    backoff_ms: u64,
    clean_since: Option<u64>,
    promoted_at: Option<u64>,
    demoted_at: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            layer: Layer::H,
            streak: 0,
            streak_cause: None,
            backoff_ms: BACKOFF_INIT_MS,
            clean_since: None,
            promoted_at: None,
            demoted_at: 0,
        }
    }
}

/// 정§10-3 — 한 눈금의 판정. 상태를 고치고 무엇을 할지 돌려준다.
pub fn policy_tick(st: &mut State, now: u64, s: &Signals) -> Decision {
    if let Some(cause) = demote_cause(st, now, s) {
        if st.streak_cause == Some(cause) {
            st.streak = st.streak.saturating_add(1);
        } else {
            st.streak_cause = Some(cause);
            st.streak = 1;
        }
        if st.streak >= DEMOTE_STREAK && st.layer == Layer::H {
            return demote(st, now, cause);
        }
        return Decision::Hold;
    }
    st.streak = 0;
    st.streak_cause = None;
    maybe_promote(st, now, s)
}

/// 강등 4사유는 ★이 순서다. 먼저 걸리는 것이 사유다 — 뒤엣것으로 적으면 처방이 갈린다.
fn demote_cause(st: &State, now: u64, s: &Signals) -> Option<Cause> {
    // ① remb — 승격 유예 동안은 건너뛴다(막 올린 것이 아직 안 찼을 뿐이다).
    let in_grace = st.promoted_at.is_some_and(|at| now.saturating_sub(at) < REMB_GRACE_MS);
    if !in_grace
        && let Some((bps, at)) = s.remb
        && now.saturating_sub(at) <= REMB_FRESH_MS
        && (bps as f64) < st.layer.bps() as f64 * HEADROOM
    {
        return Some(Cause::Remb);
    }
    // ② loss
    if let Some((pct, at)) = s.loss
        && now.saturating_sub(at) <= LOSS_FRESH_MS
        && pct >= DEMOTE_LOSS
    {
        return Some(Cause::Loss);
    }
    // ③ nack
    if s.nack_per_sec > DEMOTE_NACK_PER_SEC {
        return Some(Cause::Nack);
    }
    // ④ drop
    (s.drops > 0).then_some(Cause::Drop)
}

fn demote(st: &mut State, now: u64, cause: Cause) -> Decision {
    // ★승격 직후 무너진 것과 용량이 모자란 것을 가른다 — 배증은 전자에만이다.
    let collapsed = st.promoted_at.is_some_and(|at| now.saturating_sub(at) < COLLAPSE_MS);
    if collapsed {
        st.backoff_ms = (st.backoff_ms * 2).min(BACKOFF_CAP_MS);
    }
    st.layer = Layer::L;
    st.streak = 0;
    st.streak_cause = None;
    st.clean_since = None;
    st.promoted_at = None;
    st.demoted_at = now;
    Decision::Demote(cause)
}

fn maybe_promote(st: &mut State, now: u64, s: &Signals) -> Decision {
    if st.layer == Layer::H {
        return Decision::Hold;
    }
    let clean = s
        .loss
        .is_some_and(|(pct, at)| now.saturating_sub(at) <= LOSS_FRESH_MS && pct <= CLEAN_LOSS);
    if !clean {
        st.clean_since = None;
        return Decision::Hold;
    }
    let since = *st.clean_since.get_or_insert(now);
    // 깨끗한 것만으로는 안 올린다 — backoff 만큼 유지돼야 한다.
    if now.saturating_sub(since) < st.backoff_ms || now.saturating_sub(st.demoted_at) < st.backoff_ms {
        return Decision::Hold;
    }
    st.layer = Layer::H;
    st.promoted_at = Some(now);
    st.clean_since = None;
    Decision::Promote
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(now: u64) -> Signals {
        Signals { loss: Some((0.0, now)), remb: Some((LAYER_H_BPS * 2, now)), ..Signals::default() }
    }

    #[test]
    fn one_bad_tick_does_not_take_the_picture_down() {
        let mut st = State::default();
        let bad = Signals { drops: 1, ..clean(0) };
        assert_eq!(policy_tick(&mut st, 0, &bad), Decision::Hold, "★한 번의 튐으로 깎지 않는다");
        assert_eq!(st.layer, Layer::H);
        assert_eq!(policy_tick(&mut st, 1_000, &bad), Decision::Demote(Cause::Drop));
        assert_eq!(st.layer, Layer::L);
    }

    #[test]
    fn a_different_cause_restarts_the_streak() {
        let mut st = State::default();
        policy_tick(&mut st, 0, &Signals { drops: 1, ..clean(0) });
        let nacky = Signals { nack_per_sec: 30.0, ..clean(1_000) };
        assert_eq!(policy_tick(&mut st, 1_000, &nacky), Decision::Hold, "사유가 갈리면 다시 센다");
        assert_eq!(policy_tick(&mut st, 2_000, &nacky), Decision::Demote(Cause::Nack));
    }

    #[test]
    fn the_first_cause_in_order_is_the_reason() {
        let mut st = State::default();
        // 넷이 한꺼번에 성립한다 — 사유는 remb 다.
        let all = Signals {
            remb: Some((1, 0)),
            loss: Some((50.0, 0)),
            nack_per_sec: 99.0,
            drops: 9,
        };
        policy_tick(&mut st, 0, &all);
        assert_eq!(
            policy_tick(&mut st, 1_000, &all),
            Decision::Demote(Cause::Remb),
            "★뒤엣것으로 적으면 처방이 갈린다"
        );
    }

    #[test]
    fn a_stale_signal_does_not_judge() {
        let mut st = State::default();
        let old = Signals { remb: Some((1, 0)), loss: Some((50.0, 0)), ..Signals::default() };
        let now = LOSS_FRESH_MS + 1;
        assert_eq!(policy_tick(&mut st, now, &old), Decision::Hold);
        assert_eq!(policy_tick(&mut st, now + 1_000, &old), Decision::Hold, "낡은 값으로 깎지 않는다");
        assert_eq!(st.layer, Layer::H);
    }

    /// ★승격 유예(10s) > 재강등 창(5s) 이 두 사유를 가르는 성립 조건이다.
    #[test]
    fn a_collapse_doubles_the_backoff_but_a_shortage_does_not() {
        // 무너짐 — 승격 직후 5초 안에 loss 로 다시 내려간다.
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        let t = BACKOFF_INIT_MS;
        policy_tick(&mut st, 0, &clean(0));
        assert_eq!(policy_tick(&mut st, t, &clean(t)), Decision::Promote);
        let bad = Signals { loss: Some((50.0, t + 1_000)), ..clean(t + 1_000) };
        policy_tick(&mut st, t + 1_000, &bad);
        assert_eq!(policy_tick(&mut st, t + 2_000, &bad), Decision::Demote(Cause::Loss));
        assert_eq!(st.backoff_ms, BACKOFF_INIT_MS * 2, "무너지면 배증한다");

        // 용량 부족 — 유예가 지난 뒤 remb 로 내려간다. 배증하지 않는다.
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 0, &clean(0));
        assert_eq!(policy_tick(&mut st, t, &clean(t)), Decision::Promote);
        let late = t + REMB_GRACE_MS + 1_000;
        let poor = Signals { remb: Some((1, late)), ..clean(late) };
        policy_tick(&mut st, late, &poor);
        assert_eq!(policy_tick(&mut st, late + 1_000, &poor), Decision::Demote(Cause::Remb));
        assert_eq!(st.backoff_ms, BACKOFF_INIT_MS, "★용량이 모자란 것은 무너진 것이 아니다");
    }

    #[test]
    fn the_grace_window_skips_remb_only() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        let t = BACKOFF_INIT_MS;
        policy_tick(&mut st, 0, &clean(0));
        policy_tick(&mut st, t, &clean(t));
        // 유예 안 — remb 가 나빠도 안 내려간다.
        let poor = Signals { remb: Some((1, t + 100)), ..clean(t + 100) };
        policy_tick(&mut st, t + 100, &poor);
        assert_eq!(policy_tick(&mut st, t + 1_100, &poor), Decision::Hold);
        assert_eq!(st.layer, Layer::H, "막 올린 것이 아직 안 찼을 뿐이다");
        // 같은 유예 안이라도 loss 는 내린다 — 건너뛰는 것은 remb 하나다.
        let lossy = Signals { loss: Some((50.0, t + 2_000)), ..clean(t + 2_000) };
        policy_tick(&mut st, t + 2_000, &lossy);
        assert_eq!(policy_tick(&mut st, t + 3_000, &lossy), Decision::Demote(Cause::Loss));
    }

    #[test]
    fn promotion_waits_for_the_backoff_to_pass() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        assert_eq!(policy_tick(&mut st, 0, &clean(0)), Decision::Hold);
        let almost = BACKOFF_INIT_MS - 1;
        assert_eq!(policy_tick(&mut st, almost, &clean(almost)), Decision::Hold, "깨끗한 것만으로는 안 올린다");
        assert_eq!(policy_tick(&mut st, BACKOFF_INIT_MS, &clean(BACKOFF_INIT_MS)), Decision::Promote);
    }

    #[test]
    fn a_dirty_tick_restarts_the_clean_run() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 0, &clean(0));
        let dirty = Signals { loss: Some((CLEAN_LOSS + 0.1, 5_000)), ..clean(5_000) };
        assert_eq!(policy_tick(&mut st, 5_000, &dirty), Decision::Hold);
        // 5초를 이미 깨끗하게 보냈어도 여기서 다시 센다.
        assert_eq!(policy_tick(&mut st, 5_000 + BACKOFF_INIT_MS, &clean(5_000 + BACKOFF_INIT_MS)), Decision::Hold);
    }

    #[test]
    fn the_backoff_has_a_ceiling() {
        let mut st = State { layer: Layer::L, backoff_ms: BACKOFF_CAP_MS, ..State::default() };
        let t = BACKOFF_CAP_MS;
        policy_tick(&mut st, 0, &clean(0));
        assert_eq!(policy_tick(&mut st, t, &clean(t)), Decision::Promote);
        let bad = Signals { drops: 1, ..clean(t + 100) };
        policy_tick(&mut st, t + 100, &bad);
        policy_tick(&mut st, t + 1_100, &bad);
        assert_eq!(st.backoff_ms, BACKOFF_CAP_MS, "천장을 넘지 않는다");
    }
}
