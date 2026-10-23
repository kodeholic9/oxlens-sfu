// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-SIM-TICK · 정:RULE-SIM-MODES · 정:IMPL-SIM-SWITCH · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 자동 판단(순수) — `policy_tick(now, 신호)` · v2 지연 추정(GCC — trendline · 적응 문턱 · AIMD). 시계 · 신호 전부 인자.

use std::collections::VecDeque;

pub const H_BPS: f64 = 1_650_000.0;
pub const L_BPS: f64 = 250_000.0;
pub const HEADROOM: f64 = 1.2;
pub const LOSS_DEMOTE: f64 = 0.08;
pub const LOSS_CLEAN: f64 = 0.02;
pub const NACK_DEMOTE: f64 = 20.0;
pub const STREAK: u8 = 2;
pub const BACKOFF_INIT_MS: u64 = 15_000;
pub const BACKOFF_CAP_MS: u64 = 60_000;
pub const COLLAPSE_MS: u64 = 5_000;
pub const REMB_GRACE_MS: u64 = 10_000;
pub const FRESH_REMB_MS: u64 = 5_000;
pub const FRESH_LOSS_MS: u64 = 6_000;
pub const PENDING_MS: u64 = 10_000;
pub const PLI_H_MS: u64 = 300;
pub const PLI_L_MS: u64 = 100;
pub const V2_STALE_MS: u64 = 1_000;
pub const GCC_START_BPS: f64 = 300_000.0;
pub const GCC_FLOOR_BPS: f64 = 30_000.0;
pub const GCC_DECREASE_GAP_MS: u64 = 300;
pub const PROBE_MS: u64 = 2_500;
pub const PROBE_JUDGE_MS: u64 = 4_000;
pub const PROBE_CHUNK_MS: u64 = 20;
pub const PROBE_GAIN: f64 = 1.35;
pub const PROBE_CAP_BPS: f64 = 3_000_000.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub started: u64,
    pub target: f64,
    pub rate: f64,
    pub seqs: std::collections::BTreeSet<u16>,
    pub acked: usize,
    pub first_arr: Option<i64>,
    pub last_arr: Option<i64>,
}

impl Probe {
    pub fn new(now: u64, need: f64) -> Probe {
        Probe { started: now, target: need, rate: (need * PROBE_GAIN).min(PROBE_CAP_BPS), ..Probe::default() }
    }

    pub fn sending(&self, now: u64) -> bool {
        now.saturating_sub(self.started) < PROBE_MS
    }

    pub fn due(&self, now: u64) -> bool {
        now.saturating_sub(self.started) >= PROBE_JUDGE_MS
    }

    pub fn ack(&mut self, seq: u16, arr_us: i64, size: usize) -> bool {
        if !self.seqs.contains(&seq) {
            return false;
        }
        self.acked += size;
        self.first_arr = Some(self.first_arr.map(|f| f.min(arr_us)).unwrap_or(arr_us));
        self.last_arr = Some(self.last_arr.map(|l| l.max(arr_us)).unwrap_or(arr_us));
        true
    }

    pub fn measured(&self) -> Option<f64> {
        if self.acked == 0 {
            return None;
        }
        let span_us = match (self.first_arr, self.last_arr) {
            (Some(f), Some(l)) if l > f => (l - f) as f64,
            _ => PROBE_MS as f64 * 1000.0,
        };
        Some(self.acked as f64 * 8.0 * 1_000_000.0 / span_us.max(PROBE_CHUNK_MS as f64 * 1000.0))
    }

