// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-CONC-SANS-IO · model: claude-fable-5-2
//! 구독 전송로의 대역 추정 한 덩이(mediasoup 3.32.2 BWE 를 옮긴 원천의 거울) — 처리량 · 여유 구간 · 탐색 판단 · 묶음 쏘기 · 묶음 실측 · 손실 기반 · 목표 속도 · 지연 기반을 묶어 목표 속도 한 값을 낸다. 시각은 인자.

use std::collections::{HashMap, VecDeque};

use super::core::{Aimd, BWE_MIN, INF, InterArrival, Trendline, Usage, scale};

pub const TP_WINDOW_PACKETS: usize = 20;
pub const TP_MAX_WINDOW_PACKETS: usize = 500;
pub const TP_MIN_WINDOW_US: i64 = 1_000_000;
pub const TP_MAX_WINDOW_US: i64 = 5_000_000;
pub const TP_REQUIRED_PACKETS: usize = 10;
pub const TP_MAX_REORDER_US: i64 = 1_000_000;
pub const TP_MIN_DURATION_US: i64 = 1_000;

#[derive(Debug, Clone, Copy)]
struct Acked {
    send: i64,
    recv: i64,
    size: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Throughput {
    window: VecDeque<Acked>,
    discarded_send: Option<i64>,
}

impl Throughput {
    pub fn push(&mut self, send: i64, recv: i64, size: i64) {
        self.window.push_back(Acked { send, recv, size });
        let mut i = self.window.len() - 1;
        while i > 0 && self.window[i].recv < self.window[i - 1].recv {
            self.window.swap(i, i - 1);
            i -= 1;
        }
        if self.window.back().map(|p| p.recv - recv > TP_MAX_REORDER_US).unwrap_or(false) {
            self.window.clear();
            self.discarded_send = None;
        }
    }

    pub fn trim(&mut self) {
        while self.outside() {
            if let Some(f) = self.window.pop_front() {
                self.discarded_send = Some(self.discarded_send.unwrap_or(f.send).max(f.send));
            }
        }
    }

    fn outside(&self) -> bool {
        let (Some(first), Some(last)) = (self.window.front(), self.window.back()) else { return false };
        if self.window.len() > TP_MAX_WINDOW_PACKETS {
            return true;
        }
        let dur = last.recv - first.recv;
        if dur > TP_MAX_WINDOW_US {
            return true;
        }
        self.window.len() > TP_WINDOW_PACKETS && dur > TP_MIN_WINDOW_US
    }

    pub fn bitrate(&self) -> Option<i64> {
        let w = &self.window;
        if w.is_empty() || w.len() < TP_REQUIRED_PACKETS {
            return None;
        }
        let (mut gap1, mut gap2) = (0i64, 0i64);
        for i in 1..w.len() {
            let g = w[i].recv - w[i - 1].recv;
            if g > gap1 {
                gap2 = gap1;
                gap1 = g;
            } else if g > gap2 {
                gap2 = g;
            }
        }
        let (mut first_send, mut last_send, mut first_recv): (Option<i64>, Option<i64>, Option<i64>) = (None, None, None);
        let mut last_recv = 0i64;
        let (mut recv_size, mut send_size, mut first_recv_size, mut last_send_size) = (0i64, 0i64, 0i64, 0i64);
        let mut sent = 0usize;
        for p in w {
            if first_recv.map(|t| p.recv < t).unwrap_or(true) {
                first_recv = Some(p.recv);
                first_recv_size = p.size;
            }
            last_recv = last_recv.max(p.recv);
            recv_size += p.size;
            if self.discarded_send.map(|d| p.send < d).unwrap_or(false) {
                continue;
            }
            if last_send.map(|t| p.send > t).unwrap_or(true) {
                last_send = Some(p.send);
                last_send_size = p.size;
            }
            first_send = Some(first_send.unwrap_or(p.send).min(p.send));
            send_size += p.size;
            sent += 1;
        }
        recv_size -= first_recv_size;
        send_size -= last_send_size;
        let recv_dur = (last_recv - first_recv.unwrap_or(last_recv) - gap1 + gap2).max(TP_MIN_DURATION_US);
        let recv_rate = recv_size * 8 * 1_000_000 / recv_dur;
        if sent < TP_REQUIRED_PACKETS {
            return Some(recv_rate);
        }
        let send_dur = (last_send.unwrap_or(0) - first_send.unwrap_or(0)).max(TP_MIN_DURATION_US);
        Some((send_size * 8 * 1_000_000 / send_dur).min(recv_rate))
    }
}

pub const ALR_USAGE: f64 = 0.65;
pub const ALR_START: f64 = 0.8;
pub const ALR_STOP: f64 = 0.5;
pub const ALR_WINDOW_US: i64 = 500_000;

#[derive(Debug, Clone, Default)]
pub struct Alr {
    target: i64,
    max: i64,
    remaining: i64,
    pub last_sent: Option<i64>,
    pub started_at: Option<i64>,
}

impl Alr {
    pub fn set_estimate(&mut self, bps: i64) {
        self.target = (bps as f64 * ALR_USAGE).round() as i64;
        self.max = self.target * ALR_WINDOW_US / 8_000_000;
        self.remaining = self.remaining.max(-self.max).min(self.max);
    }

