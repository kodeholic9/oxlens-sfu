// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-SIM-TICK 정:RULE-SIM-MODES 정:RULE-CONC-SANS-IO 정:CAND-SIM-PROBE-SHAPE · model: claude-opus-5-5
//! 자동 레이어 산수 — GCC(묶음 5ms · trendline · 적응 문턱 · 과사용 판정 · AIMD + 링크 용량 추정) · 강등 사유 넷 · 승격(clean 유지 + backoff) · 프로브 판정. 시계는 인자다.

use std::collections::VecDeque;

pub const LAYER_H_BPS: f64 = 1_650_000.0;
pub const LAYER_L_BPS: f64 = 250_000.0;
pub const HEADROOM: f64 = 1.2;
pub const LOSS_BAD: f64 = 8.0;
pub const LOSS_CLEAN: f64 = 2.0;
pub const NACK_BAD_PER_S: f64 = 20.0;
pub const BAD_TICKS: u32 = 2;
pub const TICK_MS: u64 = 1000;
pub const BACKOFF_INIT_MS: u64 = 15_000;
pub const BACKOFF_CAP_MS: u64 = 60_000;
pub const COLLAPSE_MS: u64 = 5_000;
pub const PROMOTE_REMB_GRACE_MS: u64 = 10_000;
pub const REMB_FRESH_MS: u64 = 5_000;
pub const LOSS_FRESH_MS: u64 = 6_000;
pub const PENDING_EXPIRE_MS: u64 = 10_000;
pub const PLI_THROTTLE_H_MS: u64 = 300;
pub const PLI_THROTTLE_L_MS: u64 = 100;
pub const PROBE_DURATION_MS: u64 = 2_500;
pub const PROBE_JUDGE_MS: u64 = 4_000;
pub const PROBE_CHUNK_MS: u64 = 20;
pub const PROBE_GAIN: f64 = 1.35;
pub const PROBE_CAP_BPS: f64 = 3_000_000.0;
pub const PROBE_PAD_BYTES: usize = 224;
pub const V2_STALE_MS: u64 = 1_000;
pub const START_BPS: f64 = 300_000.0;
pub const FLOOR_BPS: f64 = 30_000.0;
pub const DECREASE_GAP_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    Normal,
    Overuse,
    Underuse,
}

