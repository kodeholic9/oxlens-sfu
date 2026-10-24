// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-SIM-MODES · 정:RULE-SIM-TICK · 정:IMPL-SIM-SWITCH · 정:CAND-SIM-PROBE-SHAPE · 연:RULE-SUB-LAYER-PROC · 연:RULE-SUB-LAYER-PAUSED-AXIS · model: claude-opus-5-5
//! 영역 SIM — 구독 전송로마다 판단 재료(egress twcc 송신 기록 · GCC 추정 · REMB · RR 손실 · NACK · 드롭) · 레이어 판단 · 프로브 상태.

pub mod core;

use std::collections::{BTreeMap, VecDeque};

use self::core::{Aimd, Overuse, Policy, Signals, Trendline, Usage, DECREASE_GAP_MS, START_BPS, V2_STALE_MS};

pub const SEND_LOG_MAX: usize = 4096;
pub const BURST_US: u64 = 5_000;
pub const ACKED_WINDOW_MS: u64 = 500;

#[derive(Debug, Clone, Default)]
pub struct SendLog {
    map: BTreeMap<u16, (u64, usize, bool)>,
}

impl SendLog {
    pub fn record(&mut self, seq: u16, send_us: u64, bytes: usize, probe: bool) {
        self.map.insert(seq, (send_us, bytes, probe));
        if self.map.len() > SEND_LOG_MAX
            && let Some(&k) = self.map.keys().next()
        {
            self.map.remove(&k);
        }
    }

    pub fn take(&mut self, seq: u16) -> Option<(u64, usize, bool)> {
        self.map.remove(&seq)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub started_ms: u64,
    pub rate_bps: u64,
    pub need_bps: u64,
    pub bytes_acked: u64,
    pub sent: u64,
    pub first_arrival_us: Option<i64>,
    pub last_arrival_us: Option<i64>,
}

impl Probe {
    pub fn measured_bps(&self) -> Option<u64> {
        let (a, b) = (self.first_arrival_us?, self.last_arrival_us?);
        let span = (b - a).max(0) as u64;
        if span < 100_000 || self.bytes_acked == 0 {
            return None;
        }
        Some(self.bytes_acked * 8 * 1_000_000 / span)
    }
}

#[derive(Debug, Clone)]
pub struct Bwe {
    pub aimd: Aimd,
    trend: Trendline,
    over: Overuse,
    pub policy: Policy,
    pub remb: Option<(u64, u64)>,
    pub loss: Option<(f64, u64)>,
    pub nacks: u64,
    pub drops: u64,
    pub last_fb_ms: u64,
    acked: VecDeque<(u64, usize)>,
    group: Option<(u64, u64, i64)>,
    prev_group: Option<(u64, i64)>,
    pub probe: Option<Probe>,
    pub probe_result_bps: Option<u64>,
}

impl Default for Bwe {
    fn default() -> Self {
        Bwe {
            aimd: Aimd::new(START_BPS),
            trend: Trendline::default(),
            over: Overuse::default(),
            policy: Policy::default(),
            remb: None,
            loss: None,
            nacks: 0,
            drops: 0,
            last_fb_ms: 0,
            acked: VecDeque::new(),
            group: None,
            prev_group: None,
            probe: None,
            probe_result_bps: None,
        }
    }
}

impl Bwe {
    pub fn acked_bps(&self, now_ms: u64) -> u64 {
        let bytes: usize = self.acked.iter().filter(|(t, _)| now_ms.saturating_sub(*t) <= ACKED_WINDOW_MS).map(|(_, b)| *b).sum();
        (bytes as u64 * 8 * 1000) / ACKED_WINDOW_MS
    }

    pub fn on_feedback(&mut self, pkts: &[(u64, i64, usize, bool)], now_ms: u64) {
        self.last_fb_ms = now_ms;
        let mut overused = false;
        for &(send_us, arrival_us, bytes, probe) in pkts {
            self.acked.push_back((now_ms, bytes));
            if probe && let Some(p) = self.probe.as_mut() {
                p.bytes_acked += bytes as u64;
                p.first_arrival_us.get_or_insert(arrival_us);
                p.last_arrival_us = Some(arrival_us);
            }
            match self.group {
                None => self.group = Some((send_us, send_us, arrival_us)),
                Some((first, _, arr)) if send_us.saturating_sub(first) <= BURST_US => {
                    self.group = Some((first, send_us, arr.max(arrival_us)));
                }
                Some((_, last_send, last_arr)) => {
                    if let Some((ps, pa)) = self.prev_group {
                        let recv_d = (last_arr - pa) as f64 / 1000.0;
                        let send_d = last_send.saturating_sub(ps) as f64 / 1000.0;
                        if let Some(m) = self.trend.update(recv_d, send_d, last_arr as f64 / 1000.0) {
                            let u = self.over.detect(m, send_d, now_ms as f64);
                            overused |= u == Usage::Overuse;
                        }
                    }
                    self.prev_group = Some((last_send, last_arr));
                    self.group = Some((send_us, send_us, arrival_us));
                }
            }
        }
        while let Some(&(t, _)) = self.acked.front() {
            if now_ms.saturating_sub(t) > ACKED_WINDOW_MS * 4 {
                self.acked.pop_front();
            } else {
                break;
            }
        }
        let usage = if overused { Usage::Overuse } else { self.over.usage };
        let acked = self.acked_bps(now_ms);
        if usage == Usage::Overuse && self.aimd.last_decrease_ms.map(|t| now_ms.saturating_sub(t) < DECREASE_GAP_MS).unwrap_or(false) {
            return;
        }
        self.aimd.update(usage, acked.max(1), now_ms);
    }

    pub fn signals(&self, now_ms: u64, v2: bool, tracks: u32) -> Signals {
        let v2_fresh = v2 && now_ms.saturating_sub(self.last_fb_ms) <= V2_STALE_MS && self.last_fb_ms > 0;
        let (est, est_at) = if v2_fresh {
            (Some(self.probe_result_bps.unwrap_or(0).max(self.aimd.rate())), now_ms)
        } else {
            match self.remb {
                Some((b, at)) => (Some(b), at),
                None => (None, 0),
            }
        };
        Signals {
            estimate_bps: est,
            estimate_at_ms: est_at,
            loss: self.loss.map(|l| l.0),
            loss_at_ms: self.loss.map(|l| l.1).unwrap_or(0),
            nack_per_s: self.nacks as f64,
            drops: self.drops,
            overusing: v2_fresh && self.over.usage == Usage::Overuse,
            v2_fresh,
            tracks,
        }
    }
}