    pub fn on_sent(&mut self, bytes: i64, sent_us: i64, now_us: i64) {
        let Some(last) = self.last_sent else {
            self.last_sent = Some(sent_us);
            return;
        };
        let dt = sent_us - last;
        self.last_sent = Some(sent_us);
        self.remaining = (self.remaining - bytes).max(-self.max);
        self.remaining = (self.remaining + self.target * dt / 8_000_000).min(self.max);
        let ratio = if self.max == 0 { 0.0 } else { self.remaining as f64 / self.max as f64 };
        if ratio > ALR_START && self.started_at.is_none() {
            self.started_at = Some(now_us);
        } else if ratio < ALR_STOP && self.started_at.is_some() {
            self.started_at = None;
        }
    }
}

pub const PROBE_WAIT_US: i64 = 1_000_000;
pub const PROBE_MAX_BPS: i64 = 10_000_000;
pub const PROBE_DROP_THRESHOLD: f64 = 0.66;
pub const PROBE_DROP_TIMEOUT_US: i64 = 5_000_000;
pub const PROBE_AFTER_DROP: f64 = 0.85;
pub const PROBE_ALR_ENDED_US: i64 = 3_000_000;
pub const PROBE_DROP_GAP_US: i64 = 5_000_000;
pub const PROBE_UNCERTAINTY: f64 = 0.05;
pub const PROBE_FIRST: f64 = 3.0;
pub const PROBE_SECOND: f64 = 6.0;
pub const PROBE_FURTHER: f64 = 2.0;
pub const PROBE_FURTHER_THRESHOLD: f64 = 0.7;
pub const PROBE_ALR_INTERVAL_US: i64 = 5_000_000;
pub const PROBE_ALR_SCALE: f64 = 2.0;
pub const PROBE_ALLOC_FIRST: f64 = 1.0;
pub const PROBE_ALLOC_SECOND: f64 = 2.0;
pub const PROBE_ALLOC_BY_CURRENT: f64 = 2.0;
pub const PROBE_ALLOC_CAP: f64 = 2.0;
pub const PROBE_LOSS_LIMITED_SCALE: f64 = 1.5;
pub const PROBE_DURATION_US: i64 = 15_000;
pub const PROBE_DELTA_US: i64 = 2_000;
pub const PROBE_MIN_PROBES: i64 = 5;
pub const PROBER_CLUSTER_TIMEOUT_US: i64 = 5_000_000;
pub const PROBER_MAX_PENDING: usize = 5;
pub const PROBER_MAX_DELAY_US: i64 = 10_000;
pub const PROBER_EARLY_US: i64 = 1_000;
pub const PE_MIN_PROBES: f64 = 0.8;
pub const PE_MIN_BYTES: f64 = 0.8;
pub const PE_MAX_RATIO: f64 = 2.0;
pub const PE_UNSATURATED: f64 = 0.9;
pub const PE_UTILIZATION: f64 = 0.95;
pub const PE_HISTORY_US: i64 = 1_000_000;
pub const PE_MAX_INTERVAL_US: i64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Delay,
    DelayIncreased,
    Rtt,
    Loss,
    LossIncreasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Init,
    Further,
    Alloc,
    Max,
    Alr,
    Drop,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::Init => "init",
            Why::Further => "further",
            Why::Alloc => "alloc",
            Why::Max => "max",
            Why::Alr => "alr",
            Why::Drop => "drop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cluster {
    pub id: i64,
    pub at_us: i64,
    pub bitrate: i64,
    pub duration_us: i64,
    pub delta_us: i64,
    pub min_probes: i64,
    pub why: Why,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    pub min_probes: i64,
    pub min_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeState {
    Init,
    Waiting,
    Complete,
}

#[derive(Debug, Clone)]
pub struct ProbeController {
    state: ProbeState,
    network: bool,
    pub limit: Limit,
    further_min: i64,
    last_initiated: Option<i64>,
    estimate: i64,
    start: i64,
    max: i64,
    max_allocated: i64,
    pub alr_start: Option<i64>,
    pub alr_ended: Option<i64>,
    last_drop_at: Option<i64>,
    before_drop: i64,
    last_drop_probe_at: Option<i64>,
    next_id: i64,
}

impl Default for ProbeController {
    fn default() -> Self {
        ProbeController {
            state: ProbeState::Init,
            network: false,
            limit: Limit::Delay,
            further_min: INF,
            last_initiated: None,
            estimate: 0,
            start: 0,
            max: PROBE_MAX_BPS,
            max_allocated: 0,
            alr_start: None,
            alr_ended: None,
            last_drop_at: None,
            before_drop: 0,
            last_drop_probe_at: None,
            next_id: 1,
        }
    }
}

impl ProbeController {
    pub fn set_bitrates(&mut self, min: i64, start: i64, max: i64, now_us: i64) -> Vec<Cluster> {
        if start > 0 {
            self.start = start;
            self.estimate = start;
        } else if self.start == 0 {
            self.start = min;
        }
        let old = self.max;
        self.max = if max != INF { max } else { PROBE_MAX_BPS };
        match self.state {
            ProbeState::Init => {
                if self.network {
                    return self.exponential(now_us);
                }
            }
            ProbeState::Complete => {
                if self.estimate != 0 && old < self.max && self.estimate < self.max {
                    let max = self.max;
                    return self.initiate(now_us, &[max], false, Why::Max);
                }
            }
            ProbeState::Waiting => {}
        }
        Vec::new()
    }

    pub fn on_max_allocated(&mut self, bps: i64, now_us: i64) -> Vec<Cluster> {
        if self.state == ProbeState::Complete && bps > self.max_allocated && self.estimate < self.max && self.estimate < bps {
            self.max_allocated = bps;
            let cap = scale(self.estimate, PROBE_ALLOC_BY_CURRENT);
            let mut first = scale(bps, PROBE_ALLOC_FIRST);
            let mut limited = cap < first;
            if limited {
                first = cap;
            }
            let mut probes = vec![first];
            if !limited {
                let mut second = scale(bps, PROBE_ALLOC_SECOND);
                limited = cap < second;
                if limited {
                    second = cap;
                }
                if second > first {
                    probes.push(second);
                }
            }
            return self.initiate(now_us, &probes, limited, Why::Alloc);
        }
        self.max_allocated = bps;
        Vec::new()
    }

    pub fn on_network(&mut self, available: bool, now_us: i64) -> Vec<Cluster> {
        self.network = available;
        if !available && self.state == ProbeState::Waiting {
            self.state = ProbeState::Complete;
            self.further_min = INF;
        }
        if available && self.state == ProbeState::Init && self.start != 0 {
            return self.exponential(now_us);
        }
        Vec::new()
    }

    pub fn set_estimate(&mut self, bps: i64, limit: Limit, now_us: i64) -> Vec<Cluster> {
        self.limit = limit;
        if bps < scale(self.estimate, PROBE_DROP_THRESHOLD) {
            self.last_drop_at = Some(now_us);
            self.before_drop = self.estimate;
        }
        self.estimate = bps;
        if self.state == ProbeState::Waiting && bps > self.further_min {
            return self.initiate(now_us, &[scale(bps, PROBE_FURTHER)], true, Why::Further);
        }
        Vec::new()
    }

    pub fn request_probe(&mut self, now_us: i64) -> Vec<Cluster> {
        let in_alr = self.alr_start.is_some();
        let ended_recently = self.alr_ended.map(|t| now_us - t < PROBE_ALR_ENDED_US).unwrap_or(false);
        if !in_alr && !ended_recently {
            return Vec::new();
        }
        if self.state != ProbeState::Complete {
            return Vec::new();
        }
        let suggested = scale(self.before_drop, PROBE_AFTER_DROP);
        let dropped_recently = self.last_drop_at.map(|t| now_us - t < PROBE_DROP_TIMEOUT_US).unwrap_or(false);
        let probed_recently = self.last_drop_probe_at.map(|t| now_us - t <= PROBE_DROP_GAP_US).unwrap_or(false);
        if scale(suggested, 1.0 - PROBE_UNCERTAINTY) > self.estimate && dropped_recently && !probed_recently {
            self.last_drop_probe_at = Some(now_us);
            return self.initiate(now_us, &[suggested], false, Why::Drop);
        }
        Vec::new()
    }

    pub fn process(&mut self, now_us: i64) -> Vec<Cluster> {
        if self.last_initiated.map(|t| now_us - t > PROBE_WAIT_US).unwrap_or(true) && self.state == ProbeState::Waiting {
            self.state = ProbeState::Complete;
            self.further_min = INF;
        }
        if self.estimate == 0 || self.state != ProbeState::Complete {
            return Vec::new();
        }
        if let Some(alr) = self.alr_start
            && now_us >= alr.max(self.last_initiated.unwrap_or(alr)) + PROBE_ALR_INTERVAL_US
        {
            return self.initiate(now_us, &[scale(self.estimate, PROBE_ALR_SCALE)], true, Why::Alr);
        }
        Vec::new()
    }