impl Usage {
    pub fn parse(s: &str) -> Usage {
        match s {
            "overuse" => Usage::Overuse,
            "underuse" => Usage::Underuse,
            _ => Usage::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RcState {
    Hold,
    Increase,
    Decrease,
}

#[derive(Debug, Clone, Copy)]
struct LinkCap {
    est_kbps: f64,
    dev: f64,
}

impl LinkCap {
    fn std(&self) -> f64 {
        (self.dev * self.est_kbps).sqrt()
    }
    fn upper(&self) -> f64 {
        self.est_kbps + 3.0 * self.std()
    }
    fn lower(&self) -> f64 {
        self.est_kbps - 3.0 * self.std()
    }
}

#[derive(Debug, Clone)]
pub struct Aimd {
    rate: f64,
    state: RcState,
    last_ms: Option<u64>,
    cap: Option<LinkCap>,
    rtt_ms: f64,
}

impl Aimd {
    pub fn new(start_bps: f64) -> Aimd {
        Aimd { rate: start_bps, state: RcState::Hold, last_ms: None, cap: None, rtt_ms: 200.0 }
    }

    pub fn rate_bps(&self) -> u64 {
        self.rate as u64
    }

    pub fn near(&self) -> bool {
        self.cap.is_some()
    }

    fn cap_update(&mut self, sample_kbps: f64) {
        const A: f64 = 0.05;
        let c = match self.cap {
            None => LinkCap { est_kbps: sample_kbps, dev: 0.4 },
            Some(c) => LinkCap { est_kbps: (1.0 - A) * c.est_kbps + A * sample_kbps, dev: c.dev },
        };
        let norm = c.est_kbps.max(1.0);
        let err = c.est_kbps - sample_kbps;
        let dev = ((1.0 - A) * c.dev + A * err * err / norm).clamp(0.4, 2.5);
        self.cap = Some(LinkCap { est_kbps: c.est_kbps, dev });
    }

    fn additive_bps_per_s(&self) -> f64 {
        let frame_bytes = self.rate / 30.0 / 8.0;
        let packets = (frame_bytes / 1200.0).ceil().max(1.0);
        let avg_packet_bits = frame_bytes / packets * 8.0;
        let response_s = 2.0 * (self.rtt_ms + 100.0) / 1000.0;
        (avg_packet_bits / response_s).max(4000.0)
    }

    pub fn update(&mut self, at_ms: u64, usage: Usage, acked_bps: f64) -> (u64, bool) {
        let elapsed_s = self.last_ms.map(|l| at_ms.saturating_sub(l) as f64 / 1000.0).unwrap_or(0.0);
        let prev = self.state;
        self.state = match usage {
            Usage::Overuse => RcState::Decrease,
            Usage::Underuse => RcState::Hold,
            Usage::Normal => match self.state {
                RcState::Hold => RcState::Increase,
                s => s,
            },
        };
        let acked_kbps = acked_bps / 1000.0;
        match self.state {
            RcState::Hold => {}
            RcState::Increase => {
                if let Some(c) = self.cap
                    && acked_kbps > c.upper()
                {
                    self.cap = None;
                }
                let elapsed = if prev == RcState::Hold { 0.0 } else { elapsed_s };
                let limit = 1.5 * acked_bps + 10_000.0;
                if self.rate < limit {
                    let inc = match self.cap {
                        Some(_) => self.additive_bps_per_s() * elapsed,
                        None => (self.rate * (1.08f64.powf(elapsed.min(1.0)) - 1.0)).max(1000.0),
                    };
                    self.rate = (self.rate + inc).min(limit);
                }
            }
            RcState::Decrease => {
                let mut dec = 0.85 * acked_bps;
                if dec > 5000.0 {
                    dec -= 5000.0;
                }
                if dec > self.rate
                    && let Some(c) = self.cap
                {
                    dec = 0.85 * c.est_kbps * 1000.0;
                }
                if dec < self.rate {
                    self.rate = dec;
                }
                if let Some(c) = self.cap
                    && acked_kbps < c.lower()
                {
                    self.cap = None;
                }
                self.cap_update(acked_kbps);
                self.state = RcState::Hold;
            }
        }
        self.rate = self.rate.max(FLOOR_BPS).floor();
        self.last_ms = Some(at_ms);
        (self.rate as u64, self.cap.is_some())
    }
}

#[derive(Debug, Clone)]
pub struct Trendline {
    window: VecDeque<(f64, f64)>,
    acc_delay: f64,
    smoothed: f64,
    first_ms: Option<f64>,
    num_deltas: u32,
    threshold: f64,
    last_threshold_ms: Option<f64>,
    overuse_ms: f64,
    overuse_count: u32,
    prev_trend: f64,
    prev_m: f64,
    pub usage: Usage,
}

impl Default for Trendline {
    fn default() -> Self {
        Trendline {
            window: VecDeque::new(),
            acc_delay: 0.0,
            smoothed: 0.0,
            first_ms: None,
            num_deltas: 0,
            threshold: 12.5,
            last_threshold_ms: None,
            overuse_ms: -1.0,
            overuse_count: 0,
            prev_trend: 0.0,
            prev_m: 0.0,
            usage: Usage::Normal,
        }
    }
}

impl Trendline {
    const WINDOW: usize = 20;
    const SMOOTHING: f64 = 0.9;
    const GAIN: f64 = 4.0;
    const K_UP: f64 = 0.0087;
    const K_DOWN: f64 = 0.039;
    const OVERUSE_TIME_MS: f64 = 10.0;

    pub fn update(&mut self, recv_delta_ms: f64, send_delta_ms: f64, arrival_ms: f64) -> Usage {
        let delay = recv_delta_ms - send_delta_ms;
        self.num_deltas = (self.num_deltas + 1).min(1000);
        let first = *self.first_ms.get_or_insert(arrival_ms);
        self.acc_delay += delay;
        self.smoothed = Self::SMOOTHING * self.smoothed + (1.0 - Self::SMOOTHING) * self.acc_delay;
        self.window.push_back((arrival_ms - first, self.smoothed));
        if self.window.len() > Self::WINDOW {
            self.window.pop_front();
        }
        let mut trend = self.prev_trend;
        if self.window.len() == Self::WINDOW {
            let n = self.window.len() as f64;
            let mx = self.window.iter().map(|p| p.0).sum::<f64>() / n;
            let my = self.window.iter().map(|p| p.1).sum::<f64>() / n;
            let num: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            let den: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
            if den != 0.0 {
                trend = num / den;
            }
        }
        self.detect(trend, send_delta_ms, arrival_ms);
        self.prev_trend = trend;
        self.usage
    }

    fn detect(&mut self, trend: f64, ts_delta_ms: f64, now_ms: f64) {
        if self.num_deltas < 2 {
            self.usage = Usage::Normal;
            return;
        }
        let m = (self.num_deltas.min(60) as f64) * trend * Self::GAIN;
        if m > self.threshold {
            if self.overuse_ms < 0.0 {
                self.overuse_ms = ts_delta_ms / 2.0;
            } else {
                self.overuse_ms += ts_delta_ms;
            }
            self.overuse_count += 1;
            if self.overuse_ms > Self::OVERUSE_TIME_MS && self.overuse_count > 1 && trend >= self.prev_trend {
                self.overuse_ms = 0.0;
                self.overuse_count = 0;
                self.usage = Usage::Overuse;
            }
        } else if m < -self.threshold {
            self.overuse_ms = -1.0;
            self.overuse_count = 0;
            self.usage = Usage::Underuse;
        } else {
            self.overuse_ms = -1.0;
            self.overuse_count = 0;
            self.usage = Usage::Normal;
        }
        self.prev_m = m;
        self.adapt(m, now_ms);
    }

    fn adapt(&mut self, m: f64, now_ms: f64) {
        let last = *self.last_threshold_ms.get_or_insert(now_ms);
        if m.abs() > self.threshold + 15.0 {
            self.last_threshold_ms = Some(now_ms);
            return;
        }
        let k = if m.abs() < self.threshold { Self::K_DOWN } else { Self::K_UP };
        let dt = (now_ms - last).min(100.0);
        self.threshold = (self.threshold + k * (m.abs() - self.threshold) * dt).clamp(6.0, 600.0);
        self.last_threshold_ms = Some(now_ms);
    }
}

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

#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    pub estimate_bps: Option<(f64, u64)>,
    pub overuse: bool,
    pub loss_pct: Option<(f64, u64)>,
    pub nack_per_s: f64,
    pub drops: u64,
    pub demand_bps: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Stay,
    Demote(Reason),
    Promote,
    ProbeStart { rate_bps: f64 },
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub layer: Layer,
    bad: Option<(Reason, u32)>,
    clean_since: Option<u64>,
    last_demote: Option<u64>,
    last_promote: Option<u64>,
    backoff_ms: u64,
    pub probing: Option<(u64, f64)>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { layer: Layer::High, bad: None, clean_since: None, last_demote: None, last_promote: None, backoff_ms: BACKOFF_INIT_MS, probing: None }
    }
}

impl Policy {
    fn fresh(v: Option<(f64, u64)>, now: u64, ttl: u64) -> Option<f64> {
        v.filter(|(_, at)| now.saturating_sub(*at) <= ttl).map(|(x, _)| x)
    }

