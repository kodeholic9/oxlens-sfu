// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · 정:IMPL-SIM-SWITCH · 정:CAND-SIM-PROBE-SHAPE · model: claude-opus-5-5
//! 자동 판단 산수(순수) — GCC 속도 제어(AIMD · 링크 용량 추정) · trendline 지연 기울기 · 적응 문턱 과사용 판정 · 레이어 정책 tick. 시각 · 신호 전부 인자다.

pub const START_BPS: f64 = 300_000.0;
pub const FLOOR_BPS: f64 = 30_000.0;
pub const RTT_MS: f64 = 200.0;
pub const PACKET_BYTES: f64 = 1_200.0;
pub const FPS: f64 = 30.0;
pub const BETA: f64 = 0.85;

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

#[derive(Debug, Clone, Copy)]
pub struct LinkCapacity {
    estimate_kbps: Option<f64>,
    deviation_kbps: f64,
}

impl Default for LinkCapacity {
    fn default() -> LinkCapacity {
        LinkCapacity { estimate_kbps: None, deviation_kbps: 0.4 }
    }
}

impl LinkCapacity {
    fn dev(&self) -> f64 {
        self.estimate_kbps.map(|e| (self.deviation_kbps * e).sqrt()).unwrap_or(0.0)
    }

    fn upper_kbps(&self) -> f64 {
        self.estimate_kbps.map(|e| e + 3.0 * self.dev()).unwrap_or(f64::INFINITY)
    }

    fn lower_kbps(&self) -> f64 {
        self.estimate_kbps.map(|e| (e - 3.0 * self.dev()).max(0.0)).unwrap_or(0.0)
    }

    fn update(&mut self, sample_kbps: f64, alpha: f64) {
        let e = match self.estimate_kbps {
            None => sample_kbps,
            Some(e) => (1.0 - alpha) * e + alpha * sample_kbps,
        };
        self.estimate_kbps = Some(e);
        let norm = e.max(1.0);
        let err = e - sample_kbps;
        self.deviation_kbps = ((1.0 - alpha) * self.deviation_kbps + alpha * err * err / norm).clamp(0.4, 2.5);
    }

