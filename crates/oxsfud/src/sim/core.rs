// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-fable-5-2
//! 자동 층 판단의 바닥 산수 — 지연 기울기 판정(trendline) · 묶음 간 델타 · AIMD 속도(mediasoup 3.32.2 AimdRateControl · LinkCapacityEstimator) · v1 층 판단(신호 → 결정 + 사유). 시각은 인자.

use std::collections::VecDeque;

pub const BWE_MIN: i64 = 5_000;
pub const INF: i64 = i64::MAX;

pub fn scale(bps: i64, factor: f64) -> i64 {
    if bps == INF {
        return INF;
    }
    let r = bps as f64 * factor;
    if r.is_nan() || r >= 9_007_199_254_740_991.0 {
        return INF;
    }
    if r <= 0.0 { 0 } else { r.round() as i64 }
}

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

    pub fn name(self) -> &'static str {
        match self {
            Usage::Normal => "normal",
            Usage::Overuse => "overuse",
            Usage::Underuse => "underuse",
        }
    }
}

pub const AIMD_BETA: f64 = 0.85;
pub const AIMD_RTT_US: i64 = 200_000;
pub const AIMD_INIT_US: i64 = 5_000_000;
pub const AIMD_REDUCE_MIN_US: i64 = 10_000;
pub const AIMD_REDUCE_MAX_US: i64 = 200_000;
pub const AIMD_MAX_BPS: i64 = 30_000_000;
pub const AIMD_HEADROOM_BPS: i64 = 10_000;
pub const AIMD_DECREASE_MARGIN_BPS: i64 = 5_000;
pub const AIMD_MIN_MUL_BPS: i64 = 1_000;
pub const AIMD_ALPHA: f64 = 1.08;
pub const AIMD_FRAME_US: i64 = 33_333;
pub const AIMD_PKT_BYTES: i64 = 1200;
pub const AIMD_DETECTOR_US: i64 = 100_000;
pub const AIMD_MIN_ADD_BPS_PER_S: i64 = 4_000;
pub const CAP_ALPHA: f64 = 0.05;
pub const CAP_BOUND: f64 = 3.0;
pub const CAP_MIN_DEV: f64 = 0.4;
pub const CAP_MAX_DEV: f64 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Hold,
    Increase,
    Decrease,
}

#[derive(Debug, Clone)]
pub struct Aimd {
    pub min: i64,
    pub rate: i64,
    latest: i64,
    pub rtt_us: i64,
    pub valid: bool,
    first_throughput_at: Option<i64>,
    state: State,
    last_change: Option<i64>,
    cap: Option<f64>,
    dev: f64,
}

impl Default for Aimd {
    fn default() -> Self {
        Aimd::new()
    }
}

impl Aimd {
    pub fn new() -> Aimd {
        Aimd {
            min: BWE_MIN,
            rate: AIMD_MAX_BPS,
            latest: AIMD_MAX_BPS,
            rtt_us: AIMD_RTT_US,
            valid: false,
            first_throughput_at: None,
            state: State::Hold,
            last_change: None,
            cap: None,
            dev: CAP_MIN_DEV,
        }
    }

    pub fn near(&self) -> bool {
        self.cap.is_some()
    }

    pub fn cap_kbps(&self) -> Option<f64> {
        self.cap
    }

    pub fn set_start(&mut self, bps: i64) {
        self.rate = bps;
        self.latest = bps;
        self.valid = true;
    }

    pub fn set_min(&mut self, bps: i64) {
        self.min = bps;
        self.rate = bps.max(self.rate);
    }

    pub fn set_estimate(&mut self, bps: i64, now_us: i64) {
        self.valid = true;
        self.rate = bps.max(self.min);
        self.last_change = Some(now_us);
    }