    pub fn proven(&self) -> Option<bool> {
        self.measured().map(|m| m >= self.target)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    High,
    Low,
    Pause,
}

impl Layer {
    pub fn rid_index(self) -> Option<u8> {
        match self {
            Layer::High => Some(1),
            Layer::Low => Some(0),
            Layer::Pause => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Layer::High => "h",
            Layer::Low => "l",
            Layer::Pause => "pause",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signals {
    pub now: u64,
    pub remb: Option<(f64, u64)>,
    pub loss: Option<(f64, u64)>,
    pub nack_per_s: f64,
    pub drops: u64,
    pub est: Option<(f64, u64)>,
    pub overuse: bool,
    pub tracks: usize,
    pub v2: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    pub layer: Layer,
    pub streak: u8,
    pub last_promote: u64,
    pub last_demote: u64,
    pub backoff: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { layer: Layer::High, streak: 0, last_promote: 0, last_demote: 0, backoff: BACKOFF_INIT_MS }
    }
}

fn fresh(v: Option<(f64, u64)>, now: u64, ttl: u64) -> Option<f64> {
    v.filter(|(_, at)| now.saturating_sub(*at) <= ttl).map(|(x, _)| x)
}

pub fn estimate(s: &Signals) -> Option<f64> {
    if s.v2
        && let Some(e) = fresh(s.est, s.now, V2_STALE_MS) {
            return Some(e);
        }
    fresh(s.remb, s.now, FRESH_REMB_MS)
}

pub fn policy_tick(p: &mut Policy, s: &Signals) -> Option<(Layer, &'static str)> {
    if s.tracks == 0 {
        return None;
    }
    let n = s.tracks as f64;
    let est = estimate(s);
    let loss = fresh(s.loss, s.now, FRESH_LOSS_MS);
    let demand = n * if p.layer == Layer::High { H_BPS } else { L_BPS } * HEADROOM;
    if p.layer != Layer::Pause {
        let in_grace = p.layer == Layer::High && s.now.saturating_sub(p.last_promote) < REMB_GRACE_MS && p.last_promote > 0;
        let reason = if !in_grace && est.map(|e| e < demand).unwrap_or(false) {
            Some("remb")
        } else if loss.map(|l| l > LOSS_DEMOTE).unwrap_or(false) {
            Some("loss")
        } else if s.nack_per_s > NACK_DEMOTE {
            Some("nack")
        } else if s.drops > 0 {
            Some("drop")
        } else {
            None
        };
        match reason {
            Some(r) => {
                p.streak += 1;
                if p.streak >= STREAK {
                    p.streak = 0;
                    let collapsed = p.last_promote > 0 && s.now.saturating_sub(p.last_promote) < COLLAPSE_MS && r != "remb";
                    if collapsed {
                        p.backoff = (p.backoff * 2).min(BACKOFF_CAP_MS);
                    }
                    p.last_demote = s.now;
                    p.layer = if p.layer == Layer::High { Layer::Low } else { Layer::Pause };
                    return Some((p.layer, r));
                }
                return None;
            }
            None => p.streak = 0,
        }
    }
    let clean = loss.map(|l| l <= LOSS_CLEAN).unwrap_or(true) && !s.overuse;
    let up = match p.layer {
        Layer::Pause => Layer::Low,
        Layer::Low => Layer::High,
        Layer::High => return None,
    };
    let need = n * if up == Layer::High { H_BPS } else { L_BPS } * HEADROOM;
    let cap_ok = est.map(|e| e >= need).unwrap_or(!s.v2);
    if clean && cap_ok && s.now.saturating_sub(p.last_demote) >= p.backoff {
        p.layer = up;
        p.last_promote = s.now;
        return Some((up, "clean"));
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage {
    Normal,
    Over,
    Under,
}

#[derive(Clone, Debug)]
pub struct Gcc {
    pub est: f64,
    pub acked_bps: f64,
    acc: f64,
    smooth: f64,
    window: VecDeque<(f64, f64)>,
    first_arrival: Option<f64>,
    thr: f64,
    over_since: Option<f64>,
    over_count: u32,
    prev_trend: f64,
    pub usage: Usage,
    last_decrease: u64,
    last_update: u64,
    group: Option<(f64, f64, f64)>,
    prev_group: Option<(f64, f64)>,
    samples: u32,
}

impl Default for Gcc {
    fn default() -> Self {
        Gcc {
            est: GCC_START_BPS,
            acked_bps: 0.0,
            acc: 0.0,
            smooth: 0.0,
            window: VecDeque::new(),
            first_arrival: None,
            thr: 12.5,
            over_since: None,
            over_count: 0,
            prev_trend: 0.0,
            usage: Usage::Normal,
            last_decrease: 0,
            last_update: 0,
            group: None,
            prev_group: None,
            samples: 0,
        }
    }
}

impl Gcc {
    pub fn on_packet(&mut self, send_ms: f64, arrival_ms: f64) {
        match self.group {
            Some((first_send, last_send, last_arr)) if send_ms - first_send <= 5.0 => {
                self.group = Some((first_send, last_send.max(send_ms), last_arr.max(arrival_ms)));
            }
            Some((_, last_send, last_arr)) => {
                if let Some((ps, pa)) = self.prev_group {
                    self.delta(last_send - ps, last_arr - pa, last_arr);
                }
                self.prev_group = Some((last_send, last_arr));
                self.group = Some((send_ms, send_ms, arrival_ms));
            }
            None => self.group = Some((send_ms, send_ms, arrival_ms)),
        }
    }

    fn delta(&mut self, send_d: f64, arr_d: f64, arrival: f64) {
        let d = arr_d - send_d;
        self.acc += d;
        self.smooth = 0.9 * self.smooth + 0.1 * self.acc;
        let base = *self.first_arrival.get_or_insert(arrival);
        self.window.push_back((arrival - base, self.smooth));
        if self.window.len() > 20 {
            self.window.pop_front();
        }
        self.samples = self.samples.saturating_add(1);
        if self.window.len() < 2 {
            return;
        }
        let n = self.window.len() as f64;
        let mx = self.window.iter().map(|p| p.0).sum::<f64>() / n;
        let my = self.window.iter().map(|p| p.1).sum::<f64>() / n;
        let num: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let den: f64 = self.window.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        let slope = if den > 0.0 { num / den } else { 0.0 };
        let m = slope * (self.samples.min(60) as f64) * 4.0;
        if (m.abs() - self.thr) <= 15.0 {
            let k = if m.abs() < self.thr { 0.039 } else { 0.0087 };
            self.thr = (self.thr + k * (m.abs() - self.thr) * send_d.abs().clamp(1.0, 100.0)).clamp(6.0, 600.0);
        }
        if m > self.thr {
            let since = *self.over_since.get_or_insert(arrival);
            self.over_count += 1;
            if arrival - since > 10.0 && self.over_count > 1 && m >= self.prev_trend {
                self.usage = Usage::Over;
            }
        } else if m < -self.thr {
            self.over_since = None;
            self.over_count = 0;
            self.usage = Usage::Under;
        } else {
            self.over_since = None;
            self.over_count = 0;
            self.usage = Usage::Normal;
        }
        self.prev_trend = m;
    }

    pub fn update(&mut self, now: u64, acked_bps: f64) {
        self.acked_bps = acked_bps;
        let dt = if self.last_update == 0 { 0.0 } else { (now.saturating_sub(self.last_update)) as f64 / 1000.0 };
        self.last_update = now;
        match self.usage {
            Usage::Over => {
                if now.saturating_sub(self.last_decrease) >= GCC_DECREASE_GAP_MS {
                    self.est = (0.85 * acked_bps).max(GCC_FLOOR_BPS);
                    self.last_decrease = now;
                }
            }
            Usage::Normal => {
                let cap = 1.5 * acked_bps + 10_000.0;
                let next = self.est * 1.08f64.powf(dt.min(1.0));
                self.est = next.min(cap.max(self.est)).max(GCC_FLOOR_BPS);
            }
            Usage::Under => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(now: u64) -> Signals {
        Signals { now, tracks: 1, ..Signals::default() }
    }

    #[test]
    fn demote_needs_two_ticks_and_promote_waits_backoff() {
        let mut p = Policy::default();
        let mut s = sig(1000);
        s.loss = Some((0.2, 1000));
        assert_eq!(policy_tick(&mut p, &s), None, "정:RULE-SIM-TICK 2연속");
        s.now = 2000;
        assert_eq!(policy_tick(&mut p, &s), Some((Layer::Low, "loss")));
        let mut c = sig(3000);
        c.loss = Some((0.0, 3000));
        assert_eq!(policy_tick(&mut p, &c), None);
        c.now = 2000 + BACKOFF_INIT_MS;
        c.loss = Some((0.0, c.now));
        assert_eq!(policy_tick(&mut p, &c), Some((Layer::High, "clean")));
    }

    #[test]
    fn remb_reason_skipped_in_grace() {
        let mut p = Policy { layer: Layer::High, last_promote: 1000, ..Policy::default() };
        let mut s = sig(2000);
        s.remb = Some((100_000.0, 2000));
        assert_eq!(policy_tick(&mut p, &s), None);
        s.now = 3000;
        s.remb = Some((100_000.0, 3000));
        assert_eq!(policy_tick(&mut p, &s), None, "S-sym-RULE-SIM-TICK-1 승격 유예 10s 중 remb skip");
    }

    #[test]
    fn probe_unmeasured_is_none_not_zero() {
        let mut p = Probe::new(0, 1_980_000.0);
        p.seqs.insert(5);
        assert_eq!(p.measured(), None, "S-sym-RULE-SIM-TICK-2 못 잰 것은 0 이 아니라 없음");
        assert_eq!(p.proven(), None);
        assert!(p.ack(5, 0, 250_000) && p.ack(5, 1_000_000, 250_000));
        assert_eq!(p.proven(), Some(true));
        assert!((p.rate - 1_980_000.0 * PROBE_GAIN).abs() < 1.0);
    }

    #[test]
    fn gcc_decreases_on_overuse() {
        let mut g = Gcc::default();
        let mut arr = 0.0;
        for i in 0..200 {
            let send = i as f64 * 10.0;
            arr += 10.0 + (i as f64) * 0.5;
            g.on_packet(send, arr);
        }
        assert_eq!(g.usage, Usage::Over);
        g.update(1000, 200_000.0);
        assert!((g.est - 170_000.0).abs() < 1.0);
    }
}
