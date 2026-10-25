// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-RTCP-TERMINATE 정:RULE-RTCP-NACK-UP 정:RULE-CONC-SANS-IO 정:CAND-RTCP-TWCC-HISTORY · model: claude-opus-5-5
//! RTCP 산수 — RR 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 피드백 조립(draft-holmer §3.1 · 창은 libwebrtc 수신 쪽) · 상향 NACK 결손 추적. 시계 · I/O 없음.

use std::collections::BTreeMap;

const RTP_SEQ_MOD: u32 = 1 << 16;
const MAX_DROPOUT: u32 = 3000;
const MAX_MISORDER: u32 = 100;
const MIN_SEQUENTIAL: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub fraction: u8,
    pub lost: i32,
    pub extended_max: u32,
    pub jitter: u32,
}

#[derive(Debug, Clone)]
pub struct RecvStats {
    clock: u32,
    started: bool,
    probation: u32,
    base_seq: u32,
    max_seq: u16,
    bad_seq: u32,
    cycles: u32,
    received: u32,
    received_prior: u32,
    expected_prior: u32,
    transit: Option<i64>,
    jitter: f64,
    pub packets: u64,
    pub bytes: u64,
}

impl RecvStats {
    pub fn new(clock: u32) -> RecvStats {
        RecvStats {
            clock,
            started: false,
            probation: 0,
            base_seq: 0,
            max_seq: 0,
            bad_seq: RTP_SEQ_MOD + 1,
            cycles: 0,
            received: 0,
            received_prior: 0,
            expected_prior: 0,
            transit: None,
            jitter: 0.0,
            packets: 0,
            bytes: 0,
        }
    }

    fn init_seq(&mut self, seq: u16) {
        self.base_seq = u32::from(seq);
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD + 1;
        self.cycles = 0;
        self.received = 0;
        self.received_prior = 0;
        self.expected_prior = 0;
    }

