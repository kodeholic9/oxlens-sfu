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
/// v2 — 이보다 낡은 send-side 신호는 못 믿는다. 그때는 v1 축(REMB+RR)으로 내려선다.
pub const V2_STALE_MS: u64 = 1_000;
/// v2 프로브 — 지속 · 판정 창 · 청크 · 목표 배율 · 상한.
pub const PROBE_HOLD_MS: u64 = 2_500;
pub const PROBE_VERDICT_MS: u64 = 4_000;
pub const PROBE_CHUNK_MS: u64 = 20;
pub const PROBE_FACTOR: f64 = 1.35;
pub const PROBE_CAP_BPS: u64 = 3_000_000;

/// 정§10-3 운영 모드 셋. ★`off` 가 끄는 것은 이 절(자동 판단)뿐이다 — §10-1·§10-2 는 항상 돈다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    /// REMB + RR loss.
    V1,
    /// ★TWCC send-side — 서버가 egress 에 번호를 찍고 구독자 피드백으로 실수신 속도를 잰다 + RTX 패딩 프로브.
    V2,
}

impl Mode {
    pub fn parse(s: &str) -> Self {
        match s {
            "v1" => Mode::V1,
            "v2" => Mode::V2,
            _ => Mode::Off,
        }
    }
    pub fn on(self) -> bool {
        self != Mode::Off
    }
    pub fn is_v2(self) -> bool {
        self == Mode::V2
    }
}

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
    /// v2 — 구독자 피드백이 확인해 준 ★실수신 속도(측정값). 낡으면 v1 축으로 내려선다.
    pub send_side: Option<(u64, u64)>,
    /// v2 — TWCC 가 알려 준 손실률. RR 과 같은 자리를 다른 축이 채운다.
    pub send_loss: Option<(f64, u64)>,
    /// v2 — RTX 패딩 프로브가 ★실증한 속도. ★`None` = 아직 안 쟀다이고, 그 상태로는 안 올린다.
    pub probe: Option<(u64, u64)>,
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
    /// v2 — 올릴 조건은 다 섰는데 ★실측이 없다. 프로브를 쏘고 그 결과로 다시 묻는다.
    Probe,
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
/// v2 프로브가 겨눌 송신량 — 지금 받아내는 값의 배율만큼, 상한까지.
pub fn probe_target_bps(current_bps: u64) -> u64 {
    (((current_bps as f64) * PROBE_FACTOR) as u64).min(PROBE_CAP_BPS)
}

pub fn policy_tick(st: &mut State, now: u64, s: &Signals, mode: Mode) -> Decision {
    // ★내려갈 자리가 있을 때만 강등을 잰다. `L` 에서 강등 사유를 재면 그것이 서 있는 동안
    //   승격 판정에 영영 못 간다 — 인코더가 층 명목값만큼 안 내는 판에서는 그 사유가 상시 참이다.
    if st.layer == Layer::H {
        if let Some(cause) = demote_cause(st, now, s, mode) {
            if st.streak_cause == Some(cause) {
                st.streak = st.streak.saturating_add(1);
            } else {
                st.streak_cause = Some(cause);
                st.streak = 1;
            }
            if st.streak >= DEMOTE_STREAK {
                return demote(st, now, cause);
            }
            return Decision::Hold;
        }
        st.streak = 0;
        st.streak_cause = None;
        return Decision::Hold;
    }
    st.streak = 0;
    st.streak_cause = None;
    maybe_promote(st, now, s, mode)
}

/// 정§10-3 — 대역 축의 값 하나. ★v2 면 실측 수신률이고, 그것이 낡으면(1s) v1 축으로 내려선다.
/// 두 축이 같은 자리를 채우므로 강등 4사유의 **순서**(remb→loss→nack→drop)는 그대로 산다.
///
/// ★이 축은 「지금 층을 받칠 만큼 받아내고 있나」를 묻는다. 인코더가 층 명목값만큼 안 내는 판에서는
/// 늘 참이 되는데, 그것을 막는 것이 ★**승격 유예**(10s — 이 축**만** 건너뛴다)다. 유예 동안 올라간
/// 화질이 실제로 무너지면 손실 축이 잡는다 — 두 축이 그렇게 짝을 이룬다.
fn bandwidth(now: u64, s: &Signals, mode: Mode) -> Option<(u64, u64)> {
    if mode.is_v2()
        && let Some((bps, at)) = s.send_side
        && now.saturating_sub(at) <= V2_STALE_MS
    {
        return Some((bps, at));
    }
    s.remb
}

