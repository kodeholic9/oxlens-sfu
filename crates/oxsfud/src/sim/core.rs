// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-SIM-MODES · 정:RULE-SIM-TICK · 정:IMPL-SIM-SWITCH · 정:CAND-SIM-PROBE-SHAPE · 정:RULE-CONC-SANS-IO · 연:RULE-PUB-RID-TO-SPATIAL · model: claude-opus-5-5
//! 시뮬캐스트 산수(순수) — 속도 제어(GCC §5.5 · libwebrtc AimdRateControl · LinkCapacityEstimator) · 지연 기울기(trendline) · 과사용 판정 · 레이어 판단 tick · 전환 기계.

use std::collections::VecDeque;

pub const START_BPS: u64 = 300_000;
pub const FLOOR_BPS: u64 = 30_000;
pub const RTT_MS: f64 = 200.0;
pub const PACKET_BYTES: f64 = 1200.0;
pub const FPS: f64 = 30.0;
pub const BETA: f64 = 0.85;
pub const DECREASE_GAP_MS: u64 = 300;

pub const LAYER_H_BPS: u64 = 1_650_000;
pub const LAYER_L_BPS: u64 = 250_000;
pub const HEADROOM: f64 = 1.2;
pub const LOSS_BAD: f64 = 0.08;
pub const LOSS_CLEAN: f64 = 0.02;
pub const NACK_BAD_PER_S: f64 = 20.0;
pub const DEMOTE_TICKS: u32 = 2;
pub const TICK_MS: u64 = 1000;
pub const BACKOFF_START_MS: u64 = 15_000;
pub const BACKOFF_MAX_MS: u64 = 60_000;
pub const COLLAPSE_MS: u64 = 5_000;
pub const REMB_GRACE_MS: u64 = 10_000;
pub const FRESH_REMB_MS: u64 = 5_000;
pub const FRESH_LOSS_MS: u64 = 6_000;
pub const PENDING_MS: u64 = 10_000;
pub const V2_STALE_MS: u64 = 1_000;
pub const PROBE_RUN_MS: u64 = 2_500;
pub const PROBE_JUDGE_MS: u64 = 4_000;
pub const PROBE_CHUNK_MS: u64 = 20;
pub const PROBE_GAIN: f64 = 1.35;
pub const PROBE_CAP_BPS: u64 = 3_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    Normal,
    Overuse,
    Underuse,
}

