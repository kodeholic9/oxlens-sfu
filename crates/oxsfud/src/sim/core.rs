// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! v2 속도 산수 — AIMD(draft-ietf-rmcat-gcc-02 §5.5 · libwebrtc AimdRateControl · LinkCapacityEstimator) · trendline 과사용 판정 · 층 판단(신호 → 결정 + 사유). 시각은 인자.

use std::collections::VecDeque;

pub const FLOOR_BPS: f64 = 30_000.0;
pub const BETA: f64 = 0.85;
pub const PACKET_BYTES: f64 = 1200.0;
pub const FPS: f64 = 30.0;

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

#[derive(Debug, Clone, Copy, PartialEq)]
struct LinkCapacity {
    estimate_kbps: Option<f64>,
    deviation_kbps: f64,
}

impl LinkCapacity {
    fn new() -> LinkCapacity {
        LinkCapacity { estimate_kbps: None, deviation_kbps: 0.4 }
    }

    fn reset(&mut self) {
        self.estimate_kbps = None;
    }

    fn deviation_bps(&self, e: f64) -> f64 {
        (self.deviation_kbps * e).sqrt() * 1000.0
    }

    fn upper_bps(&self) -> f64 {
        match self.estimate_kbps {
            Some(e) => e * 1000.0 + 3.0 * self.deviation_bps(e),
            None => f64::INFINITY,
        }
    }

    fn lower_bps(&self) -> f64 {
        match self.estimate_kbps {
            Some(e) => (e * 1000.0 - 3.0 * self.deviation_bps(e)).max(0.0),
            None => 0.0,
        }
    }

    fn on_overuse(&mut self, acked_bps: f64) {
        let alpha = 0.05;
        let sample = acked_bps / 1000.0;
        let est = match self.estimate_kbps {
            None => sample,
            Some(e) => (1.0 - alpha) * e + alpha * sample,
        };
        self.estimate_kbps = Some(est);
        let norm = est.max(1.0);
        let err = est - sample;
        self.deviation_kbps = ((1.0 - alpha) * self.deviation_kbps + alpha * err * err / norm).clamp(0.4, 2.5);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Hold,
    Increase,
    Decrease,
}

#[derive(Debug, Clone)]
pub struct Aimd {
    current_bps: f64,
    state: State,
    link: LinkCapacity,
    last_change_ms: Option<u64>,
    pub rtt_ms: u64,
    pub min_bps: f64,
    pub max_bps: f64,
    pub measured: bool,
}

impl Aimd {
    pub fn new(start_bps: u64, rtt_ms: u64) -> Aimd {
        Aimd {
            current_bps: start_bps as f64,
            state: State::Hold,
            link: LinkCapacity::new(),
            last_change_ms: None,
            rtt_ms,
            min_bps: FLOOR_BPS,
            max_bps: f64::INFINITY,
            measured: false,
        }
    }

    pub fn bps(&self) -> u64 {
        self.current_bps as u64
    }

    pub fn near(&self) -> bool {
        self.link.estimate_kbps.is_some()
    }

    pub fn set_estimate(&mut self, bps: u64, now_ms: u64) {
        self.current_bps = bps as f64;
        self.last_change_ms = Some(now_ms);
        self.measured = true;
    }

    fn near_max_rate(&self) -> f64 {
        let frame_bits = self.current_bps / FPS;
        let packet_bits = PACKET_BYTES * 8.0;
        let packets = (frame_bits / packet_bits).ceil().max(1.0);
        let avg_packet_bits = frame_bits / packets;
        let response_s = 2.0 * (self.rtt_ms as f64 + 100.0) / 1000.0;
        (avg_packet_bits / response_s).max(4000.0)
    }