    pub fn time_to_reduce_further(&self, now_us: i64, throughput: i64) -> bool {
        let gap = self.rtt_us.clamp(AIMD_REDUCE_MIN_US, AIMD_REDUCE_MAX_US);
        match self.last_change {
            None => return true,
            Some(t) if now_us.saturating_sub(t) >= gap => return true,
            _ => {}
        }
        self.valid && throughput < (0.5 * self.rate as f64).round() as i64
    }

    pub fn initial_time_to_reduce_further(&self, now_us: i64) -> bool {
        self.valid && self.time_to_reduce_further(now_us, self.rate / 2 - 1)
    }

    fn cap_upper(&self) -> i64 {
        match self.cap {
            None => INF,
            Some(c) => ((c + CAP_BOUND * (self.dev * c).sqrt()) * 1000.0) as i64,
        }
    }

    fn cap_lower(&self) -> i64 {
        match self.cap {
            None => 0,
            Some(c) => ((c - CAP_BOUND * (self.dev * c).sqrt()).max(0.0) * 1000.0) as i64,
        }
    }

    fn cap_update(&mut self, bps: i64) {
        let sample = (bps as f64 / 1000.0).round();
        let c = match self.cap {
            None => sample,
            Some(c) => (1.0 - CAP_ALPHA) * c + CAP_ALPHA * sample,
        };
        self.cap = Some(c);
        let err = c - sample;
        self.dev = ((1.0 - CAP_ALPHA) * self.dev + (CAP_ALPHA * err * err) / c.max(1.0)).clamp(CAP_MIN_DEV, CAP_MAX_DEV);
    }

    fn near_rate(&self) -> i64 {
        let frame = self.rate.saturating_mul(AIMD_FRAME_US).saturating_add(4_000_000) / 8_000_000;
        let pkt = (frame as f64 / (frame as f64 / AIMD_PKT_BYTES as f64).ceil()).round() as i64;
        AIMD_MIN_ADD_BPS_PER_S.max(pkt.saturating_mul(8_000_000) / (self.rtt_us.saturating_add(AIMD_DETECTOR_US).saturating_mul(2)).max(1))
    }