    pub fn tick(&mut self, now: u64, s: &Signals) -> Decision {
        let est = Self::fresh(s.estimate_bps, now, REMB_FRESH_MS);
        let loss = Self::fresh(s.loss_pct, now, LOSS_FRESH_MS);
        let need = s.demand_bps * HEADROOM;
        let in_grace = self.last_promote.is_some_and(|p| now.saturating_sub(p) < PROMOTE_REMB_GRACE_MS);
        let reason = if !in_grace && est.is_some_and(|e| e < need) && self.layer == Layer::High {
            Some(Reason::Remb)
        } else if loss.is_some_and(|l| l >= LOSS_BAD) {
            Some(Reason::Loss)
        } else if s.nack_per_s > NACK_BAD_PER_S {
            Some(Reason::Nack)
        } else if s.drops > 0 {
            Some(Reason::Drop)
        } else {
            None
        };
        let clean = loss.is_none_or(|l| l <= LOSS_CLEAN) && !s.overuse && reason.is_none();
        if self.layer == Layer::High {
            if let Some(r) = reason {
                let n = match self.bad {
                    Some((pr, c)) if pr == r => c + 1,
                    _ => 1,
                };
                self.bad = Some((r, n));
                if n >= BAD_TICKS {
                    self.bad = None;
                    if self.last_promote.is_some_and(|p| now.saturating_sub(p) < COLLAPSE_MS) {
                        self.backoff_ms = (self.backoff_ms * 2).min(BACKOFF_CAP_MS);
                    }
                    self.layer = Layer::Low;
                    self.last_demote = Some(now);
                    self.clean_since = None;
                    return Decision::Demote(r);
                }
            } else {
                self.bad = None;
            }
            return Decision::Stay;
        }
        if !clean {
            self.clean_since = None;
            return Decision::Stay;
        }
        let since = *self.clean_since.get_or_insert(now);
        let backoff_ok = self.last_demote.is_none_or(|d| now.saturating_sub(d) >= self.backoff_ms);
        if !backoff_ok || now.saturating_sub(since) < TICK_MS {
            return Decision::Stay;
        }
        if est.is_some_and(|e| e >= need) || est.is_none() {
            self.layer = Layer::High;
            self.last_promote = Some(now);
            self.clean_since = None;
            self.probing = None;
            return Decision::Promote;
        }
        if self.probing.is_none() {
            let rate = (need * PROBE_GAIN).min(PROBE_CAP_BPS);
            self.probing = Some((now, rate));
            return Decision::ProbeStart { rate_bps: rate };
        }
        Decision::Stay
    }