/// 손실 축 — ★v2 면 TWCC 가, 아니면 RR 이 그 자리를 채운다. v2 의 강등·승격이 전부 여기 걸린다.
fn loss_of(now: u64, s: &Signals, mode: Mode) -> Option<(f64, u64)> {
    if mode.is_v2()
        && let Some((pct, at)) = s.send_loss
        && now.saturating_sub(at) <= V2_STALE_MS
    {
        return Some((pct, at));
    }
    s.loss
}

/// 강등 4사유는 ★이 순서다. 먼저 걸리는 것이 사유다 — 뒤엣것으로 적으면 처방이 갈린다.
fn demote_cause(st: &State, now: u64, s: &Signals, mode: Mode) -> Option<Cause> {
    // ① remb — 승격 유예 동안은 건너뛴다(막 올린 것이 아직 안 찼을 뿐이다).
    let in_grace = st.promoted_at.is_some_and(|at| now.saturating_sub(at) < REMB_GRACE_MS);
    if !in_grace
        && let Some((bps, at)) = bandwidth(now, s, mode)
        && now.saturating_sub(at) <= REMB_FRESH_MS
        && (bps as f64) < st.layer.bps() as f64 * HEADROOM
    {
        return Some(Cause::Remb);
    }
    // ② loss
    if let Some((pct, at)) = loss_of(now, s, mode)
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

fn maybe_promote(st: &mut State, now: u64, s: &Signals, mode: Mode) -> Decision {
    if st.layer == Layer::H {
        return Decision::Hold;
    }
    let clean = loss_of(now, s, mode)
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
    // ★v2 는 실측 없이 안 올린다 — 프로브가 「지금보다 이만큼 더 흘려도 그대로 도착한다」를
    //   보여야 한다(정§10-3 프로브 게이트). 기준은 ★그 판의 목표(지금 실측 × 배율)이지
    //   층의 절대 비트레이트가 아니다 — 절대값을 기준으로 삼으면 인코더가 그만큼 안 내는 판에서
    //   승격이 영영 안 온다. `None` 은 "아직 안 쟀다" 이고 그 상태의 승격이 곧 게이트가 뚫린 것이다.
    if mode.is_v2() && s.probe.is_none_or(|(_, at)| now.saturating_sub(at) > PROBE_VERDICT_MS) {
        return Decision::Probe;
    }
    st.layer = Layer::H;
    st.promoted_at = Some(now);
    st.clean_since = None;
    Decision::Promote
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v2 신호 한 벌 — 두 축을 v2 가 채운다: 실측 수신률(대역)과 지연 증가(손실).
    fn v2(now: u64, miss_pct: f64) -> Signals {
        Signals { send_side: Some((LAYER_H_BPS * 4, now)), send_loss: Some((miss_pct, now)), ..Signals::default() }
    }

    /// ★v2 는 실측 없이 안 올린다 — 프로브를 물어보고, 실증이 서면 그때 올린다(정§10-3 프로브 게이트).
    #[test]
    fn v2_asks_a_probe_before_it_promotes_and_never_promotes_without_one() {
        let mut st = State::default();
        // 보낸 것이 창 안에 안 들어온다(유실이든 밀림이든) → 2연속에 강등.
        assert_eq!(policy_tick(&mut st, 0, &v2(0, DEMOTE_LOSS + 1.0), Mode::V2), Decision::Hold);
        assert_eq!(policy_tick(&mut st, 1_000, &v2(1_000, DEMOTE_LOSS + 1.0), Mode::V2), Decision::Demote(Cause::Loss));
        assert_eq!(st.layer, Layer::L);

        // 깨끗해도 backoff 전에는 아무것도 안 한다(깨끗함이 그만큼 **유지**돼야 한다).
        let t = 1_001;
        assert_eq!(policy_tick(&mut st, t, &v2(t, 0.0), Mode::V2), Decision::Hold);
        let t = 1_000 + BACKOFF_INIT_MS - 1;
        assert_eq!(policy_tick(&mut st, t, &v2(t, 0.0), Mode::V2), Decision::Hold);

        // backoff 가 지나면 ★올리는 것이 아니라 **묻는다**.
        let t = 1_000 + BACKOFF_INIT_MS + 1;
        assert_eq!(policy_tick(&mut st, t, &v2(t, 0.0), Mode::V2), Decision::Probe);
        assert_eq!(st.layer, Layer::L, "물어보는 동안은 안 올린다");
        // 여러 번 물어도 실증이 없으면 계속 묻는다 — ★없는 실측으로 올리지 않는다.
        assert_eq!(policy_tick(&mut st, t + 1_000, &v2(t + 1_000, 0.0), Mode::V2), Decision::Probe);

        // 실증이 서면 올린다.
        let t2 = t + 2_000;
        let proven = Signals { probe: Some((LAYER_H_BPS * 2, t2)), ..v2(t2, 0.0) };
        assert_eq!(policy_tick(&mut st, t2, &proven, Mode::V2), Decision::Promote);
        assert_eq!(st.layer, Layer::H);
    }

    /// ★`L` 에서는 강등 사유를 재지 않는다 — 내려갈 자리가 없다.
    /// 그것을 재면 사유가 서 있는 동안 승격 판정에 영영 못 가고, 화질이 바닥에 눌러앉는다.
    #[test]
    fn at_the_bottom_a_standing_demote_reason_does_not_block_the_way_up() {
        let mut st = State::default();
        // 대역 사유가 상시 참인 판(인코더가 층 명목값만큼 안 낸다) — 그래도 내려간 뒤엔 올라갈 길이 있다.
        let low = |t: u64| Signals { send_side: Some((1_000, t)), send_loss: Some((0.0, t)), ..Signals::default() };
        assert_eq!(policy_tick(&mut st, 0, &low(0), Mode::V2), Decision::Hold);
        assert_eq!(policy_tick(&mut st, 1_000, &low(1_000), Mode::V2), Decision::Demote(Cause::Remb));
        // 사유는 그대로 서 있다. 그래도 backoff 뒤엔 ★묻는다.
        let t = 1_000 + BACKOFF_INIT_MS + 1;
        assert_eq!(policy_tick(&mut st, 1_001, &low(1_001), Mode::V2), Decision::Hold);
        assert_eq!(policy_tick(&mut st, t, &low(t), Mode::V2), Decision::Probe);
    }

    /// 낡은 실증으로는 안 올린다 — 판정 창(4s)을 넘긴 값은 지난 이야기다.
    #[test]
    fn a_stale_probe_is_not_evidence() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 1, &v2(1, 0.0), Mode::V2);   // 깨끗함이 여기서 시작한다
        let t = BACKOFF_INIT_MS + 2;
        let stale = Signals { probe: Some((LAYER_H_BPS * 2, 0)), ..v2(t, 0.0) };
        assert!(t > PROBE_VERDICT_MS);
        assert_eq!(policy_tick(&mut st, t, &stale, Mode::V2), Decision::Probe, "낡은 실증은 없는 것과 같다");
    }

    /// ★v2 신호가 낡으면 v1 축(REMB·RR)으로 내려선다 — 안 내려서면 그 구독자만 판단이 멎는다.
    #[test]
    fn v2_falls_back_to_v1_when_its_own_signal_goes_stale() {
        let mut st = State::default();
        let now = 10_000;
        // v2 실측은 낡았고(1s 초과) v1 REMB 는 신선하며 지금 층을 못 받친다.
        // v2 손실 축은 낡았고(1s 초과) v1 축(REMB·RR)은 신선하며 지금 층을 못 받친다.
        let mixed = Signals {
            send_loss: Some((0.0, now - V2_STALE_MS - 1)),
            remb: Some((LAYER_L_BPS, now)),
            loss: Some((0.0, now)),
            ..Signals::default()
        };
        assert_eq!(policy_tick(&mut st, now, &mixed, Mode::V2), Decision::Hold);
        assert_eq!(policy_tick(&mut st, now + 1_000, &mixed, Mode::V2), Decision::Demote(Cause::Remb),
            "낡은 v2 를 붙들지 않고 v1 이 본 것으로 내린다");
    }

    /// v1 은 프로브를 묻지 않는다 — 프로브는 v2 의 축이다.
    #[test]
    fn v1_promotes_without_a_probe() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 1, &clean(1), Mode::V1);
        let t = BACKOFF_INIT_MS + 2;
        assert_eq!(policy_tick(&mut st, t, &clean(t), Mode::V1), Decision::Promote);
    }

    #[test]
    fn the_probe_target_is_the_measured_rate_times_the_factor_capped() {
        assert_eq!(probe_target_bps(1_000_000), 1_350_000);
        assert_eq!(probe_target_bps(PROBE_CAP_BPS * 2), PROBE_CAP_BPS, "상한을 넘지 않는다");
    }

    fn clean(now: u64) -> Signals {
        Signals { loss: Some((0.0, now)), remb: Some((LAYER_H_BPS * 2, now)), ..Signals::default() }
    }

    #[test]
    fn one_bad_tick_does_not_take_the_picture_down() {
        let mut st = State::default();
        let bad = Signals { drops: 1, ..clean(0) };
        assert_eq!(policy_tick(&mut st, 0, &bad, Mode::V1), Decision::Hold, "★한 번의 튐으로 깎지 않는다");
        assert_eq!(st.layer, Layer::H);
        assert_eq!(policy_tick(&mut st, 1_000, &bad, Mode::V1), Decision::Demote(Cause::Drop));
        assert_eq!(st.layer, Layer::L);
    }

    #[test]
    fn a_different_cause_restarts_the_streak() {
        let mut st = State::default();
        policy_tick(&mut st, 0, &Signals { drops: 1, ..clean(0) }, Mode::V1);
        let nacky = Signals { nack_per_sec: 30.0, ..clean(1_000) };
        assert_eq!(policy_tick(&mut st, 1_000, &nacky, Mode::V1), Decision::Hold, "사유가 갈리면 다시 센다");
        assert_eq!(policy_tick(&mut st, 2_000, &nacky, Mode::V1), Decision::Demote(Cause::Nack));
    }

    #[test]
    fn the_first_cause_in_order_is_the_reason() {
        let mut st = State::default();
        // 넷이 한꺼번에 성립한다 — 사유는 remb 다.
        let all = Signals {
            send_side: None,
            send_loss: None,
            probe: None,
            remb: Some((1, 0)),
            loss: Some((50.0, 0)),
            nack_per_sec: 99.0,
            drops: 9,
        };
        policy_tick(&mut st, 0, &all, Mode::V1);
        assert_eq!(
            policy_tick(&mut st, 1_000, &all, Mode::V1),
            Decision::Demote(Cause::Remb),
            "★뒤엣것으로 적으면 처방이 갈린다"
        );
    }

    #[test]
    fn a_stale_signal_does_not_judge() {
        let mut st = State::default();
        let old = Signals { remb: Some((1, 0)), loss: Some((50.0, 0)), ..Signals::default() };
        let now = LOSS_FRESH_MS + 1;
        assert_eq!(policy_tick(&mut st, now, &old, Mode::V1), Decision::Hold);
        assert_eq!(policy_tick(&mut st, now + 1_000, &old, Mode::V1), Decision::Hold, "낡은 값으로 깎지 않는다");
        assert_eq!(st.layer, Layer::H);
    }

    /// ★승격 유예(10s) > 재강등 창(5s) 이 두 사유를 가르는 성립 조건이다.
    #[test]
    fn a_collapse_doubles_the_backoff_but_a_shortage_does_not() {
        // 무너짐 — 승격 직후 5초 안에 loss 로 다시 내려간다.
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        let t = BACKOFF_INIT_MS;
        policy_tick(&mut st, 0, &clean(0), Mode::V1);
        assert_eq!(policy_tick(&mut st, t, &clean(t), Mode::V1), Decision::Promote);
        let bad = Signals { loss: Some((50.0, t + 1_000)), ..clean(t + 1_000) };
        policy_tick(&mut st, t + 1_000, &bad, Mode::V1);
        assert_eq!(policy_tick(&mut st, t + 2_000, &bad, Mode::V1), Decision::Demote(Cause::Loss));
        assert_eq!(st.backoff_ms, BACKOFF_INIT_MS * 2, "무너지면 배증한다");

        // 용량 부족 — 유예가 지난 뒤 remb 로 내려간다. 배증하지 않는다.
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 0, &clean(0), Mode::V1);
        assert_eq!(policy_tick(&mut st, t, &clean(t), Mode::V1), Decision::Promote);
        let late = t + REMB_GRACE_MS + 1_000;
        let poor = Signals { remb: Some((1, late)), ..clean(late) };
        policy_tick(&mut st, late, &poor, Mode::V1);
        assert_eq!(policy_tick(&mut st, late + 1_000, &poor, Mode::V1), Decision::Demote(Cause::Remb));
        assert_eq!(st.backoff_ms, BACKOFF_INIT_MS, "★용량이 모자란 것은 무너진 것이 아니다");
    }

    #[test]
    fn the_grace_window_skips_remb_only() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        let t = BACKOFF_INIT_MS;
        policy_tick(&mut st, 0, &clean(0), Mode::V1);
        policy_tick(&mut st, t, &clean(t), Mode::V1);
        // 유예 안 — remb 가 나빠도 안 내려간다.
        let poor = Signals { remb: Some((1, t + 100)), ..clean(t + 100) };
        policy_tick(&mut st, t + 100, &poor, Mode::V1);
        assert_eq!(policy_tick(&mut st, t + 1_100, &poor, Mode::V1), Decision::Hold);
        assert_eq!(st.layer, Layer::H, "막 올린 것이 아직 안 찼을 뿐이다");
        // 같은 유예 안이라도 loss 는 내린다 — 건너뛰는 것은 remb 하나다.
        let lossy = Signals { loss: Some((50.0, t + 2_000)), ..clean(t + 2_000) };
        policy_tick(&mut st, t + 2_000, &lossy, Mode::V1);
        assert_eq!(policy_tick(&mut st, t + 3_000, &lossy, Mode::V1), Decision::Demote(Cause::Loss));
    }

    #[test]
    fn promotion_waits_for_the_backoff_to_pass() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        assert_eq!(policy_tick(&mut st, 0, &clean(0), Mode::V1), Decision::Hold);
        let almost = BACKOFF_INIT_MS - 1;
        assert_eq!(policy_tick(&mut st, almost, &clean(almost), Mode::V1), Decision::Hold, "깨끗한 것만으로는 안 올린다");
        assert_eq!(policy_tick(&mut st, BACKOFF_INIT_MS, &clean(BACKOFF_INIT_MS), Mode::V1), Decision::Promote);
    }

    #[test]
    fn a_dirty_tick_restarts_the_clean_run() {
        let mut st = State { layer: Layer::L, demoted_at: 0, ..State::default() };
        policy_tick(&mut st, 0, &clean(0), Mode::V1);
        let dirty = Signals { loss: Some((CLEAN_LOSS + 0.1, 5_000)), ..clean(5_000) };
        assert_eq!(policy_tick(&mut st, 5_000, &dirty, Mode::V1), Decision::Hold);
        // 5초를 이미 깨끗하게 보냈어도 여기서 다시 센다.
        assert_eq!(policy_tick(&mut st, 5_000 + BACKOFF_INIT_MS, &clean(5_000 + BACKOFF_INIT_MS), Mode::V1), Decision::Hold);
    }

    #[test]
    fn the_backoff_has_a_ceiling() {
        let mut st = State { layer: Layer::L, backoff_ms: BACKOFF_CAP_MS, ..State::default() };
        let t = BACKOFF_CAP_MS;
        policy_tick(&mut st, 0, &clean(0), Mode::V1);
        assert_eq!(policy_tick(&mut st, t, &clean(t), Mode::V1), Decision::Promote);
        let bad = Signals { drops: 1, ..clean(t + 100) };
        policy_tick(&mut st, t + 100, &bad, Mode::V1);
        policy_tick(&mut st, t + 1_100, &bad, Mode::V1);
        assert_eq!(st.backoff_ms, BACKOFF_CAP_MS, "천장을 넘지 않는다");
    }
}