impl Usage {
    pub fn parse(s: &str) -> Option<Usage> {
        match s {
            "normal" => Some(Usage::Normal),
            "overuse" => Some(Usage::Overuse),
            "underuse" => Some(Usage::Underuse),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RateState {
    Hold,
    Increase,
    Decrease,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkCapacity {
    estimate_kbps: Option<f64>,
    deviation_kbps: f64,
}

impl Default for LinkCapacity {
    fn default() -> Self {
        LinkCapacity { estimate_kbps: None, deviation_kbps: 0.4 }
    }
}

impl LinkCapacity {
    fn dev(&self) -> f64 {
        self.estimate_kbps.map(|e| (self.deviation_kbps * e).sqrt()).unwrap_or(0.0)
    }

    pub fn upper_kbps(&self) -> f64 {
        self.estimate_kbps.map(|e| e + 3.0 * self.dev()).unwrap_or(f64::INFINITY)
    }

    pub fn lower_kbps(&self) -> f64 {
        self.estimate_kbps.map(|e| e - 3.0 * self.dev()).unwrap_or(0.0)
    }

    pub fn reset(&mut self) {
        self.estimate_kbps = None;
    }

    pub fn has(&self) -> bool {
        self.estimate_kbps.is_some()
    }

    pub fn kbps(&self) -> Option<f64> {
        self.estimate_kbps
    }

    pub fn on_overuse(&mut self, sample_kbps: f64) {
        let alpha = 0.05;
        let est = match self.estimate_kbps {
            None => sample_kbps,
            Some(e) => (1.0 - alpha) * e + alpha * sample_kbps,
        };
        self.estimate_kbps = Some(est);
        let norm = est.max(1.0);
        let err = est - sample_kbps;
        self.deviation_kbps = ((1.0 - alpha) * self.deviation_kbps + alpha * err * err / norm).clamp(0.4, 2.5);
    }
}

#[derive(Debug, Clone)]
pub struct Aimd {
    rate: u64,
    state: RateState,
    last_change_ms: Option<u64>,
    pub link: LinkCapacity,
    pub last_decrease_ms: Option<u64>,
}

impl Aimd {
    pub fn new(start_bps: u64) -> Aimd {
        Aimd { rate: start_bps, state: RateState::Hold, last_change_ms: None, link: LinkCapacity::default(), last_decrease_ms: None }
    }

    pub fn rate(&self) -> u64 {
        self.rate
    }

    pub fn near(&self) -> bool {
        self.link.has()
    }

    pub fn reset_rate(&mut self, bps: u64) {
        self.rate = bps.max(FLOOR_BPS);
        self.link.reset();
    }

    fn near_max_increase_bps_per_s(&self) -> f64 {
        let frame_bytes = self.rate as f64 / 8.0 / FPS;
        let packets = (frame_bytes / PACKET_BYTES).ceil().max(1.0);
        let avg_bits = frame_bytes / packets * 8.0;
        let response_s = 2.0 * (RTT_MS + 100.0) / 1000.0;
        (avg_bits / response_s).max(4000.0)
    }

    pub fn update(&mut self, usage: Usage, acked_bps: u64, now_ms: u64) -> u64 {
        match usage {
            Usage::Normal => {
                if self.state == RateState::Hold {
                    self.last_change_ms = Some(now_ms);
                    self.state = RateState::Increase;
                }
            }
            Usage::Overuse => self.state = RateState::Decrease,
            Usage::Underuse => self.state = RateState::Hold,
        }
        let acked = acked_bps as f64;
        let cur = self.rate as f64;
        let limit = 1.5 * acked + 10_000.0;
        let mut new_rate: Option<f64> = None;
        match self.state {
            RateState::Hold => {}
            RateState::Increase => {
                if acked / 1000.0 > self.link.upper_kbps() {
                    self.link.reset();
                }
                if cur < limit {
                    let elapsed_s = self.last_change_ms.map(|t| now_ms.saturating_sub(t) as f64 / 1000.0).unwrap_or(0.0);
                    let inc = if self.link.has() {
                        self.near_max_increase_bps_per_s() * elapsed_s
                    } else {
                        (cur * (1.08f64.powf(elapsed_s.min(1.0)) - 1.0)).max(1000.0)
                    };
                    new_rate = Some((cur + inc).min(limit));
                }
                self.last_change_ms = Some(now_ms);
            }
            RateState::Decrease => {
                let mut dec = acked * BETA;
                if dec > 5000.0 {
                    dec -= 5000.0;
                }
                if dec > cur
                    && let Some(e) = self.link.kbps()
                {
                    dec = BETA * e * 1000.0;
                }
                if dec < cur {
                    new_rate = Some(dec);
                }
                if acked / 1000.0 < self.link.lower_kbps() {
                    self.link.reset();
                }
                self.link.on_overuse(acked / 1000.0);
                self.state = RateState::Hold;
                self.last_change_ms = Some(now_ms);
                self.last_decrease_ms = Some(now_ms);
            }
        }
        let next = new_rate.map(|r| r as u64).unwrap_or(self.rate);
        self.rate = next.max(FLOOR_BPS);
        self.rate
    }
}

#[derive(Debug, Clone)]
pub struct Trendline {
    window: VecDeque<(f64, f64)>,
    acc_delay: f64,
    smoothed: f64,
    first_ms: Option<f64>,
    deltas: u32,
    pub prev_trend: f64,
}

impl Default for Trendline {
    fn default() -> Self {
        Trendline { window: VecDeque::new(), acc_delay: 0.0, smoothed: 0.0, first_ms: None, deltas: 0, prev_trend: 0.0 }
    }
}

pub const TRENDLINE_WINDOW: usize = 20;
pub const TRENDLINE_SMOOTHING: f64 = 0.9;
pub const TRENDLINE_GAIN: f64 = 4.0;

impl Trendline {
    pub fn update(&mut self, recv_delta_ms: f64, send_delta_ms: f64, arrival_ms: f64) -> Option<f64> {
        let delta = recv_delta_ms - send_delta_ms;
        self.deltas = (self.deltas + 1).min(1000);
        let first = *self.first_ms.get_or_insert(arrival_ms);
        self.acc_delay += delta;
        self.smoothed = TRENDLINE_SMOOTHING * self.smoothed + (1.0 - TRENDLINE_SMOOTHING) * self.acc_delay;
        self.window.push_back((arrival_ms - first, self.smoothed));
        if self.window.len() > TRENDLINE_WINDOW {
            self.window.pop_front();
        }
        if self.window.len() < TRENDLINE_WINDOW {
            return None;
        }
        let n = self.window.len() as f64;
        let mx = self.window.iter().map(|p| p.0).sum::<f64>() / n;
        let my = self.window.iter().map(|p| p.1).sum::<f64>() / n;
        let num: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let den: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        if den == 0.0 {
            return None;
        }
        let slope = num / den;
        let modified = (self.deltas.min(60) as f64) * slope * TRENDLINE_GAIN;
        Some(modified)
    }
}

#[derive(Debug, Clone)]
pub struct Overuse {
    threshold: f64,
    last_update_ms: Option<f64>,
    over_ms: f64,
    over_count: u32,
    prev_m: f64,
    pub usage: Usage,
}

impl Default for Overuse {
    fn default() -> Self {
        Overuse { threshold: 12.5, last_update_ms: None, over_ms: -1.0, over_count: 0, prev_m: 0.0, usage: Usage::Normal }
    }
}

impl Overuse {
    pub fn detect(&mut self, m: f64, ts_delta_ms: f64, now_ms: f64) -> Usage {
        if m > self.threshold {
            if self.over_ms < 0.0 {
                self.over_ms = ts_delta_ms / 2.0;
            } else {
                self.over_ms += ts_delta_ms;
            }
            self.over_count += 1;
            if self.over_ms > 10.0 && self.over_count > 1 && m >= self.prev_m {
                self.over_ms = 0.0;
                self.over_count = 0;
                self.usage = Usage::Overuse;
            }
        } else if m < -self.threshold {
            self.over_ms = -1.0;
            self.usage = Usage::Underuse;
        } else {
            self.over_ms = -1.0;
            self.usage = Usage::Normal;
        }
        self.prev_m = m;
        self.adapt(m, now_ms);
        self.usage
    }

    fn adapt(&mut self, m: f64, now_ms: f64) {
        let last = self.last_update_ms.unwrap_or(now_ms);
        if m.abs() > self.threshold + 15.0 {
            self.last_update_ms = Some(now_ms);
            return;
        }
        let k = if m.abs() < self.threshold { 0.039 } else { 0.0087 };
        let dt = (now_ms - last).min(100.0);
        self.threshold += k * (m.abs() - self.threshold) * dt;
        self.threshold = self.threshold.clamp(6.0, 600.0);
        self.last_update_ms = Some(now_ms);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    High,
    Low,
    Pause,
}

impl Layer {
    pub fn spatial(self) -> Option<u8> {
        match self {
            Layer::High => Some(1),
            Layer::Low => Some(0),
            Layer::Pause => None,
        }
    }

    pub fn rid(self) -> &'static str {
        match self {
            Layer::High => "h",
            Layer::Low => "l",
            Layer::Pause => "",
        }
    }

    pub fn from_spatial(s: u8) -> Layer {
        if s == 0 { Layer::Low } else { Layer::High }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Remb,
    Loss,
    Nack,
    Drop,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Remb => "remb",
            Reason::Loss => "loss",
            Reason::Nack => "nack",
            Reason::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Signals {
    pub estimate_bps: Option<u64>,
    pub estimate_at_ms: u64,
    pub loss: Option<f64>,
    pub loss_at_ms: u64,
    pub nack_per_s: f64,
    pub drops: u64,
    pub overusing: bool,
    pub v2_fresh: bool,
    pub tracks: u32,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub layer: Layer,
    pub backoff_ms: u64,
    pub last_demote_ms: Option<u64>,
    pub last_promote_ms: Option<u64>,
    streak: [u32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Stay,
    Demote(Reason),
    Promote,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { layer: Layer::High, backoff_ms: BACKOFF_START_MS, last_demote_ms: None, last_promote_ms: None, streak: [0; 4] }
    }
}

impl Policy {
    pub fn demand_bps(tracks: u32, layer: Layer) -> u64 {
        let per = match layer {
            Layer::High => LAYER_H_BPS,
            Layer::Low | Layer::Pause => LAYER_L_BPS,
        };
        per * tracks.max(1) as u64
    }

    pub fn probe_due(&self, now_ms: u64, s: &Signals) -> Option<u64> {
        if self.layer != Layer::Low || !s.v2_fresh {
            return None;
        }
        let fresh_loss = s.loss.filter(|_| now_ms.saturating_sub(s.loss_at_ms) <= FRESH_LOSS_MS);
        let clean = fresh_loss.map(|l| l <= LOSS_CLEAN).unwrap_or(true) && s.drops == 0 && !s.overusing;
        let waited = self.last_demote_ms.map(|t| now_ms.saturating_sub(t) >= self.backoff_ms).unwrap_or(true);
        let need = (Policy::demand_bps(s.tracks, Layer::High) as f64 * HEADROOM) as u64;
        let short = s.estimate_bps.map(|e| e < need).unwrap_or(true);
        (clean && waited && short).then(|| ((need as f64 * PROBE_GAIN) as u64).min(PROBE_CAP_BPS))
    }

    pub fn tick(&mut self, now_ms: u64, s: &Signals) -> Verdict {
        let fresh_est = s.estimate_bps.filter(|_| now_ms.saturating_sub(s.estimate_at_ms) <= FRESH_REMB_MS);
        let fresh_loss = s.loss.filter(|_| now_ms.saturating_sub(s.loss_at_ms) <= FRESH_LOSS_MS);
        let in_grace = self.last_promote_ms.map(|t| now_ms.saturating_sub(t) < REMB_GRACE_MS).unwrap_or(false);
        let need = (Policy::demand_bps(s.tracks, Layer::High) as f64 * HEADROOM) as u64;
        let conds = [
            !in_grace && fresh_est.map(|e| e < need).unwrap_or(false),
            fresh_loss.map(|l| l > LOSS_BAD).unwrap_or(false),
            s.nack_per_s > NACK_BAD_PER_S,
            s.drops > 0,
        ];
        for (i, c) in conds.iter().enumerate() {
            if *c {
                self.streak[i] += 1;
            } else {
                self.streak[i] = 0;
            }
        }
        if self.layer == Layer::High {
            let reasons = [Reason::Remb, Reason::Loss, Reason::Nack, Reason::Drop];
            for (i, r) in reasons.iter().enumerate() {
                if self.streak[i] >= DEMOTE_TICKS {
                    let collapsed = self.last_promote_ms.map(|t| now_ms.saturating_sub(t) < COLLAPSE_MS).unwrap_or(false);
                    if collapsed {
                        self.backoff_ms = (self.backoff_ms * 2).min(BACKOFF_MAX_MS);
                    }
                    self.layer = Layer::Low;
                    self.last_demote_ms = Some(now_ms);
                    self.streak = [0; 4];
                    return Verdict::Demote(*r);
                }
            }
            return Verdict::Stay;
        }
        if self.layer == Layer::Low {
            let clean = fresh_loss.map(|l| l <= LOSS_CLEAN).unwrap_or(true) && s.drops == 0 && !s.overusing;
            let waited = self.last_demote_ms.map(|t| now_ms.saturating_sub(t) >= self.backoff_ms).unwrap_or(true);
            let room = fresh_est.map(|e| e >= need).unwrap_or(fresh_est.is_none() && !s.v2_fresh);
            if clean && waited && room {
                self.layer = Layer::High;
                self.last_promote_ms = Some(now_ms);
                return Verdict::Promote;
            }
            if !clean {
                self.last_demote_ms = Some(now_ms);
            }
        }
        Verdict::Stay
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Forward,
    Drop,
    Switched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    pub current: Option<Layer>,
    pub target: Layer,
    pub pending_since: Option<u64>,
    pub other_since: Option<u64>,
}

impl Switch {
    pub fn new(target: Layer) -> Switch {
        Switch { current: None, target, pending_since: None, other_since: None }
    }

    pub fn retarget(&mut self, target: Layer, now_ms: u64) -> bool {
        if self.target == target {
            return false;
        }
        self.target = target;
        if Some(target) != self.current && target != Layer::Pause {
            self.pending_since = Some(now_ms);
            return true;
        }
        self.pending_since = None;
        false
    }

    pub fn step(&mut self, layer: Layer, keyframe: bool, now_ms: u64) -> Step {
        if let Some(p) = self.pending_since
            && now_ms.saturating_sub(p) >= PENDING_MS
        {
            self.pending_since = None;
            if let Some(c) = self.current {
                self.target = c;
            }
        }
        if self.target == Layer::Pause {
            return Step::Drop;
        }
        if layer == self.target && Some(layer) != self.current {
            if keyframe {
                self.current = Some(layer);
                self.pending_since = None;
                self.other_since = None;
                return Step::Switched;
            }
            return Step::Drop;
        }
        if Some(layer) == self.current {
            self.other_since = None;
            return Step::Forward;
        }
        if self.current == Some(Layer::High) && layer == Layer::Low {
            let since = *self.other_since.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= 1000 && self.target == Layer::High {
                self.target = Layer::Low;
                self.pending_since = Some(now_ms);
            }
        }
        Step::Drop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_waits_for_target_keyframe() {
        let mut s = Switch::new(Layer::High);
        assert_eq!(s.step(Layer::High, false, 0), Step::Drop);
        assert_eq!(s.step(Layer::High, true, 10), Step::Switched);
        assert_eq!(s.step(Layer::High, false, 20), Step::Forward);
        assert!(s.retarget(Layer::Low, 30));
        assert_eq!(s.step(Layer::High, false, 40), Step::Forward, "target 키프레임 전까지 current 를 릴레이한다");
        assert_eq!(s.step(Layer::Low, true, 50), Step::Switched);
        assert!(s.retarget(Layer::High, 60));
        assert_eq!(s.step(Layer::Low, false, 60 + PENDING_MS), Step::Forward, "pending 만료 10s 면 현재 층으로");
    }

    #[test]
    fn policy_demotes_after_two_ticks_and_backs_off() {
        let mut p = Policy::default();
        let bad = Signals { loss: Some(0.2), loss_at_ms: 0, tracks: 1, ..Signals::default() };
        assert_eq!(p.tick(0, &bad), Verdict::Stay);
        assert_eq!(p.tick(1000, &Signals { loss_at_ms: 1000, ..bad }), Verdict::Demote(Reason::Loss));
        let good = Signals { loss: Some(0.0), loss_at_ms: 2000, tracks: 1, ..Signals::default() };
        assert_eq!(p.tick(2000, &good), Verdict::Stay);
        assert_eq!(p.tick(1000 + BACKOFF_START_MS, &Signals { loss_at_ms: 1000 + BACKOFF_START_MS, ..good }), Verdict::Promote);
    }

    #[test]
    fn abs_layers() {
        assert_eq!(Layer::from_spatial(0), Layer::Low);
        assert_eq!(Layer::High.spatial(), Some(1));
    }
}