    fn exponential(&mut self, now_us: i64) -> Vec<Cluster> {
        let probes = [scale(self.start, PROBE_FIRST), scale(self.start, PROBE_SECOND)];
        self.initiate(now_us, &probes, true, Why::Init)
    }

    fn initiate(&mut self, now_us: i64, bitrates: &[i64], further: bool, why: Why) -> Vec<Cluster> {
        let mut further = further;
        let mut cap = self.max;
        if self.max_allocated > 0 {
            cap = cap.min(scale(self.max_allocated, PROBE_ALLOC_CAP));
        }
        if matches!(self.limit, Limit::Rtt | Limit::DelayIncreased | Limit::Loss) {
            return Vec::new();
        }
        if self.limit == Limit::LossIncreasing {
            cap = cap.min(scale(self.estimate, PROBE_LOSS_LIMITED_SCALE));
        }
        let mut out = Vec::new();
        for &bitrate in bitrates {
            let mut b = bitrate;
            if b == 0 {
                continue;
            }
            if b >= cap {
                b = cap;
                further = false;
            }
            out.push(Cluster { id: self.next_id, at_us: now_us, bitrate: b, duration_us: PROBE_DURATION_US, delta_us: PROBE_DELTA_US, min_probes: PROBE_MIN_PROBES, why });
            self.next_id += 1;
        }
        self.last_initiated = Some(now_us);
        if further && !out.is_empty() {
            self.state = ProbeState::Waiting;
            self.further_min = scale(out[out.len() - 1].bitrate, PROBE_FURTHER_THRESHOLD);
        } else {
            self.state = ProbeState::Complete;
            self.further_min = INF;
        }
        out
    }
}

#[derive(Debug, Clone, Copy)]
struct Queued {
    tag: Tag,
    bitrate: i64,
    delta_us: i64,
    requested_at: i64,
    started_at: Option<i64>,
    sent_bytes: i64,
    sent_probes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    pub tag: Tag,
    pub bytes: i64,
    pub first: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Prober {
    queue: VecDeque<Queued>,
    pub active: bool,
    next_us: Option<i64>,
}

impl Prober {
    pub fn create(&mut self, c: &Cluster) {
        while self.queue.front().map(|q| c.at_us - q.requested_at > PROBER_CLUSTER_TIMEOUT_US).unwrap_or(false) || (!self.queue.is_empty() && self.queue.len() > PROBER_MAX_PENDING) {
            self.queue.pop_front();
        }
        self.queue.push_back(Queued {
            tag: Tag { id: c.id, min_probes: c.min_probes, min_bytes: (c.bitrate * c.duration_us + 4_000_000) / 8_000_000 },
            bitrate: c.bitrate,
            delta_us: c.delta_us,
            requested_at: c.at_us,
            started_at: None,
            sent_bytes: 0,
            sent_probes: 0,
        });
        if !self.active {
            self.next_us = None;
            self.active = true;
        }
    }

    pub fn next_time(&self, now_us: i64) -> Option<i64> {
        if !self.active || self.queue.is_empty() { None } else { Some(self.next_us.unwrap_or(now_us)) }
    }

    pub fn shot(&mut self, now_us: i64) -> Option<Shot> {
        if self.queue.is_empty() || !self.active {
            return None;
        }
        if self.next_us.map(|t| now_us - t > PROBER_MAX_DELAY_US).unwrap_or(false) {
            self.queue.pop_front();
            if self.queue.is_empty() {
                self.active = false;
                return None;
            }
        }
        if now_us + PROBER_EARLY_US < self.next_us.unwrap_or(now_us) {
            return None;
        }
        let q = self.queue.front()?;
        let bytes = (q.bitrate * q.delta_us + 4_000_000) / 8_000_000;
        if bytes == 0 {
            self.queue.clear();
            self.active = false;
            self.next_us = None;
            return None;
        }
        Some(Shot { tag: q.tag, bytes, first: q.sent_bytes == 0 })
    }

    pub fn media(&mut self, now_us: i64, bytes: i64) -> Option<Tag> {
        let q = self.queue.front()?;
        q.started_at?;
        let tag = q.tag;
        self.sent(now_us, bytes);
        Some(tag)
    }