    pub fn has_estimate(&self) -> bool {
        self.estimate_kbps.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Aimd {
    bitrate: f64,
    state: RateState,
    last_change_ms: Option<u64>,
    link: LinkCapacity,
}

impl Aimd {
    pub fn new(start_bps: f64) -> Aimd {
        Aimd { bitrate: start_bps, state: RateState::Hold, last_change_ms: None, link: LinkCapacity::default() }
    }

    pub fn bitrate_bps(&self) -> u64 {
        self.bitrate as u64
    }

    pub fn near_max(&self) -> bool {
        self.link.has_estimate()
    }

    fn near_max_increase_bps_per_s(&self) -> f64 {
        let frame_bits = self.bitrate / FPS;
        let packets = (frame_bits / 8.0 / PACKET_BYTES).ceil().max(1.0);
        let avg_packet_bits = frame_bits / packets;
        let response_s = 2.0 * (RTT_MS + 100.0) / 1_000.0;
        (avg_packet_bits / response_s).max(4_000.0)
    }

    pub fn update(&mut self, usage: Usage, acked_bps: f64, now_ms: u64) -> u64 {
        match usage {
            Usage::Normal => {
                if self.state == RateState::Hold {
                    self.last_change_ms = Some(now_ms);
                    self.state = RateState::Increase;
                }
            }
            Usage::Overuse => {
                if self.state != RateState::Decrease {
                    self.state = RateState::Decrease;
                }
            }
            Usage::Underuse => self.state = RateState::Hold,
        }
        let acked_kbps = acked_bps / 1_000.0;
        let mut next = self.bitrate;
        match self.state {
            RateState::Hold => {}
            RateState::Increase => {
                if acked_kbps > self.link.upper_kbps() {
                    self.link.estimate_kbps = None;
                }
                let limit = 1.5 * acked_bps + 10_000.0;
                if self.bitrate < limit {
                    let dt_s = self.last_change_ms.map(|t| now_ms.saturating_sub(t) as f64 / 1_000.0).unwrap_or(0.0);
                    let inc = if self.link.has_estimate() {
                        self.near_max_increase_bps_per_s() * dt_s
                    } else {
                        let alpha = 1.08_f64.powf(dt_s.min(1.0));
                        (self.bitrate * (alpha - 1.0)).max(1_000.0)
                    };
                    next = (self.bitrate + inc).min(limit);
                }
                self.last_change_ms = Some(now_ms);
            }
            RateState::Decrease => {
                let mut dec = acked_bps * BETA;
                if dec > 5_000.0 {
                    dec -= 5_000.0;
                }
                if dec > self.bitrate
                    && let Some(e) = self.link.estimate_kbps
                {
                    dec = BETA * e * 1_000.0;
                }
                if dec < self.bitrate {
                    next = dec;
                }
                if acked_kbps < self.link.lower_kbps() {
                    self.link.estimate_kbps = None;
                }
                self.link.update(acked_kbps, 0.05);
                self.state = RateState::Hold;
                self.last_change_ms = Some(now_ms);
            }
        }
        self.bitrate = next.max(FLOOR_BPS).floor();
        self.bitrate as u64
    }
}

pub const TREND_WINDOW: usize = 20;
pub const TREND_SMOOTHING: f64 = 0.9;
pub const TREND_GAIN: f64 = 4.0;
pub const THRESHOLD_START_MS: f64 = 12.5;
pub const K_UP: f64 = 0.0087;
pub const K_DOWN: f64 = 0.039;
pub const OVERUSE_TIME_MS: f64 = 10.0;
pub const BURST_MS: i64 = 5;

#[derive(Debug, Clone)]
pub struct Trendline {
    num_deltas: usize,
    accumulated_delay: f64,
    smoothed_delay: f64,
    first_arrival_ms: Option<f64>,
    history: std::collections::VecDeque<(f64, f64)>,
    prev_trend: f64,
    threshold: f64,
    last_update_ms: Option<f64>,
    time_over: f64,
    overuse_count: u32,
    usage: Usage,
}

impl Default for Trendline {
    fn default() -> Trendline {
        Trendline {
            num_deltas: 0,
            accumulated_delay: 0.0,
            smoothed_delay: 0.0,
            first_arrival_ms: None,
            history: Default::default(),
            prev_trend: 0.0,
            threshold: THRESHOLD_START_MS,
            last_update_ms: None,
            time_over: -1.0,
            overuse_count: 0,
            usage: Usage::Normal,
        }
    }
}

fn slope(h: &std::collections::VecDeque<(f64, f64)>) -> Option<f64> {
    let n = h.len() as f64;
    if n < 2.0 {
        return None;
    }
    let mx = h.iter().map(|p| p.0).sum::<f64>() / n;
    let my = h.iter().map(|p| p.1).sum::<f64>() / n;
    let num: f64 = h.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let den: f64 = h.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    if den == 0.0 { None } else { Some(num / den) }
}

impl Trendline {
    pub fn new() -> Trendline {
        Trendline::default()
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    pub fn update(&mut self, recv_delta_ms: f64, send_delta_ms: f64, arrival_ms: f64) -> Usage {
        let delta = recv_delta_ms - send_delta_ms;
        self.num_deltas = (self.num_deltas + 1).min(1_000);
        let first = *self.first_arrival_ms.get_or_insert(arrival_ms);
        self.accumulated_delay += delta;
        self.smoothed_delay = TREND_SMOOTHING * self.smoothed_delay + (1.0 - TREND_SMOOTHING) * self.accumulated_delay;
        self.history.push_back((arrival_ms - first, self.smoothed_delay));
        if self.history.len() > TREND_WINDOW {
            self.history.pop_front();
        }
        let mut trend = self.prev_trend;
        if self.history.len() == TREND_WINDOW
            && let Some(s) = slope(&self.history)
        {
            trend = s;
        }
        self.detect(trend, send_delta_ms, arrival_ms);
        self.usage
    }

    fn detect(&mut self, trend: f64, ts_delta_ms: f64, now_ms: f64) {
        if self.num_deltas < 2 {
            return;
        }
        let modified = (self.num_deltas.min(60) as f64) * trend * TREND_GAIN;
        if modified > self.threshold {
            if self.time_over < 0.0 {
                self.time_over = ts_delta_ms / 2.0;
            } else {
                self.time_over += ts_delta_ms;
            }
            self.overuse_count += 1;
            if self.time_over > OVERUSE_TIME_MS && self.overuse_count > 1 && trend >= self.prev_trend {
                self.time_over = 0.0;
                self.overuse_count = 0;
                self.usage = Usage::Overuse;
            }
        } else if modified < -self.threshold {
            self.time_over = -1.0;
            self.overuse_count = 0;
            self.usage = Usage::Underuse;
        } else {
            self.time_over = -1.0;
            self.overuse_count = 0;
            self.usage = Usage::Normal;
        }
        self.prev_trend = trend;
        self.adapt(modified, now_ms);
    }

    fn adapt(&mut self, modified: f64, now_ms: f64) {
        let last = *self.last_update_ms.get_or_insert(now_ms);
        if modified.abs() > self.threshold + 15.0 {
            self.last_update_ms = Some(now_ms);
            return;
        }
        let k = if modified.abs() < self.threshold { K_DOWN } else { K_UP };
        let dt = (now_ms - last).min(100.0);
        self.threshold += k * (modified.abs() - self.threshold) * dt;
        self.threshold = self.threshold.clamp(6.0, 600.0);
        self.last_update_ms = Some(now_ms);
    }
}

pub const LAYER_H_BPS: f64 = 1_650_000.0;
pub const LAYER_L_BPS: f64 = 250_000.0;
pub const HEADROOM: f64 = 1.2;
pub const LOSS_DEMOTE: f64 = 8.0;
pub const LOSS_CLEAN: f64 = 2.0;
pub const NACK_DEMOTE_PER_S: f64 = 20.0;
pub const DEMOTE_TICKS: u32 = 2;
pub const TICK_MS: u64 = 1_000;
pub const BACKOFF_START_MS: u64 = 15_000;
pub const BACKOFF_MAX_MS: u64 = 60_000;
pub const COLLAPSE_WINDOW_MS: u64 = 5_000;
pub const REMB_GRACE_MS: u64 = 10_000;
pub const FRESH_REMB_MS: u64 = 5_000;
pub const FRESH_LOSS_MS: u64 = 6_000;
pub const PENDING_EXPIRE_MS: u64 = 10_000;
pub const PLI_THROTTLE_H_MS: u64 = 300;
pub const PLI_THROTTLE_L_MS: u64 = 100;
pub const PROBE_DURATION_MS: u64 = 2_500;
pub const PROBE_JUDGE_MS: u64 = 4_000;
pub const PROBE_CHUNK_MS: u64 = 20;
pub const PROBE_FACTOR: f64 = 1.35;
pub const PROBE_MAX_BPS: f64 = 3_000_000.0;
pub const V2_STALE_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    High,
    Low,
    Pause,
}

impl Layer {
    pub fn rid(self) -> Option<&'static str> {
        match self {
            Layer::High => Some("h"),
            Layer::Low => Some("l"),
            Layer::Pause => None,
        }
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

#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    pub estimate_bps: Option<f64>,
    pub estimate_at_ms: u64,
    pub loss_pct: Option<f64>,
    pub loss_at_ms: u64,
    pub nack_per_s: f64,
    pub drops: u64,
    pub overusing: bool,
    pub streams: u32,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub layer: Layer,
    bad: [u32; 4],
    backoff_ms: u64,
    last_demote_ms: u64,
    last_promote_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Stay,
    Demote(Reason),
    Promote,
}

impl Policy {
    pub fn new(now_ms: u64) -> Policy {
        Policy { layer: Layer::High, bad: [0; 4], backoff_ms: BACKOFF_START_MS, last_demote_ms: now_ms, last_promote_ms: None }
    }

    pub fn tick(&mut self, now_ms: u64, s: &Signals) -> Decision {
        let demand = f64::from(s.streams.max(1)) * LAYER_H_BPS * HEADROOM;
        let in_grace = self.last_promote_ms.is_some_and(|t| now_ms.saturating_sub(t) < REMB_GRACE_MS);
        let fresh_est = s.estimate_bps.filter(|_| now_ms.saturating_sub(s.estimate_at_ms) <= FRESH_REMB_MS);
        let fresh_loss = s.loss_pct.filter(|_| now_ms.saturating_sub(s.loss_at_ms) <= FRESH_LOSS_MS);
        let conds = [
            !in_grace && fresh_est.is_some_and(|e| e < demand),
            fresh_loss.is_some_and(|l| l >= LOSS_DEMOTE),
            s.nack_per_s > NACK_DEMOTE_PER_S,
            s.drops > 0,
        ];
        for (i, c) in conds.iter().enumerate() {
            self.bad[i] = if *c { self.bad[i] + 1 } else { 0 };
        }
        if self.layer == Layer::High {
            let reasons = [Reason::Remb, Reason::Loss, Reason::Nack, Reason::Drop];
            for (i, r) in reasons.iter().enumerate() {
                if self.bad[i] >= DEMOTE_TICKS {
                    if self.last_promote_ms.is_some_and(|t| now_ms.saturating_sub(t) < COLLAPSE_WINDOW_MS) {
                        self.backoff_ms = (self.backoff_ms * 2).min(BACKOFF_MAX_MS);
                    }
                    self.layer = Layer::Low;
                    self.last_demote_ms = now_ms;
                    self.bad = [0; 4];
                    return Decision::Demote(*r);
                }
            }
            return Decision::Stay;
        }
        let clean = fresh_loss.is_none_or(|l| l <= LOSS_CLEAN) && !s.overusing && s.drops == 0;
        let enough = fresh_est.is_none_or(|e| e >= demand);
        if self.layer == Layer::Low && clean && enough && now_ms.saturating_sub(self.last_demote_ms) >= self.backoff_ms {
            self.layer = Layer::High;
            self.last_promote_ms = Some(now_ms);
            return Decision::Promote;
        }
        Decision::Stay
    }

    pub fn restart_clean_window(&mut self, now_ms: u64) {
        self.last_demote_ms = now_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_first_increase() {
        let mut a = Aimd::new(START_BPS);
        assert_eq!(a.update(Usage::Normal, 300_000.0, 0), 301_000);
        let mut b = Aimd::new(100_000.0);
        assert_eq!(b.update(Usage::Overuse, 20_000.0, 0), 30_000);
        assert!(b.near_max());
    }

    #[test]
    fn policy_demotes_after_two_bad_ticks() {
        let mut p = Policy::new(0);
        let s = Signals { loss_pct: Some(10.0), loss_at_ms: 1_000, streams: 1, ..Signals::default() };
        assert_eq!(p.tick(1_000, &s), Decision::Stay);
        assert_eq!(p.tick(2_000, &Signals { loss_at_ms: 2_000, ..s }), Decision::Demote(Reason::Loss));
    }
}