    fn update_seq(&mut self, seq: u16) -> bool {
        let udelta = u32::from(seq.wrapping_sub(self.max_seq));
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
        } else if udelta < MAX_DROPOUT {
            if seq < self.max_seq {
                self.cycles = self.cycles.wrapping_add(RTP_SEQ_MOD);
            }
            self.max_seq = seq;
        } else if udelta <= RTP_SEQ_MOD - MAX_MISORDER {
            if u32::from(seq) == self.bad_seq {
                self.init_seq(seq);
            } else {
                self.bad_seq = (u32::from(seq) + 1) & (RTP_SEQ_MOD - 1);
                return false;
            }
        }
        self.received += 1;
        true
    }

    pub fn update(&mut self, seq: u16, ts: u32, arrival_ms: u64, len: usize) -> bool {
        self.packets += 1;
        self.bytes += len as u64;
        if !self.started {
            self.started = true;
            self.init_seq(seq);
            self.max_seq = seq.wrapping_sub(1);
            self.probation = MIN_SEQUENTIAL;
        }
        let valid = self.update_seq(seq);
        let arrival = ((arrival_ms as u128 * self.clock as u128) / 1000) as u32;
        let transit = i64::from(arrival.wrapping_sub(ts) as i32);
        if let Some(prev) = self.transit {
            let d = (transit - prev).abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = Some(transit);
        valid
    }

    pub fn extended_max(&self) -> u32 {
        self.cycles.wrapping_add(u32::from(self.max_seq))
    }

    pub fn report(&mut self) -> Report {
        let ext = self.extended_max();
        let expected = ext.wrapping_sub(self.base_seq).wrapping_add(1);
        let lost = (i64::from(expected) - i64::from(self.received)).clamp(-0x80_0000, 0x7f_ffff) as i32;
        let expected_interval = expected.wrapping_sub(self.expected_prior);
        self.expected_prior = expected;
        let received_interval = self.received.wrapping_sub(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = i64::from(expected_interval) - i64::from(received_interval);
        let fraction = if expected_interval == 0 || lost_interval <= 0 { 0 } else { ((lost_interval << 8) / i64::from(expected_interval)).min(255) as u8 };
        Report { fraction, lost, extended_max: ext, jitter: self.jitter as u32 }
    }

    pub fn lost(&self) -> i64 {
        let expected = self.extended_max().wrapping_sub(self.base_seq).wrapping_add(1);
        if self.started { i64::from(expected) - i64::from(self.received) } else { 0 }
    }

    pub fn jitter_secs(&self) -> f64 {
        if self.clock == 0 { 0.0 } else { self.jitter / f64::from(self.clock) }
    }

    pub fn started(&self) -> bool {
        self.started
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sym {
    NotReceived,
    Small,
    Large,
}

impl Sym {
    pub fn letter(self) -> char {
        match self {
            Sym::NotReceived => 'N',
            Sym::Small => 'S',
            Sym::Large => 'L',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    pub base_seq: u16,
    pub status_count: u16,
    pub reference_time: u32,
    pub fb_count: u8,
    pub symbols: Vec<Sym>,
    pub deltas_250us: Vec<i32>,
}

const REF_UNIT_US: u64 = 64_000;
const BACK_WINDOW_US: u64 = 500_000;
const HISTORY_MAX: usize = 1 << 15;
const MAX_PER_FEEDBACK: usize = 2001;

#[derive(Debug, Clone, Default)]
pub struct TwccRecv {
    last16: Option<u16>,
    ext: i64,
    arrivals: BTreeMap<i64, u64>,
    next_start: Option<i64>,
    late_min: Option<i64>,
    fb_count: u8,
    latest_us: u64,
}

impl TwccRecv {
    fn unwrap(&mut self, seq: u16) -> i64 {
        match self.last16 {
            None => {
                self.ext = i64::from(seq);
            }
            Some(prev) => {
                self.ext += i64::from(seq.wrapping_sub(prev) as i16);
            }
        }
        self.last16 = Some(seq);
        self.ext
    }

    pub fn record(&mut self, seq: u16, arrival_us: u64) {
        let s = self.unwrap(seq);
        if self.arrivals.contains_key(&s) {
            return;
        }
        if self.next_start.is_some_and(|n| s < n) {
            self.late_min = Some(self.late_min.map_or(s, |m| m.min(s)));
        }
        self.arrivals.insert(s, arrival_us);
        self.latest_us = self.latest_us.max(arrival_us);
        while self.arrivals.len() > HISTORY_MAX {
            let Some((&k, _)) = self.arrivals.iter().next() else { break };
            self.arrivals.remove(&k);
        }
    }

    pub fn pending(&self) -> bool {
        let Some(&end) = self.arrivals.keys().next_back() else { return false };
        match self.next_start {
            None => true,
            Some(n) => end >= n || self.late_min.is_some(),
        }
    }

    pub fn build(&mut self) -> Vec<Feedback> {
        let mut out = Vec::new();
        let Some(&end) = self.arrivals.keys().next_back() else { return out };
        let first = *self.arrivals.keys().next().unwrap_or(&end);
        let mut start = match self.next_start {
            None => first,
            Some(n) => match self.late_min {
                Some(l) => l.min(n),
                None => n,
            },
        };
        start = start.max(first);
        if start > end {
            return out;
        }
        while start <= end {
            let Some((&first_rx, &first_at)) = self.arrivals.range(start..=end).next() else { break };
            let _ = first_rx;
            let ref_units = (first_at / REF_UNIT_US) % (1 << 24);
            let mut running = (first_at / REF_UNIT_US) as i64 * REF_UNIT_US as i64;
            let mut fb = Feedback {
                base_seq: (start & 0xffff) as u16,
                status_count: 0,
                reference_time: ref_units as u32,
                fb_count: self.fb_count,
                symbols: Vec::new(),
                deltas_250us: Vec::new(),
            };
            let mut s = start;
            let mut cut = false;
            while s <= end && fb.symbols.len() < MAX_PER_FEEDBACK {
                match self.arrivals.get(&s) {
                    None => fb.symbols.push(Sym::NotReceived),
                    Some(&at) => {
                        let d = at as i64 - running;
                        let units = (d as f64 / 250.0).round() as i64;
                        if units < i64::from(i16::MIN) || units > i64::from(i16::MAX) {
                            cut = true;
                            break;
                        }
                        fb.symbols.push(if (0..=255).contains(&units) { Sym::Small } else { Sym::Large });
                        fb.deltas_250us.push(units as i32);
                        running += units * 250;
                    }
                }
                s += 1;
            }
            while fb.symbols.last() == Some(&Sym::NotReceived) {
                fb.symbols.pop();
                s -= 1;
            }
            fb.status_count = fb.symbols.len() as u16;
            out.push(fb);
            self.fb_count = self.fb_count.wrapping_add(1);
            start = s;
            if !cut && s > end {
                break;
            }
        }
        self.next_start = Some(end + 1);
        self.late_min = None;
        let horizon = self.latest_us.saturating_sub(BACK_WINDOW_US);
        let keep_from = end + 1;
        self.arrivals.retain(|k, at| *k >= keep_from || *at >= horizon);
        out
    }
}

#[derive(Debug, Clone, Copy)]
struct Miss {
    first_ms: u64,
    sent_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NackUp {
    last: Option<u16>,
    missing: BTreeMap<u16, Miss>,
}

pub const NACK_RETRY_MS: u64 = 200;
pub const NACK_GIVEUP_MS: u64 = 1000;
pub const NACK_TRACK_MAX: usize = 128;

pub enum Gap {
    None,
    Tracked(usize),
    TooLarge,
}

impl NackUp {
    pub fn on_packet(&mut self, seq: u16, now_ms: u64) -> Gap {
        let Some(last) = self.last else {
            self.last = Some(seq);
            return Gap::None;
        };
        let diff = seq.wrapping_sub(last) as i16;
        if diff <= 0 {
            self.missing.remove(&seq);
            return Gap::None;
        }
        self.last = Some(seq);
        if diff == 1 {
            return Gap::None;
        }
        if diff >= 128 {
            self.missing.clear();
            return Gap::TooLarge;
        }
        let mut n = 0;
        let mut s = last.wrapping_add(1);
        while s != seq {
            if self.missing.len() >= NACK_TRACK_MAX {
                break;
            }
            self.missing.insert(s, Miss { first_ms: now_ms, sent_ms: None });
            n += 1;
            s = s.wrapping_add(1);
        }
        Gap::Tracked(n)
    }

    pub fn due(&mut self, now_ms: u64) -> Vec<u16> {
        self.missing.retain(|_, m| now_ms.saturating_sub(m.first_ms) < NACK_GIVEUP_MS);
        let mut out = Vec::new();
        for (s, m) in self.missing.iter_mut() {
            if m.sent_ms.is_none_or(|t| now_ms.saturating_sub(t) >= NACK_RETRY_MS) {
                m.sent_ms = Some(now_ms);
                out.push(*s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_gap_rules() {
        let mut n = NackUp::default();
        n.on_packet(10, 0);
        assert!(matches!(n.on_packet(13, 1), Gap::Tracked(2)));
        assert_eq!(n.due(5), vec![11, 12]);
        assert!(n.due(100).is_empty());
        n.on_packet(11, 150);
        assert_eq!(n.due(300), vec![12]);
        assert!(matches!(n.on_packet(400, 310), Gap::TooLarge), "S-sym-RULE-RTCP-NACK-UP-1 128 이상 틈은 NACK 하지 않는다");
        assert!(n.due(1500).is_empty());
    }
}