    pub fn sent(&mut self, now_us: i64, bytes: i64) {
        let Some(q) = self.queue.front_mut() else { return };
        if q.sent_probes == 0 {
            q.started_at = Some(now_us);
        }
        q.sent_bytes += bytes;
        q.sent_probes += 1;
        self.next_us = Some(q.started_at.unwrap_or(now_us) + q.sent_bytes * 8 * 1_000_000 / q.bitrate);
        if q.sent_bytes >= q.tag.min_bytes && q.sent_probes >= q.tag.min_probes {
            self.queue.pop_front();
        }
        if self.queue.is_empty() {
            self.active = false;
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Seen {
    first_send: Option<i64>,
    last_send: Option<i64>,
    first_recv: Option<i64>,
    last_recv: Option<i64>,
    size_last_send: i64,
    size_first_recv: i64,
    total: i64,
    probes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    pub id: i64,
    pub send: i64,
    pub recv: i64,
    pub result: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ProbeEstimator {
    clusters: HashMap<i64, Seen>,
    estimate: Option<i64>,
    pub last: Option<Measured>,
}

impl ProbeEstimator {
    pub fn handle(&mut self, tag: Tag, send_us: i64, recv_us: i64, size: i64) -> Option<i64> {
        self.clusters.retain(|_, c| !c.last_recv.map(|t| t + PE_HISTORY_US < recv_us).unwrap_or(false));
        let c = self.clusters.entry(tag.id).or_default();
        if c.first_send.map(|t| send_us < t).unwrap_or(true) {
            c.first_send = Some(send_us);
        }
        if c.last_send.map(|t| send_us > t).unwrap_or(true) {
            c.last_send = Some(send_us);
            c.size_last_send = size;
        }
        if c.first_recv.map(|t| recv_us < t).unwrap_or(true) {
            c.first_recv = Some(recv_us);
            c.size_first_recv = size;
        }
        if c.last_recv.map(|t| recv_us > t).unwrap_or(true) {
            c.last_recv = Some(recv_us);
        }
        c.total += size;
        c.probes += 1;
        if c.probes < (tag.min_probes as f64 * PE_MIN_PROBES) as i64 || c.total < (tag.min_bytes as f64 * PE_MIN_BYTES).round() as i64 {
            return None;
        }
        let send_dur = c.last_send.unwrap_or(0) - c.first_send.unwrap_or(0);
        let recv_dur = c.last_recv.unwrap_or(0) - c.first_recv.unwrap_or(0);
        if send_dur <= 0 || send_dur > PE_MAX_INTERVAL_US || recv_dur <= 0 || recv_dur > PE_MAX_INTERVAL_US {
            return None;
        }
        let send = (c.total - c.size_last_send) * 8 * 1_000_000 / send_dur;
        let recv = (c.total - c.size_first_recv) * 8 * 1_000_000 / recv_dur;
        if recv as f64 / send as f64 > PE_MAX_RATIO {
            self.last = Some(Measured { id: tag.id, send, recv, result: None });
            return None;
        }
        let mut bitrate = send.min(recv);
        if recv < scale(send, PE_UNSATURATED) {
            bitrate = scale(recv, PE_UTILIZATION);
        }
        self.estimate = Some(bitrate);
        self.last = Some(Measured { id: tag.id, send, recv, result: Some(bitrate) });
        Some(bitrate)
    }

    pub fn fetch(&mut self) -> Option<i64> {
        self.estimate.take()
    }
}

pub const LOSS_CANDIDATES: [f64; 3] = [1.02, 1.0, 0.95];
pub const LOSS_RAMPUP: f64 = 1.5;
pub const LOSS_RAMPUP_IN_HOLD: f64 = 1.2;
pub const LOSS_RAMPUP_HOLD_THRESHOLD: f64 = 1.3;
pub const LOSS_BIAS_LINEAR: f64 = 0.0002;
pub const LOSS_BIAS_LOG: f64 = 0.02;
pub const LOSS_BIAS_LOSS_THRESHOLD: f64 = 0.2;
pub const LOSS_BIAS_SMOOTHING: f64 = 0.002;
pub const LOSS_INHERENT_MIN: f64 = 1.0e-3;
pub const LOSS_INHERENT_UPPER_OFFSET: f64 = 0.05;
pub const LOSS_INHERENT_UPPER_BALANCE: f64 = 100_000.0;
pub const LOSS_INHERENT_INIT: f64 = 0.01;
pub const LOSS_NEWTON_ITERATIONS: usize = 1;
pub const LOSS_NEWTON_STEP: f64 = 0.75;
pub const LOSS_OBSERVATION_US: i64 = 250_000;
pub const LOSS_WINDOW: usize = 15;
pub const LOSS_MIN_OBSERVATIONS: i64 = 3;
pub const LOSS_TEMPORAL_WEIGHT: f64 = 0.9;
pub const LOSS_UPPER_TEMPORAL_WEIGHT: f64 = 0.9;
pub const LOSS_UPPER_LOSS_OFFSET: f64 = 0.05;
pub const LOSS_UPPER_BALANCE: f64 = 100_000.0;
pub const LOSS_LOWER_BY_ACKED: f64 = 1.0;
pub const LOSS_BACKOFF_BY_ACKED: f64 = 1.0;
pub const LOSS_MAX_INCREASE: f64 = 1.3;
pub const LOSS_INCREASE_WINDOW_US: i64 = 300_000;
pub const LOSS_HOLD_INIT_US: i64 = 300_000;
pub const LOSS_HOLD_MAX_US: i64 = 60_000_000;
pub const LOSS_HOLD_FACTOR: f64 = 2.0;
pub const LOSS_PADDING_US: i64 = 2_000_000;
pub const PADDING_CATCH_UP_US: i64 = 25_000;
pub const PADDING_DEBT_US: i64 = 500_000;

#[derive(Debug, Clone, Default)]
pub struct Padder {
    budget: i64,
    rate: i64,
    at: Option<i64>,
}

impl Padder {
    pub fn due(&mut self, now_us: i64, rate: i64) -> i64 {
        if rate <= 0 {
            self.at = None;
            self.budget = 0;
            self.rate = 0;
            return 0;
        }
        self.rate = rate;
        let Some(at) = self.at else {
            self.at = Some(now_us);
            self.budget = 0;
            return 0;
        };
        let dt = (now_us - at).max(0);
        self.at = Some(now_us);
        self.budget = (self.budget + rate.saturating_mul(dt) / 8_000_000).min(rate.saturating_mul(PADDING_CATCH_UP_US) / 8_000_000);
        self.budget.max(0)
    }

    pub fn on_sent(&mut self, bytes: i64) {
        if self.at.is_none() {
            return;
        }
        self.budget = (self.budget - bytes).max(-(self.rate.saturating_mul(PADDING_DEBT_US) / 8_000_000));
    }
}
pub const LOSS_MEDIAN_SENDING_FACTOR: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossState {
    Delay,
    Decreasing,
    Increasing,
    Padding,
}

impl LossState {
    pub fn name(self) -> &'static str {
        match self {
            LossState::Delay => "delay",
            LossState::Decreasing => "decreasing",
            LossState::Increasing => "increasing",
            LossState::Padding => "padding",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LossPacket {
    pub seq: i64,
    pub send_us: i64,
    pub size: i64,
    pub received: bool,
}

#[derive(Debug, Clone, Copy)]
struct Channel {
    inherent: f64,
    bitrate: i64,
}

#[derive(Debug, Clone, Copy)]
struct Obs {
    id: i64,
    size: i64,
    lost_size: i64,
    rate: i64,
}

#[derive(Debug, Clone, Copy)]
struct Hold {
    at_us: i64,
    duration_us: i64,
    bitrate: i64,
}

fn loss_probability(inherent: f64, limit: i64, rate: i64) -> f64 {
    let inherent = inherent.max(0.0).min(1.0);
    let mut p = inherent;
    if rate > 0 && limit > 0 && rate > limit {
        p += ((1.0 - inherent) * (rate - limit) as f64) / rate as f64;
    }
    p.max(1.0e-6).min(1.0 - 1.0e-6)
}

#[derive(Debug, Clone)]
pub struct LossController {
    obs: Vec<Obs>,
    partial_packets: i64,
    partial_size: i64,
    partial_lost: HashMap<i64, i64>,
    best: Channel,
    hold: Hold,
    pad_rate: i64,
    pad_at: Option<i64>,
    result_bitrate: i64,
    result_state: LossState,
    count: i64,
    avg_loss: f64,
    acked: i64,
    delay_based: i64,
    min: i64,
    max: i64,
    limit_in_window: i64,
    last_send: Option<i64>,
    upper: Option<i64>,
    lower: Option<i64>,
    recovering_at: Option<i64>,
}

impl Default for LossController {
    fn default() -> Self {
        LossController {
            obs: vec![Obs { id: -1, size: 0, lost_size: 0, rate: 0 }; LOSS_WINDOW],
            partial_packets: 0,
            partial_size: 0,
            partial_lost: HashMap::new(),
            best: Channel { inherent: LOSS_INHERENT_INIT, bitrate: 0 },
            hold: Hold { at_us: 0, duration_us: LOSS_HOLD_INIT_US, bitrate: INF },
            pad_rate: 0,
            pad_at: None,
            result_bitrate: 0,
            result_state: LossState::Delay,
            count: 0,
            avg_loss: 0.0,
            acked: INF,
            delay_based: INF,
            min: 0,
            max: INF,
            limit_in_window: INF,
            last_send: None,
            upper: None,
            lower: None,
            recovering_at: None,
        }
    }
}

impl LossController {
    pub fn ready(&self) -> bool {
        self.best.bitrate > 0 && self.count >= LOSS_MIN_OBSERVATIONS
    }

    pub fn get(&self) -> (i64, LossState) {
        if self.ready() { (self.result_bitrate, self.result_state) } else { (self.delay_based, LossState::Delay) }
    }

    pub fn set_acked(&mut self, bps: i64) {
        self.acked = bps;
        self.calc_lower();
    }

    pub fn set_limits(&mut self, min: i64, max: i64) {
        self.min = min;
        self.max = max;
        self.calc_lower();
    }

    fn limited(&self) -> bool {
        self.result_state != LossState::Delay
    }

    fn increasing_when_limited(&self, old: i64, next: i64) -> bool {
        (old < next || (old == next && matches!(self.result_state, LossState::Increasing | LossState::Padding))) && self.limited()
    }

    fn can_keep_increasing(&self, estimate: i64, last_send: i64) -> bool {
        if LOSS_PADDING_US == 0 || self.result_state != LossState::Padding {
            return true;
        }
        self.pad_at.is_some_and(|at| at + LOSS_PADDING_US >= last_send) || self.pad_rate < estimate
    }

    fn padding(&self, last_send: i64) -> bool {
        LOSS_PADDING_US > 0 && self.pad_at.is_some_and(|at| at + LOSS_PADDING_US >= last_send)
    }

    fn upper_bound(&self) -> i64 {
        self.upper.unwrap_or(self.max)
    }

    fn lower_bound(&self) -> i64 {
        self.lower.unwrap_or(0)
    }

    fn calc_lower(&mut self) {
        let by_acked = if self.acked != INF { scale(self.acked, LOSS_LOWER_BY_ACKED) } else { 0 };
        self.lower = Some(by_acked.max(self.min));
    }

    pub fn update(&mut self, packets: &[LossPacket], delay_based: i64, in_alr: bool) {
        self.delay_based = delay_based;
        if !self.observe(packets) {
            return;
        }
        let Some(last_send) = self.last_send else { return };
        if self.best.bitrate == 0 {
            if delay_based == INF {
                return;
            }
            self.best.bitrate = delay_based;
            self.result_bitrate = delay_based;
            self.result_state = LossState::Delay;
        }
        let mut best = self.best;
        let mut max_objective = -f64::MAX;
        for mut cand in self.candidates(in_alr) {
            self.newton(&mut cand);
            let o = self.objective(&cand);
            if o > max_objective || (o == max_objective && cand.bitrate > best.bitrate) {
                max_objective = o;
                best = cand;
            }
        }
        if self.avg_loss > best.inherent && self.best.bitrate < best.bitrate {
            best.bitrate = self.best.bitrate;
        }
        if self.limited() {
            if self.recovering_at.map(|t| t + LOSS_INCREASE_WINDOW_US > last_send).unwrap_or(false) && best.bitrate > self.limit_in_window {
                best.bitrate = self.limit_in_window;
            }
            if self.increasing_when_limited(self.best.bitrate, best.bitrate) && self.acked != INF {
                let ramp = if self.hold.bitrate != INF && self.acked < scale(self.hold.bitrate, LOSS_RAMPUP_HOLD_THRESHOLD) { LOSS_RAMPUP_IN_HOLD } else { LOSS_RAMPUP };
                best.bitrate = self.best.bitrate.max(best.bitrate.min(scale(self.acked, ramp)));
                if self.result_state == LossState::Decreasing && best.bitrate == self.best.bitrate {
                    best.bitrate = self.best.bitrate + 1;
                }
            }
        }
        let bounded = self.lower_bound().max(best.bitrate.min(self.upper_bound()).min(self.delay_based));
        if bounded < best.bitrate {
            self.best.bitrate = bounded;
            self.best.inherent = 0.0;
        } else {
            self.best = best;
            self.best.bitrate = self.best.bitrate.max(self.lower_bound());
        }
        if self.result_state == LossState::Decreasing && self.hold.at_us > last_send && bounded < self.delay_based {
            self.hold.bitrate = self.lower_bound().max(self.hold.bitrate);
            self.result_bitrate = self.hold.bitrate.min(bounded);
            return;
        }
        if self.increasing_when_limited(self.result_bitrate, bounded) && self.can_keep_increasing(bounded, last_send) && bounded < self.delay_based && bounded < self.max {
            if LOSS_PADDING_US > 0 && bounded > self.pad_rate {
                self.pad_rate = bounded;
                self.pad_at = Some(last_send);
            }
            self.result_state = if LOSS_PADDING_US > 0 { LossState::Padding } else { LossState::Increasing };
        } else if bounded < self.delay_based && bounded < self.max {
            if self.result_state != LossState::Decreasing {
                self.hold = Hold { at_us: last_send + self.hold.duration_us, duration_us: LOSS_HOLD_MAX_US.min((self.hold.duration_us as f64 * LOSS_HOLD_FACTOR).round() as i64), bitrate: bounded };
            }
            self.pad_rate = 0;
            self.pad_at = None;
            self.result_state = LossState::Decreasing;
        } else {
            self.hold = Hold { at_us: 0, duration_us: LOSS_HOLD_INIT_US, bitrate: INF };
            self.pad_rate = 0;
            self.pad_at = None;
            self.result_state = LossState::Delay;
        }
        self.result_bitrate = bounded;
        if self.limited() && self.recovering_at.map(|t| t + LOSS_INCREASE_WINDOW_US < last_send).unwrap_or(true) {
            self.limit_in_window = BWE_MIN.max(scale(self.best.bitrate, LOSS_MAX_INCREASE));
            self.recovering_at = Some(last_send);
        }
    }

    fn observe(&mut self, packets: &[LossPacket]) -> bool {
        if packets.is_empty() {
            return false;
        }
        let (mut first, mut last) = (INF, 0i64);
        for p in packets {
            if p.received {
                self.partial_lost.remove(&p.seq);
            } else {
                self.partial_lost.entry(p.seq).or_insert(p.size);
            }
            self.partial_packets += 1;
            self.partial_size += p.size;
            first = first.min(p.send_us);
            last = last.max(p.send_us);
        }
        let base = *self.last_send.get_or_insert(first);
        let dur = last - base;
        if dur <= 0 || dur < LOSS_OBSERVATION_US {
            return false;
        }
        self.last_send = Some(last);
        let lost_size: i64 = self.partial_lost.values().sum();
        let o = Obs { id: self.count, size: self.partial_size, lost_size, rate: self.partial_size * 8 * 1_000_000 / dur };
        self.count += 1;
        self.obs[(o.id as usize) % LOSS_WINDOW] = o;
        self.partial_packets = 0;
        self.partial_size = 0;
        self.partial_lost.clear();
        self.avg_loss = self.average_loss();
        self.upper = Some(if self.avg_loss > LOSS_UPPER_LOSS_OFFSET {
            let b = (LOSS_UPPER_BALANCE / (self.avg_loss - LOSS_UPPER_LOSS_OFFSET)).round();
            if self.max == INF || b < self.max as f64 { b as i64 } else { self.max }
        } else {
            self.max
        });
        true
    }

    fn candidates(&self, in_alr: bool) -> Vec<Channel> {
        let mut rates: Vec<i64> = LOSS_CANDIDATES.iter().map(|f| scale(self.best.bitrate, *f)).collect();
        if self.acked != INF && (!in_alr || self.last_send.is_some_and(|t| self.padding(t))) {
            rates.push(scale(self.acked, LOSS_BACKOFF_BY_ACKED));
        }
        if self.delay_based != INF && self.delay_based > self.best.bitrate {
            rates.push(self.delay_based);
        }
        let cap = if self.limited() && self.limit_in_window != INF { self.limit_in_window } else { self.max };
        rates
            .into_iter()
            .map(|r| {
                let mut c = Channel { inherent: self.best.inherent, bitrate: r.min(self.best.bitrate.max(cap)) };
                c.inherent = self.feasible(&c);
                c
            })
            .collect()
    }

    fn weight(&self, o: &Obs, factor: f64) -> f64 {
        let mut w = 1.0;
        let mut i = self.count - 1 - o.id;
        while i > 0 {
            w *= factor;
            i -= 1;
        }
        w
    }

    fn objective(&self, c: &Channel) -> f64 {
        let mut sum = 0.0;
        let bias = self.bias(c.bitrate);
        for o in &self.obs {
            if o.id == -1 {
                continue;
            }
            let p = loss_probability(c.inherent, c.bitrate, o.rate);
            let w = self.weight(o, LOSS_TEMPORAL_WEIGHT);
            sum += w * ((o.lost_size as f64 / 1000.0) * p.ln() + ((o.size - o.lost_size) as f64 / 1000.0) * (1.0 - p).ln());
            sum += w * bias * (o.size as f64 / 1000.0);
        }
        sum
    }

    fn newton(&self, c: &mut Channel) {
        if self.count <= 0 {
            return;
        }
        for _ in 0..LOSS_NEWTON_ITERATIONS {
            let (mut d1, mut d2) = (0.0f64, 0.0f64);
            for o in &self.obs {
                if o.id == -1 {
                    continue;
                }
                let p = loss_probability(c.inherent, c.bitrate, o.rate);
                let w = self.weight(o, LOSS_TEMPORAL_WEIGHT);
                d1 += w * (o.lost_size as f64 / 1000.0 / p - (o.size - o.lost_size) as f64 / 1000.0 / (1.0 - p));
                d2 -= w * (o.lost_size as f64 / 1000.0 / (p * p) + (o.size - o.lost_size) as f64 / 1000.0 / ((1.0 - p) * (1.0 - p)));
            }
            if d2 >= 0.0 {
                d2 = -1.0e-6;
            }
            c.inherent -= (LOSS_NEWTON_STEP * d1) / d2;
            c.inherent = self.feasible(c);
        }
    }

    fn feasible(&self, c: &Channel) -> f64 {
        let upper = if c.bitrate == 0 { 1.0 } else { (LOSS_INHERENT_UPPER_OFFSET + LOSS_INHERENT_UPPER_BALANCE / c.bitrate as f64).min(1.0) };
        c.inherent.max(LOSS_INHERENT_MIN).min(upper)
    }

    fn bias(&self, bitrate: i64) -> f64 {
        if bitrate <= 0 {
            return 0.0;
        }
        let adj = |f: f64| (f * (LOSS_BIAS_LOSS_THRESHOLD - self.avg_loss)) / (LOSS_BIAS_SMOOTHING + (LOSS_BIAS_LOSS_THRESHOLD - self.avg_loss).abs());
        let kbps = (bitrate as f64 / 1000.0).round();
        adj(LOSS_BIAS_LINEAR) * kbps + adj(LOSS_BIAS_LOG) * (1.0 + kbps).ln()
    }

    fn average_loss(&self) -> f64 {
        if self.count <= 0 {
            return 0.0;
        }
        let (mut total, mut lost) = (0i64, 0i64);
        let (mut min_rate, mut max_rate) = (1.0f64, 0.0f64);
        let (mut min_lost, mut max_lost, mut min_bytes, mut max_bytes, mut rate_of_max) = (0i64, 0i64, 0i64, 0i64, 0i64);
        for o in &self.obs {
            if o.id == -1 {
                continue;
            }
            let w = self.weight(o, LOSS_UPPER_TEMPORAL_WEIGHT);
            total += (w * o.size as f64).round() as i64;
            lost += (w * o.lost_size as f64).round() as i64;
            let r = if o.size != 0 { o.lost_size as f64 / o.size as f64 } else { 0.0 };
            if self.count <= 3 {
                continue;
            }
            if r > max_rate {
                max_rate = r;
                max_lost = (w * o.lost_size as f64).round() as i64;
                max_bytes = (w * o.size as f64).round() as i64;
                rate_of_max = o.rate;
            }
            if r < min_rate {
                min_rate = r;
                min_lost = (w * o.lost_size as f64).round() as i64;
                min_bytes = (w * o.size as f64).round() as i64;
            }
        }
        if total == 0 {
            return 0.0;
        }
        if scale(self.median_rate(), LOSS_MEDIAN_SENDING_FACTOR) <= rate_of_max {
            return lost as f64 / total as f64;
        }
        if total == max_bytes + min_bytes {
            return lost as f64 / total as f64;
        }
        (lost - min_lost - max_lost) as f64 / (total - max_bytes - min_bytes) as f64
    }

    fn median_rate(&self) -> i64 {
        let mut r: Vec<i64> = self.obs.iter().filter(|o| o.id != -1 && o.rate != 0).map(|o| o.rate).collect();
        if r.is_empty() {
            return 0;
        }
        r.sort();
        if r.len() % 2 == 0 { (r[r.len() / 2 - 1] + r[r.len() / 2]) / 2 } else { r[(r.len() - 1) / 2] }
    }
}

pub const TARGET_MAX_BPS: i64 = 1_000_000_000;
pub const TARGET_MAX_RTT_US: i64 = 3_000_000;
pub const TARGET_RTT_DROP: f64 = 0.8;
pub const TARGET_RTT_DROP_INTERVAL_US: i64 = 1_000_000;
pub const TARGET_RTT_FLOOR_BPS: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Start,
    Delay,
    Loss,
    Rtt,
    Probe,
    Keep,
}

impl By {
    pub fn name(self) -> &'static str {
        match self {
            By::Start => "start",
            By::Delay => "delay",
            By::Loss => "loss",
            By::Rtt => "rtt",
            By::Probe => "probe",
            By::Keep => "keep",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TargetRate {
    loss: LossController,
    current: i64,
    min: i64,
    max: i64,
    delay_limit: i64,
    pub loss_state: LossState,
    pub by: By,
    last_decrease_at: Option<i64>,
    rtt_at: Option<i64>,
    rtt_us: i64,
    sent_at: Option<i64>,
}

impl Default for TargetRate {
    fn default() -> Self {
        let mut loss = LossController::default();
        loss.set_limits(BWE_MIN, TARGET_MAX_BPS);
        TargetRate { loss, current: 0, min: BWE_MIN, max: TARGET_MAX_BPS, delay_limit: INF, loss_state: LossState::Delay, by: By::Keep, last_decrease_at: None, rtt_at: None, rtt_us: 0, sent_at: None }
    }
}

impl TargetRate {
    pub fn target(&self) -> i64 {
        self.min.max(self.current)
    }

    pub fn restart(&mut self) {
        self.loss = LossController::default();
        self.loss.set_limits(self.min, self.max);
        self.loss_state = LossState::Delay;
        self.rtt_at = None;
        self.rtt_us = 0;
        self.sent_at = None;
    }

    pub fn set_limits(&mut self, min: i64, max: i64) {
        self.min = min.max(BWE_MIN);
        self.max = if max > 0 && max != INF { self.min.max(max) } else { TARGET_MAX_BPS };
        self.loss.set_limits(self.min, self.max);
    }

    pub fn set_acked(&mut self, bps: i64) {
        self.loss.set_acked(bps);
    }

    pub fn set_send(&mut self, bps: i64) {
        self.delay_limit = INF;
        self.set(bps);
        self.by = By::Probe;
    }

    pub fn set_delay_based(&mut self, bps: i64) {
        self.delay_limit = if bps == 0 { INF } else { bps };
        self.set(self.current);
    }

    pub fn propagation_rtt(&mut self, us: i64, now_us: i64) {
        self.rtt_at = Some(now_us);
        self.rtt_us = us;
    }

    pub fn on_sent(&mut self, sent_us: i64) {
        self.sent_at = Some(sent_us);
    }

    pub fn rtt_above(&self) -> bool {
        let wait = match (self.sent_at, self.rtt_at) {
            (Some(s), Some(r)) => (s - r).max(0),
            _ => 0,
        };
        wait + self.rtt_us > TARGET_MAX_RTT_US
    }

    pub fn on_feedback(&mut self, packets: &[LossPacket], in_alr: bool, now_us: i64) {
        self.loss.update(packets, self.delay_limit, in_alr);
        self.update(now_us);
    }

    pub fn update(&mut self, now_us: i64) {
        if self.rtt_above() {
            if self.last_decrease_at.map(|t| now_us - t >= TARGET_RTT_DROP_INTERVAL_US).unwrap_or(true) && self.current > TARGET_RTT_FLOOR_BPS {
                self.last_decrease_at = Some(now_us);
                self.set(scale(self.current, TARGET_RTT_DROP).max(TARGET_RTT_FLOOR_BPS));
                self.by = By::Rtt;
            } else {
                self.set(self.current);
            }
            return;
        }
        if !self.loss.ready() {
            let bitrate = if self.delay_limit != INF { self.delay_limit.max(self.current) } else { self.current };
            if bitrate != self.current {
                self.set(bitrate);
                self.by = By::Start;
            } else {
                self.set(self.current);
            }
            return;
        }
        let (bitrate, state) = self.loss.get();
        self.loss_state = state;
        self.set(bitrate);
        self.by = if state == LossState::Delay { By::Delay } else { By::Loss };
    }

    fn set(&mut self, bps: i64) {
        self.current = self.min.max(bps.min(self.delay_limit).min(self.max));
    }
}

pub const CONGESTION_START_BPS: i64 = 600_000;
pub const CONGESTION_PROCESS_US: i64 = 100_000;
pub const CONGESTION_PROBE_DROP_THROUGHPUT: f64 = 0.85;
pub const CONGESTION_STREAM_TIMEOUT_US: i64 = 2_000_000;

#[derive(Debug, Clone, Copy)]
pub struct Fed {
    pub seq: i64,
    pub send_us: i64,
    pub size: i64,
    pub recv_us: Option<i64>,
    pub tag: Option<Tag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub target: Option<i64>,
    pub by: By,
    pub opened: Vec<Cluster>,
    pub judged: Option<Measured>,
}

#[derive(Debug, Clone, Copy)]
struct Got {
    send_us: i64,
    recv_us: i64,
    size: i64,
    tag: Option<Tag>,
}

struct Delayed {
    updated: bool,
    probe: bool,
    target: i64,
    recovered: bool,
}

#[derive(Debug, Clone)]
pub struct DelayBased {
    arrival: InterArrival,
    pub trend: Trendline,
    pub aimd: Aimd,
    last_seen: Option<i64>,
    pub prev_bitrate: i64,
    pub prev_state: Usage,
}

impl Default for DelayBased {
    fn default() -> Self {
        DelayBased { arrival: InterArrival::default(), trend: Trendline::new(), aimd: Aimd::new(), last_seen: None, prev_bitrate: 0, prev_state: Usage::Normal }
    }
}

impl DelayBased {
    fn process(&mut self, now_us: i64) {
        if self.last_seen.is_none_or(|t| now_us - t <= CONGESTION_STREAM_TIMEOUT_US) {
            return;
        }
        self.arrival = InterArrival::default();
        self.trend = Trendline::new();
        self.last_seen = None;
        self.prev_state = Usage::Normal;
    }

    fn on_feedback(&mut self, got: &[Got], feedback_us: i64, acked: Option<i64>, probe: Option<i64>) -> Delayed {
        let mut r = Delayed { updated: false, probe: false, target: 0, recovered: false };
        if got.is_empty() {
            return r;
        }
        let mut recovered = false;
        let mut prev = self.trend.state();
        for p in got {
            if self.last_seen.map(|t| feedback_us - t > CONGESTION_STREAM_TIMEOUT_US).unwrap_or(true) {
                self.arrival = InterArrival::default();
                self.trend = Trendline::new();
            }
            self.last_seen = Some(feedback_us);
            if let Some((recv_d, send_d)) = self.arrival.push(p.send_us, p.recv_us, feedback_us) {
                self.trend.update(recv_d, send_d, p.recv_us as f64 / 1000.0);
            }
            if prev == Usage::Underuse && self.trend.state() == Usage::Normal {
                recovered = true;
            }
            prev = self.trend.state();
        }
        let usage = self.trend.state();
        if usage == Usage::Overuse {
            match acked {
                Some(k) => {
                    if self.aimd.time_to_reduce_further(feedback_us, k) {
                        r.target = self.aimd.update(feedback_us, Usage::Overuse, Some(k));
                        r.updated = self.aimd.valid;
                    }
                }
                None => {
                    if self.aimd.valid && self.aimd.initial_time_to_reduce_further(feedback_us) {
                        self.aimd.set_estimate(self.aimd.rate / 2, feedback_us);
                        r.updated = true;
                        r.target = self.aimd.rate;
                    }
                }
            }
        } else if let Some(b) = probe {
            r.probe = true;
            r.updated = true;
            self.aimd.set_estimate(b, feedback_us);
            r.target = self.aimd.rate;
        } else {
            r.target = self.aimd.update(feedback_us, usage, acked);
            r.updated = self.aimd.valid;
            r.recovered = recovered;
        }
        if (r.updated && self.prev_bitrate != r.target) || usage != self.prev_state {
            if r.updated {
                self.prev_bitrate = r.target;
            }
            self.prev_state = usage;
        }
        r
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Notified {
    target: i64,
    state: LossState,
    limited: bool,
}

#[derive(Debug, Clone)]
pub struct Congestion {
    pub delay: DelayBased,
    pub throughput: Throughput,
    pub rate: TargetRate,
    pub alr: Alr,
    pub probes: ProbeController,
    pub prober: Prober,
    pub estimator: ProbeEstimator,
    pub padder: Padder,
    quiet_at: Option<i64>,
    idle: bool,
    first_sent: bool,
    first_sent_at: Option<i64>,
    first_feedback: bool,
    prev_in_alr: bool,
    offset_us: Option<i64>,
    last: Option<Notified>,
    opened: Vec<Cluster>,
}

impl Congestion {
    pub fn new(start_bps: i64, now_us: i64) -> Congestion {
        let start = start_bps.max(BWE_MIN);
        let mut c = Congestion {
            delay: DelayBased::default(),
            throughput: Throughput::default(),
            rate: TargetRate::default(),
            alr: Alr::default(),
            probes: ProbeController::default(),
            prober: Prober::default(),
            estimator: ProbeEstimator::default(),
            padder: Padder::default(),
            quiet_at: None,
            idle: false,
            first_sent: false,
            first_sent_at: None,
            first_feedback: false,
            prev_in_alr: false,
            offset_us: None,
            last: None,
            opened: Vec::new(),
        };
        c.rate.set_limits(BWE_MIN, INF);
        c.delay.aimd.set_min(BWE_MIN);
        c.rate.set_send(start);
        c.rate.by = By::Start;
        c.delay.aimd.set_start(start);
        let opened = c.probes.set_bitrates(BWE_MIN, start, INF, now_us);
        c.open(opened);
        c
    }

    pub fn available(&self) -> i64 {
        self.rate.target()
    }

    pub fn twccless(&self, now_us: i64) -> bool {
        !self.first_feedback && self.first_sent_at.is_some_and(|t| now_us - t > CONGESTION_STREAM_TIMEOUT_US)
    }

    pub fn padding(&mut self, now_us: i64) -> i64 {
        let rate = if self.rate.loss_state == LossState::Padding { self.rate.target() } else { 0 };
        self.padder.due(now_us, rate)
    }

    pub fn set_network(&mut self, available: bool, now_us: i64) -> Step {
        let opened = self.probes.on_network(available, now_us);
        self.open(opened);
        self.step(None, None, None)
    }

    pub fn set_desired(&mut self, bps: i64, now_us: i64) -> Step {
        let opened = self.probes.on_max_allocated(bps, now_us);
        self.open(opened);
        self.step(None, None, None)
    }

    pub fn on_rtt(&mut self, rtt_us: i64) {
        let r = rtt_us / 1000 * 1000;
        if r > 0 {
            self.delay.aimd.rtt_us = r;
        }
    }

    pub fn on_sent(&mut self, size: i64, sent_us: i64, now_us: i64) {
        self.alr.on_sent(size, sent_us, now_us);
        self.padder.on_sent(size);
        self.quiet_at = Some(sent_us);
        self.idle = false;
        if !self.first_sent {
            self.first_sent = true;
            self.first_sent_at = Some(sent_us);
            self.rate.propagation_rtt(0, sent_us);
        }
        self.rate.on_sent(sent_us);
    }

    pub fn on_feedback(&mut self, fed: &[Fed], feedback_us: i64, base_us: i64) -> Step {
        if fed.is_empty() {
            return self.step(None, None, None);
        }
        self.first_feedback = true;
        self.quiet_at = Some(feedback_us);
        self.idle = false;
        let offset = *self.offset_us.get_or_insert(feedback_us - base_us);
        let mut got: Vec<Got> = fed.iter().filter_map(|p| p.recv_us.map(|r| Got { send_us: p.send_us, recv_us: r + offset, size: p.size, tag: p.tag })).collect();
        got.sort_by_key(|p| p.recv_us);
        if let Some(last) = got.last() {
            let max_recv = last.recv_us;
            let rtt = got.iter().map(|p| feedback_us - p.send_us - (max_recv - p.recv_us)).min().unwrap_or(0);
            self.rate.propagation_rtt(rtt, feedback_us);
        }
        let alr_start = self.alr.started_at;
        if self.prev_in_alr && alr_start.is_none() {
            self.probes.alr_ended = Some(feedback_us);
        }
        self.prev_in_alr = alr_start.is_some();
        for p in &got {
            self.throughput.push(p.send_us, p.recv_us, p.size);
        }
        self.throughput.trim();
        let acked = self.throughput.bitrate();
        if let Some(k) = acked {
            self.rate.set_acked(k);
        }
        for p in &got {
            if let Some(tag) = p.tag {
                self.estimator.handle(tag, p.send_us, p.recv_us, p.size);
            }
        }
        let mut probe = self.estimator.fetch();
        let judged = if probe.is_some() { self.estimator.last } else { None };
        if let (Some(b), Some(k)) = (probe, acked) {
            probe = Some(b.max(self.delay.prev_bitrate.min(scale(k, CONGESTION_PROBE_DROP_THROUGHPUT))));
        }
        let r = self.delay.on_feedback(&got, feedback_us, acked, probe);
        if r.updated {
            if r.probe {
                self.rate.set_send(r.target);
            }
            self.rate.set_delay_based(r.target);
        }
        let packets: Vec<LossPacket> = fed.iter().map(|p| LossPacket { seq: p.seq, send_us: p.send_us, size: p.size, received: p.recv_us.is_some() }).collect();
        self.rate.on_feedback(&packets, alr_start.is_some(), feedback_us);
        let mut notify = None;
        if r.updated {
            notify = self.notify(feedback_us);
        }
        if r.recovered {
            self.probes.alr_start = alr_start;
            let opened = self.probes.request_probe(feedback_us);
            self.open(opened);
        }
        self.step(notify, judged, if r.probe { Some(By::Probe) } else { None })
    }

    pub fn process(&mut self, now_us: i64) -> Step {
        self.delay.process(now_us);
        if !self.idle && self.quiet_at.is_some_and(|t| now_us - t > CONGESTION_STREAM_TIMEOUT_US) {
            self.idle = true;
            self.rate.restart();
        }
        if self.alr.last_sent.is_some_and(|t| now_us - t > CONGESTION_PROCESS_US) {
            self.alr.on_sent(0, now_us, now_us);
        }
        self.rate.update(now_us);
        self.probes.alr_start = if self.first_feedback { self.alr.started_at } else { None };
        self.probes.limit = self.limit();
        let opened = self.probes.process(now_us);
        self.open(opened);
        let notify = self.notify(now_us);
        self.step(notify, None, None)
    }

    fn notify(&mut self, now_us: i64) -> Option<i64> {
        let n = Notified { target: self.rate.target(), state: self.rate.loss_state, limited: self.alr.started_at.is_none() };
        if self.last == Some(n) {
            return None;
        }
        self.last = Some(n);
        self.alr.set_estimate(n.target);
        let limit = self.limit();
        let opened = self.probes.set_estimate(n.target, limit, now_us);
        self.open(opened);
        Some(n.target)
    }

    pub fn limit(&self) -> Limit {
        let usage = self.delay.prev_state;
        if usage == Usage::Overuse || usage == Usage::Underuse {
            Limit::DelayIncreased
        } else if self.rate.rtt_above() {
            Limit::Rtt
        } else {
            match self.rate.loss_state {
                LossState::Decreasing | LossState::Padding => Limit::Loss,
                LossState::Increasing => Limit::LossIncreasing,
                LossState::Delay => Limit::Delay,
            }
        }
    }

    fn open(&mut self, clusters: Vec<Cluster>) {
        for c in clusters {
            self.prober.create(&c);
            self.opened.push(c);
        }
    }

    fn step(&mut self, target: Option<i64>, judged: Option<Measured>, by: Option<By>) -> Step {
        Step { target, by: by.unwrap_or(self.rate.by), opened: std::mem::take(&mut self.opened), judged }
    }
}