    pub fn update(&mut self, now_ms: u64, usage: Usage, acked_bps: u64) -> u64 {
        let acked = acked_bps as f64;
        match usage {
            Usage::Normal => {
                if self.state == State::Hold {
                    self.last_change_ms = Some(now_ms);
                    self.state = State::Increase;
                }
            }
            Usage::Overuse => self.state = State::Decrease,
            Usage::Underuse => self.state = State::Hold,
        }
        let mut new = self.current_bps;
        match self.state {
            State::Hold => {}
            State::Increase => {
                if acked > self.link.upper_bps() {
                    self.link.reset();
                }
                let elapsed_s = self.last_change_ms.map(|t| now_ms.saturating_sub(t) as f64 / 1000.0).unwrap_or(0.0);
                if self.near() {
                    new += self.near_max_rate() * elapsed_s;
                } else {
                    let alpha = 1.08f64.powf(elapsed_s.min(1.0));
                    new += (self.current_bps * (alpha - 1.0)).max(1000.0);
                }
                self.last_change_ms = Some(now_ms);
            }
            State::Decrease => {
                let mut dec = acked * BETA;
                if dec > 5000.0 {
                    dec -= 5000.0;
                }
                if dec > self.current_bps
                    && let Some(e) = self.link.estimate_kbps
                {
                    dec = BETA * e * 1000.0;
                }
                if dec < self.current_bps {
                    new = dec;
                }
                if acked < self.link.lower_bps() {
                    self.link.reset();
                }
                self.link.on_overuse(acked);
                self.measured = true;
                self.state = State::Hold;
                self.last_change_ms = Some(now_ms);
            }
        }
        let cap = 1.5 * acked + 10_000.0;
        if new > self.current_bps && new > cap {
            new = self.current_bps.max(cap);
        }
        self.current_bps = new.clamp(self.min_bps, self.max_bps).floor();
        self.bps()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Trendline {
    first_arrival_ms: Option<f64>,
    acc_delay: f64,
    smoothed: f64,
    samples: std::collections::VecDeque<(f64, f64)>,
    threshold: f64,
    last_update_ms: Option<f64>,
    overuse_ms: f64,
    overuse_count: u32,
    prev_trend: f64,
    deltas: u32,
    pub usage: Option<Usage>,
}

pub const TREND_WINDOW: usize = 20;
pub const TREND_MIN_DELTAS: u32 = 60;
pub const TREND_DELTA_MAX: u32 = 1000;

impl Trendline {
    pub fn new() -> Trendline {
        Trendline { threshold: 12.5, overuse_ms: -1.0, usage: Some(Usage::Normal), ..Default::default() }
    }

    pub fn update(&mut self, recv_delta_ms: f64, send_delta_ms: f64, arrival_ms: f64) -> Usage {
        let delta = recv_delta_ms - send_delta_ms;
        self.deltas = (self.deltas + 1).min(TREND_DELTA_MAX);
        let first = *self.first_arrival_ms.get_or_insert(arrival_ms);
        self.acc_delay += delta;
        self.smoothed = 0.9 * self.smoothed + 0.1 * self.acc_delay;
        self.samples.push_back((arrival_ms - first, self.smoothed));
        if self.samples.len() > TREND_WINDOW {
            self.samples.pop_front();
        }
        let mut trend = self.prev_trend;
        if self.samples.len() == TREND_WINDOW {
            let n = self.samples.len() as f64;
            let mx = self.samples.iter().map(|s| s.0).sum::<f64>() / n;
            let my = self.samples.iter().map(|s| s.1).sum::<f64>() / n;
            let num: f64 = self.samples.iter().map(|s| (s.0 - mx) * (s.1 - my)).sum();
            let den: f64 = self.samples.iter().map(|s| (s.0 - mx) * (s.0 - mx)).sum();
            if den != 0.0 {
                trend = num / den;
            }
        }
        if self.deltas < 2 {
            self.usage = Some(Usage::Normal);
            return Usage::Normal;
        }
        let modified = (self.deltas.min(TREND_MIN_DELTAS) as f64) * trend * 4.0;
        let usage = if modified > self.threshold {
            self.overuse_ms = if self.overuse_ms < 0.0 { send_delta_ms / 2.0 } else { self.overuse_ms + send_delta_ms };
            self.overuse_count += 1;
            if self.overuse_ms > 10.0 && self.overuse_count > 1 && trend >= self.prev_trend {
                self.overuse_ms = 0.0;
                self.overuse_count = 0;
                Usage::Overuse
            } else {
                self.usage.unwrap_or(Usage::Normal)
            }
        } else if modified < -self.threshold {
            self.overuse_ms = -1.0;
            self.overuse_count = 0;
            Usage::Underuse
        } else {
            self.overuse_ms = -1.0;
            self.overuse_count = 0;
            Usage::Normal
        };
        let dt = self.last_update_ms.map(|t| (arrival_ms - t).min(100.0)).unwrap_or(0.0);
        let k = if modified.abs() < self.threshold { 0.039 } else { 0.0087 };
        if modified.abs() <= self.threshold + 15.0 {
            self.threshold = (self.threshold + k * (modified.abs() - self.threshold) * dt).clamp(6.0, 600.0);
        }
        self.last_update_ms = Some(arrival_ms);
        self.prev_trend = trend;
        self.usage = Some(usage);
        usage
    }
}

pub const LB_MIN_PACKETS: u64 = 20;
pub const LB_LOW_LOSS: f64 = 0.02;
pub const LB_HIGH_LOSS: f64 = 0.1;
pub const LB_DECREASE_GAP_MS: u64 = 300;
pub const LB_INCREASE_WINDOW_MS: u64 = 1_000;
pub const LB_INCREASE_GAIN: f64 = 1.08;
pub const LB_INCREASE_ADD_BPS: f64 = 1_000.0;

#[derive(Debug, Clone)]
pub struct LossBased {
    pub target: f64,
    hist: VecDeque<(u64, f64)>,
    expected: u64,
    lost: u64,
    frac: Option<u32>,
    decreased_since_report: bool,
    last_decrease: Option<u64>,
    pub measured: bool,
}

impl LossBased {
    pub fn new(start_bps: f64) -> LossBased {
        LossBased { target: start_bps, hist: VecDeque::new(), expected: 0, lost: 0, frac: None, decreased_since_report: false, last_decrease: None, measured: false }
    }

    pub fn report(&mut self, lost: u64, packets: u64, now: u64, limit: f64) -> f64 {
        if packets > 0 {
            let expected = self.expected + packets;
            if expected < LB_MIN_PACKETS {
                self.expected = expected;
                self.lost += lost;
                return self.set(self.target, now, limit);
            }
            self.decreased_since_report = false;
            self.frac = Some((((self.lost + lost) * 256) / expected).min(255) as u32);
            self.expected = 0;
            self.lost = 0;
        }
        self.update(now, limit)
    }

    fn update(&mut self, now: u64, limit: f64) -> f64 {
        let Some(frac) = self.frac else { return self.set(self.target, now, limit) };
        let loss = frac as f64 / 256.0;
        if loss <= LB_LOW_LOSS {
            while self.hist.front().is_some_and(|h| now.saturating_sub(h.0) >= LB_INCREASE_WINDOW_MS) {
                self.hist.pop_front();
            }
            let base = self.hist.iter().map(|h| h.1).fold(self.target, f64::min);
            return self.set((base * LB_INCREASE_GAIN + 0.5).floor() + LB_INCREASE_ADD_BPS, now, limit);
        }
        if loss > LB_HIGH_LOSS && !self.decreased_since_report && self.last_decrease.is_none_or(|t| now.saturating_sub(t) >= LB_DECREASE_GAP_MS) {
            self.last_decrease = Some(now);
            self.decreased_since_report = true;
            self.measured = true;
            return self.set((self.target * (512 - frac) as f64 / 512.0).floor(), now, limit);
        }
        self.set(self.target, now, limit)
    }

    fn set(&mut self, bps: f64, now: u64, limit: f64) -> f64 {
        self.target = bps.min(limit);
        while self.hist.front().is_some_and(|h| now.saturating_sub(h.0) >= LB_INCREASE_WINDOW_MS) {
            self.hist.pop_front();
        }
        self.hist.push_back((now, self.target));
        self.target
    }

    pub fn set_estimate(&mut self, bps: f64, now: u64) {
        self.target = bps;
        self.hist.clear();
        self.hist.push_back((now, bps));
        self.measured = true;
    }
}

pub const IA_GROUP_US: i64 = 5_000;
pub const IA_BURST_US: i64 = 5_000;
pub const IA_MAX_BURST_US: i64 = 100_000;
pub const IA_OFFSET_RESET_US: i64 = 3_000_000;
pub const IA_REORDER_RESET: u32 = 3;

#[derive(Debug, Clone, Copy)]
struct ArrivalGroup {
    first_send: i64,
    send: i64,
    first_arrival: i64,
    complete: i64,
    system: i64,
}

#[derive(Debug, Clone, Default)]
pub struct InterArrival {
    cur: Option<ArrivalGroup>,
    prev: Option<ArrivalGroup>,
    reordered: u32,
}

impl InterArrival {
    pub fn push(&mut self, send: i64, arrival: i64, system: i64) -> Option<(f64, f64)> {
        let fresh = ArrivalGroup { first_send: send, send, first_arrival: arrival, complete: arrival, system };
        let Some(c) = self.cur else {
            self.cur = Some(fresh);
            return None;
        };
        if c.first_send > send {
            return None;
        }
        if !self.belongs_to_burst(&c, send, arrival) && send - c.first_send > IA_GROUP_US {
            let mut out = None;
            if let Some(p) = self.prev {
                let send_d = c.send - p.send;
                let arr_d = c.complete - p.complete;
                let sys_d = c.system - p.system;
                if arr_d - sys_d >= IA_OFFSET_RESET_US {
                    self.reset();
                    return None;
                }
                if arr_d < 0 {
                    self.reordered += 1;
                    if self.reordered >= IA_REORDER_RESET {
                        self.reset();
                    }
                    return None;
                }
                self.reordered = 0;
                out = Some((arr_d as f64 / 1000.0, send_d as f64 / 1000.0));
            }
            self.prev = Some(c);
            self.cur = Some(fresh);
            return out;
        }
        self.cur = Some(ArrivalGroup { send: c.send.max(send), complete: arrival, system, ..c });
        None
    }

    fn belongs_to_burst(&self, c: &ArrivalGroup, send: i64, arrival: i64) -> bool {
        let arr_d = arrival - c.complete;
        let send_d = send - c.send;
        if send_d == 0 {
            return true;
        }
        arr_d - send_d < 0 && arr_d <= IA_BURST_US && arrival - c.first_arrival < IA_MAX_BURST_US
    }

    fn reset(&mut self) {
        self.cur = None;
        self.prev = None;
        self.reordered = 0;
    }
}

pub const P_H_BPS: f64 = 1_650_000.0;
pub const P_L_BPS: f64 = 250_000.0;
pub const P_HEADROOM: f64 = 1.2;
pub const P_DEMOTE_LOSS: f64 = 0.08;
pub const P_CLEAN_LOSS: f64 = 0.02;
pub const P_NACK_RATIO: f64 = 0.08;
pub const P_NACK_WIN_TICKS: usize = 3;
pub const P_DEMOTE_TICKS: u32 = 2;
pub const P_BACKOFF_MS: u64 = 15_000;
pub const P_BACKOFF_MAX_MS: u64 = 60_000;
pub const P_COLLAPSE_MS: u64 = 5_000;
pub const P_GRACE_MS: u64 = 10_000;
pub const P_PROBE_JUDGE_MS: u64 = 4_000;
pub const P_PROBE_GAIN: f64 = 1.35;
pub const P_PROBE_MAX_BPS: f64 = 3_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Estimate,
    Loss,
    Nack,
    Drop,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::Estimate => "estimate",
            Reason::Loss => "loss",
            Reason::Nack => "nack",
            Reason::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    pub now: u64,
    pub v2: bool,
    pub rows: usize,
    pub loss: f64,
    pub sent: u64,
    pub repeats: u64,
    pub drops: u64,
    pub estimate: f64,
    pub measured: bool,
    pub overuse: bool,
    pub probe: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub to: Option<u8>,
    pub reasons: Vec<Reason>,
    pub promote_by: Option<&'static str>,
    pub probe: Option<f64>,
    pub probe_end: bool,
    pub raise: Option<f64>,
    pub nack_ratio: f64,
    pub demand_h: f64,
    pub demand_now: f64,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub level: u8,
    pub bad: u32,
    pub last_demote: u64,
    pub last_promote: u64,
    pub backoff: u64,
    pub win: VecDeque<(u64, u64)>,
    pub probe_at: Option<u64>,
    pub started: bool,
    pub joined_at: Option<u64>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { level: 1, bad: 0, last_demote: 0, last_promote: 0, backoff: P_BACKOFF_MS, win: VecDeque::new(), probe_at: None, started: false, joined_at: None }
    }
}

impl Policy {
    pub fn tick(&mut self, s: &Signals) -> Decision {
        let rows = s.rows as f64;
        let demand_h = rows * P_H_BPS * P_HEADROOM;
        let demand_now = rows * if self.level == 1 { P_H_BPS } else { P_L_BPS } * P_HEADROOM;
        self.win.push_back((s.sent, s.repeats));
        while self.win.len() > P_NACK_WIN_TICKS {
            self.win.pop_front();
        }
        let (sent, rep) = self.win.iter().fold((0u64, 0u64), |(a, b), (x, y)| (a + x, b + y));
        let nack_ratio = if sent == 0 { if rep > 0 { 1.0 } else { 0.0 } } else { (rep as f64 / sent as f64).min(1.0) };
        let joined = *self.joined_at.get_or_insert(s.now);
        let in_grace = s.now.saturating_sub(self.last_promote.max(joined)) < P_GRACE_MS;
        let mut reasons = Vec::new();
        if s.v2 && s.measured && !in_grace && self.level == 1 && s.estimate < demand_now {
            reasons.push(Reason::Estimate);
        }
        if !s.v2 && s.loss >= P_DEMOTE_LOSS {
            reasons.push(Reason::Loss);
        }
        if !s.v2 && self.win.len() >= P_NACK_WIN_TICKS && rep > 0 && nack_ratio > P_NACK_RATIO {
            reasons.push(Reason::Nack);
        }
        if s.drops > 0 {
            reasons.push(Reason::Drop);
        }
        let bad = !reasons.is_empty();
        self.bad = if bad { self.bad + 1 } else { 0 };
        let (mut to, mut probe, mut raise, mut probe_end) = (None, None, None, false);
        let mut promote_by = None;
        if self.level == 1 && self.bad >= P_DEMOTE_TICKS {
            if s.now.saturating_sub(self.last_promote) < P_COLLAPSE_MS && !reasons.contains(&Reason::Estimate) {
                self.backoff = (self.backoff * 2).min(P_BACKOFF_MAX_MS);
            }
            self.level = 0;
            self.bad = 0;
            self.last_demote = s.now;
            to = Some(0);
        } else if self.level == 0 && !bad && (s.v2 || s.loss <= P_CLEAN_LOSS) && s.now.saturating_sub(self.last_demote) >= self.backoff {
            if !s.v2 || (!s.overuse && s.estimate >= demand_h) {
                self.level = 1;
                self.last_promote = s.now;
                to = Some(1);
                promote_by = Some(if s.v2 { "estimate" } else { "clean" });
            } else if self.probe_at.is_none() {
                self.probe_at = Some(s.now);
                probe = Some((demand_h * P_PROBE_GAIN).min(P_PROBE_MAX_BPS));
            }
        }
        if s.v2 && !self.started && self.level == 1 && self.probe_at.is_none() {
            self.started = true;
            self.probe_at = Some(s.now);
            probe = Some((demand_h * P_PROBE_GAIN).min(P_PROBE_MAX_BPS));
        }
        if let Some(at) = self.probe_at
            && s.now.saturating_sub(at) >= P_PROBE_JUDGE_MS
        {
            self.probe_at = None;
            probe_end = true;
            match s.probe {
                Some(x) if x >= demand_h => raise = Some(x),
                _ => self.last_demote = s.now,
            }
        }
        Decision { to, reasons, promote_by, probe, probe_end, raise, nack_ratio, demand_h, demand_now }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_increase_is_1000() {
        let mut a = Aimd::new(300_000, 200);
        assert_eq!(a.update(0, Usage::Normal, 300_000), 301_000);
        assert_eq!(a.update(500, Usage::Normal, 300_000), 312_808);
    }

    #[test]
    fn decrease_from_acked() {
        let mut a = Aimd::new(1_000_000, 200);
        assert_eq!(a.update(0, Usage::Overuse, 900_000), 760_000);
        assert!(a.near());
    }
}