    pub fn probe_result(&mut self, now: u64, measured_bps: Option<f64>, demand_bps: f64) -> Option<f64> {
        let (started, _) = self.probing?;
        if now.saturating_sub(started) < PROBE_JUDGE_MS {
            return None;
        }
        self.probing = None;
        match measured_bps {
            Some(m) if m >= demand_bps * HEADROOM => Some(m),
            _ => {
                self.last_demote = Some(now);
                self.clean_since = None;
                None
            }
        }
    }

    pub fn force(&mut self, layer: Layer) {
        self.layer = layer;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demote_needs_two_ticks_and_promote_waits_backoff() {
        let mut p = Policy::default();
        let bad = Signals { loss_pct: Some((10.0, 0)), demand_bps: LAYER_H_BPS, ..Default::default() };
        assert_eq!(p.tick(0, &bad), Decision::Stay);
        assert_eq!(p.tick(1000, &Signals { loss_pct: Some((10.0, 1000)), ..bad }), Decision::Demote(Reason::Loss));
        let good = |t| Signals { loss_pct: Some((0.0, t)), demand_bps: LAYER_H_BPS, ..Default::default() };
        assert_eq!(p.tick(5000, &good(5000)), Decision::Stay);
        assert_eq!(p.tick(15_999, &good(15_999)), Decision::Stay);
        assert_eq!(p.tick(16_000, &good(16_000)), Decision::Promote);
    }

    #[test]
    fn trendline_flags_rising_delay() {
        let mut t = Trendline::default();
        let mut u = Usage::Normal;
        for i in 0..60 {
            u = t.update(5.0 + i as f64 * 0.5, 5.0, i as f64 * 5.0);
        }
        assert_eq!(u, Usage::Overuse);
    }
}
