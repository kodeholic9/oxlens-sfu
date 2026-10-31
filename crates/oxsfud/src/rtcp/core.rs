// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-RTCP-TERMINATE · 정:RULE-CONC-SANS-IO · 정:CAND-RTCP-TWCC-HISTORY · model: claude-opus-5-5
//! RTCP 산수 — 수신 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 피드백 창(draft-holmer §3.1 · libwebrtc 창). 시각은 인자.

use std::collections::BTreeMap;

const MIN_SEQUENTIAL: u32 = 2;
const MAX_DROPOUT: u32 = 3000;
const MAX_MISORDER: u32 = 100;
const SEQ_MOD: u32 = 1 << 16;

#[derive(Debug, Clone, Default)]
pub struct RecvStats {
    pub clock: u32,
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    probation: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    transit: Option<u32>,
    jitter: f64,
    started: bool,
    pub packets: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub fraction: u8,
    pub lost: i32,
    pub extended_max: u32,
    pub jitter: u32,
}

impl RecvStats {
    pub fn new(clock: u32) -> RecvStats {
        RecvStats { clock, ..Default::default() }
    }

    fn init_seq(&mut self, seq: u16) {
        self.base_seq = seq as u32;
        self.max_seq = seq;
        self.bad_seq = SEQ_MOD + 1;
        self.cycles = 0;
        self.received = 0;
        self.received_prior = 0;
        self.expected_prior = 0;
    }

    fn update_seq(&mut self, seq: u16) -> bool {
        if !self.started {
            self.started = true;
            self.init_seq(seq);
            self.max_seq = seq.wrapping_sub(1);
            self.probation = MIN_SEQUENTIAL;
        }
        let udelta = seq.wrapping_sub(self.max_seq) as u32;
        if self.probation > 0 {
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    self.received += 1;
                    return true;
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            return false;
        }
        if udelta < MAX_DROPOUT {
            if seq < self.max_seq {
                self.cycles += SEQ_MOD;
            }
            self.max_seq = seq;
        } else if udelta <= SEQ_MOD - MAX_MISORDER {
            if seq as u32 == self.bad_seq {
                self.init_seq(seq);
            } else {
                self.bad_seq = (seq as u32 + 1) & (SEQ_MOD - 1);
                return false;
            }
        }
        self.received += 1;
        true
    }

    pub fn update(&mut self, seq: u16, ts: u32, arrival_ms: u64, bytes: usize) -> bool {
        let valid = self.update_seq(seq);
        self.packets += 1;
        self.bytes += bytes as u64;
        let arrival = ((arrival_ms as u128 * self.clock as u128 / 1000) & 0xffff_ffff) as u32;
        let transit = arrival.wrapping_sub(ts);
        if let Some(prev) = self.transit {
            let d = (transit.wrapping_sub(prev) as i32).unsigned_abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = Some(transit);
        valid
    }

    pub fn extended_max(&self) -> u32 {
        self.cycles + self.max_seq as u32
    }

    pub fn report(&mut self) -> Option<Report> {
        if !self.started || self.probation > 0 {
            return None;
        }
        let ext = self.extended_max();
        let expected = ext.wrapping_sub(self.base_seq).wrapping_add(1);
        let lost = (expected as i64 - self.received as i64).clamp(-0x80_0000, 0x7f_ffff) as i32;
        let expected_interval = expected.wrapping_sub(self.expected_prior);
        self.expected_prior = expected;
        let received_interval = self.received.wrapping_sub(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = expected_interval as i64 - received_interval as i64;
        let fraction = if expected_interval == 0 || lost_interval <= 0 {
            0
        } else {
            ((lost_interval << 8) / expected_interval as i64).min(255) as u8
        };
        Some(Report { fraction, lost, extended_max: ext, jitter: self.jitter as u32 })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    pub base_seq: u16,
    pub status_count: u16,
    pub reference_time: u32,
    pub fb_count: u8,
    pub symbols: Vec<u8>,
    pub deltas_250us: Vec<i32>,
}

pub const SYM_NONE: u8 = 0;
pub const SYM_SMALL: u8 = 1;
pub const SYM_LARGE: u8 = 2;

#[derive(Debug, Clone, Default)]
pub struct TwccRecv {
    arrivals: BTreeMap<i64, i64>,
    last_unwrapped: Option<i64>,
    window_start: Option<i64>,
    fb_count: u8,
    newest_us: i64,
}

pub const KEEP_US: i64 = 500_000;

impl TwccRecv {
    fn unwrap(&mut self, seq: u16) -> i64 {
        let u = match self.last_unwrapped {
            None => seq as i64,
            Some(l) => {
                let d = seq.wrapping_sub(l as u16) as i16 as i64;
                l + d
            }
        };
        if self.last_unwrapped.map(|l| u > l).unwrap_or(true) {
            self.last_unwrapped = Some(u);
        }
        u
    }

    pub fn record(&mut self, seq: u16, arrival_us: i64) {
        let u = self.unwrap(seq);
        self.arrivals.entry(u).or_insert(arrival_us);
        if arrival_us > self.newest_us {
            self.newest_us = arrival_us;
        }
        match self.window_start {
            Some(w) if u < w => self.window_start = Some(u),
            None => self.window_start = Some(u),
            _ => {}
        }
        let horizon = self.newest_us - KEEP_US;
        let ws = self.window_start.unwrap_or(i64::MIN);
        self.arrivals.retain(|k, t| *t >= horizon || *k >= ws);
    }

    pub fn pending(&self) -> bool {
        let Some(w) = self.window_start else { return false };
        self.arrivals.range(w..).next().is_some()
    }

    pub fn build(&mut self) -> Option<Feedback> {
        let w = self.window_start?;
        let (&first_seq, &first_t) = self.arrivals.range(w..).next()?;
        let reference_time = first_t.div_euclid(64_000);
        let mut last = reference_time * 64_000;
        let base = w;
        let mut symbols = Vec::new();
        let mut deltas = Vec::new();
        let mut end = base - 1;
        let max_seq = self.arrivals.keys().next_back().copied().unwrap_or(first_seq);
        let mut s = base;
        while s <= max_seq && symbols.len() < 0xffff {
            match self.arrivals.get(&s) {
                None => symbols.push(SYM_NONE),
                Some(&t) => {
                    let d = (t - last + 125).div_euclid(250);
                    if !(-32768..=32767).contains(&d) {
                        break;
                    }
                    symbols.push(if (0..=255).contains(&d) { SYM_SMALL } else { SYM_LARGE });
                    deltas.push(d as i32);
                    last += d * 250;
                }
            }
            end = s;
            s += 1;
        }
        while symbols.last() == Some(&SYM_NONE) {
            symbols.pop();
            end -= 1;
        }
        if symbols.is_empty() {
            return None;
        }
        let fb = Feedback {
            base_seq: base as u16,
            status_count: symbols.len() as u16,
            reference_time: (reference_time as u32) & 0x00ff_ffff,
            fb_count: self.fb_count,
            symbols,
            deltas_250us: deltas,
        };
        self.fb_count = self.fb_count.wrapping_add(1);
        self.window_start = Some(end + 1);
        Some(fb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twcc_small_run() {
        let mut t = TwccRecv::default();
        for (s, a) in [(10u16, 1_000_000i64), (11, 1_020_000), (12, 1_040_000)] {
            t.record(s, a);
        }
        let f = t.build().unwrap();
        assert_eq!((f.base_seq, f.status_count, f.reference_time), (10, 3, 15));
        assert_eq!(f.deltas_250us, vec![160, 80, 80]);
    }
}
