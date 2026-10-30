// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! v2 속도 산수 — AIMD(draft-ietf-rmcat-gcc-02 §5.5 · libwebrtc AimdRateControl · LinkCapacityEstimator) · trendline 과사용 판정. 시각은 인자.

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
    pub usage: Option<Usage>,
}

pub const TREND_WINDOW: usize = 20;

impl Trendline {
    pub fn new() -> Trendline {
        Trendline { threshold: 12.5, usage: Some(Usage::Normal), ..Default::default() }
    }

    pub fn update(&mut self, recv_delta_ms: f64, send_delta_ms: f64, arrival_ms: f64) -> Usage {
        let delta = recv_delta_ms - send_delta_ms;
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
        let modified = (self.samples.len().min(60) as f64) * trend * 4.0;
        let usage = if modified > self.threshold {
            self.overuse_ms += send_delta_ms.max(0.0);
            self.overuse_count += 1;
            if self.overuse_ms > 10.0 && self.overuse_count > 1 && trend >= self.prev_trend {
                self.overuse_ms = 0.0;
                self.overuse_count = 0;
                Usage::Overuse
            } else {
                self.usage.unwrap_or(Usage::Normal)
            }
        } else if modified < -self.threshold {
            self.overuse_ms = 0.0;
            self.overuse_count = 0;
            Usage::Underuse
        } else {
            self.overuse_ms = 0.0;
            self.overuse_count = 0;
            Usage::Normal
        };
        let dt = self.last_update_ms.map(|t| (arrival_ms - t).min(100.0)).unwrap_or(0.0);
        let k = if modified.abs() < self.threshold { 0.039 } else { 0.0087 };
        if modified.abs() - self.threshold < 15.0 {
            self.threshold = (self.threshold + k * (modified.abs() - self.threshold) * dt).clamp(6.0, 600.0);
        }
        self.last_update_ms = Some(arrival_ms);
        self.prev_trend = trend;
        self.usage = Some(usage);
        usage
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