    pub fn update(&mut self, now_us: i64, usage: Usage, throughput: Option<i64>) -> i64 {
        if !self.valid {
            match self.first_throughput_at {
                None => {
                    if throughput.is_some() {
                        self.first_throughput_at = Some(now_us);
                    }
                }
                Some(t) => {
                    if now_us.saturating_sub(t) > AIMD_INIT_US
                        && let Some(k) = throughput
                    {
                        self.rate = k;
                        self.valid = true;
                    }
                }
            }
        }
        let est = throughput.unwrap_or(self.latest);
        if let Some(k) = throughput {
            self.latest = k;
        }
        if !self.valid && usage != Usage::Overuse {
            return self.rate;
        }
        match usage {
            Usage::Normal => {
                if self.state == State::Hold {
                    self.last_change = Some(now_us);
                    self.state = State::Increase;
                }
            }
            Usage::Overuse => self.state = State::Decrease,
            Usage::Underuse => self.state = State::Hold,
        }
        let mut next: Option<i64> = None;
        match self.state {
            State::Hold => {}
            State::Increase => {
                if est > self.cap_upper() {
                    self.cap = None;
                }
                let limit = ((1.5 * est as f64).round() as i64).saturating_add(AIMD_HEADROOM_BPS);
                if self.rate < limit {
                    let inc = if self.cap.is_some() {
                        self.near_rate().saturating_mul(now_us.saturating_sub(self.last_change.unwrap_or(now_us))) / 1_000_000
                    } else {
                        let alpha = match self.last_change {
                            Some(t) => AIMD_ALPHA.powf((now_us.saturating_sub(t) as f64 / 1_000_000.0).min(1.0)),
                            None => AIMD_ALPHA,
                        };
                        ((self.rate as f64 * (alpha - 1.0)).round() as i64).max(AIMD_MIN_MUL_BPS)
                    };
                    next = Some(self.rate.saturating_add(inc).min(limit));
                }
                self.last_change = Some(now_us);
            }
            State::Decrease => {
                let mut dec = (AIMD_BETA * est as f64).round() as i64;
                if dec > AIMD_DECREASE_MARGIN_BPS {
                    dec -= AIMD_DECREASE_MARGIN_BPS;
                }
                if dec > self.rate
                    && let Some(c) = self.cap
                {
                    dec = (AIMD_BETA * ((c * 1000.0) as i64) as f64).round() as i64;
                }
                if dec < self.rate {
                    next = Some(dec);
                }
                if est < self.cap_lower() {
                    self.cap = None;
                }
                self.valid = true;
                self.cap_update(est);
                self.state = State::Hold;
                self.last_change = Some(now_us);
            }
        }
        self.rate = next.unwrap_or(self.rate).max(self.min);
        self.rate
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

    pub fn state(&self) -> Usage {
        self.usage.unwrap_or(Usage::Normal)
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

pub const P_DEMOTE_LOSS: f64 = 0.08;
pub const P_CLEAN_LOSS: f64 = 0.02;
pub const P_NACK_RATIO: f64 = 0.08;
pub const P_NACK_WIN_TICKS: usize = 3;
pub const P_DEMOTE_TICKS: u32 = 2;
pub const P_BACKOFF_MS: u64 = 15_000;
pub const P_BACKOFF_MAX_MS: u64 = 60_000;
pub const P_COLLAPSE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Loss,
    Nack,
    Drop,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::Loss => "loss",
            Reason::Nack => "nack",
            Reason::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    pub now: u64,
    pub loss: f64,
    pub sent: u64,
    pub repeats: u64,
    pub drops: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub to: Option<u8>,
    pub reasons: Vec<Reason>,
    pub nack_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub level: u8,
    bad: u32,
    last_demote: u64,
    last_promote: u64,
    pub backoff: u64,
    win: VecDeque<(u64, u64)>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { level: 1, bad: 0, last_demote: 0, last_promote: 0, backoff: P_BACKOFF_MS, win: VecDeque::new() }
    }
}

impl Policy {
    pub fn tick(&mut self, s: &Signals) -> Decision {
        self.win.push_back((s.sent, s.repeats));
        while self.win.len() > P_NACK_WIN_TICKS {
            self.win.pop_front();
        }
        let sent: u64 = self.win.iter().map(|w| w.0).sum();
        let rep: u64 = self.win.iter().map(|w| w.1).sum();
        let nack_ratio = if sent == 0 {
            if rep > 0 { 1.0 } else { 0.0 }
        } else {
            (rep as f64 / sent as f64).min(1.0)
        };
        let mut reasons = Vec::new();
        if s.loss >= P_DEMOTE_LOSS {
            reasons.push(Reason::Loss);
        }
        if self.win.len() >= P_NACK_WIN_TICKS && rep > 0 && nack_ratio > P_NACK_RATIO {
            reasons.push(Reason::Nack);
        }
        if s.drops > 0 {
            reasons.push(Reason::Drop);
        }
        let bad = !reasons.is_empty();
        self.bad = if bad { self.bad + 1 } else { 0 };
        let mut to = None;
        if self.level == 1 && self.bad >= P_DEMOTE_TICKS {
            if s.now.saturating_sub(self.last_promote) < P_COLLAPSE_MS {
                self.backoff = (self.backoff * 2).min(P_BACKOFF_MAX_MS);
            }
            self.level = 0;
            self.bad = 0;
            self.last_demote = s.now;
            to = Some(0);
        } else if self.level == 0 && !bad && s.loss <= P_CLEAN_LOSS && s.now.saturating_sub(self.last_demote) >= self.backoff {
            self.level = 1;
            self.last_promote = s.now;
            to = Some(1);
        }
        Decision { to, reasons, nack_ratio }
    }
}
