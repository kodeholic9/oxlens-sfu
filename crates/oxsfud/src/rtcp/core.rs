// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-CONC-SANS-IO · 정:CAND-RTCP-TWCC-HISTORY · 정:CAND-RTCP-RR-PER-STREAM · model: claude-opus-5-5
//! RTCP 산수(순수) — RR 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 수신 피드백(draft-holmer-01 §3.1 · libwebrtc 창) · NACK 상향 결손 추적. 시각은 인자다.

use std::collections::BTreeMap;

pub const MIN_SEQUENTIAL: u32 = 2;
pub const MAX_DROPOUT: u32 = 3_000;
pub const MAX_MISORDER: u32 = 100;
const SEQ_MOD: u32 = 1 << 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RrReport {
    pub fraction: u8,
    pub lost: i32,
    pub extended_max: u32,
    pub jitter: u32,
}

#[derive(Debug, Clone)]
pub struct RecvStats {
    clock: u32,
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    probation: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    transit: Option<i32>,
    jitter: f64,
    started: bool,
}

impl RecvStats {
    pub fn new(clock: u32) -> RecvStats {
        RecvStats {
            clock,
            max_seq: 0,
            cycles: 0,
            base_seq: 0,
            bad_seq: SEQ_MOD + 1,
            probation: MIN_SEQUENTIAL,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            transit: None,
            jitter: 0.0,
            started: false,
        }
    }

    fn init_seq(&mut self, seq: u16) {
        self.base_seq = u32::from(seq);
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
                self.cycles += SEQ_MOD;
            }
            self.max_seq = seq;
        } else if udelta <= SEQ_MOD - MAX_MISORDER {
            if u32::from(seq) == self.bad_seq {
                self.init_seq(seq);
            } else {
                self.bad_seq = (u32::from(seq) + 1) & (SEQ_MOD - 1);
                return false;
            }
        }
        self.received += 1;
        true
    }

    pub fn update(&mut self, seq: u16, ts: u32, arrival_ms: u64) -> bool {
        let valid = self.update_seq(seq);
        let arrival = (u128::from(arrival_ms) * u128::from(self.clock) / 1_000) as u32;
        let transit = arrival.wrapping_sub(ts) as i32;
        if let Some(prev) = self.transit {
            let d = f64::from(transit.wrapping_sub(prev)).abs();
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = Some(transit);
        valid
    }

    pub fn extended_max(&self) -> u32 {
        self.cycles.wrapping_add(u32::from(self.max_seq))
    }

    pub fn jitter(&self) -> u32 {
        self.jitter as u32
    }

    pub fn received(&self) -> u32 {
        self.received
    }

    pub fn lost(&self) -> i64 {
        if !self.started() {
            return 0;
        }
        i64::from(self.extended_max()) - i64::from(self.base_seq) + 1 - i64::from(self.received)
    }

    pub fn jitter_secs(&self) -> f64 {
        self.jitter / f64::from(self.clock.max(1))
    }

    pub fn started(&self) -> bool {
        self.started && self.probation == 0
    }

    pub fn report(&mut self) -> RrReport {
        let ext = self.extended_max();
        let expected = i64::from(ext) - i64::from(self.base_seq) + 1;
        let lost = (expected - i64::from(self.received)).clamp(-0x80_0000, 0x7f_ffff) as i32;
        let expected_interval = expected - i64::from(self.expected_prior);
        self.expected_prior = expected.max(0) as u32;
        let received_interval = i64::from(self.received) - i64::from(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = expected_interval - received_interval;
        let fraction = if expected_interval == 0 || lost_interval <= 0 { 0 } else { ((lost_interval << 8) / expected_interval).min(255) as u8 };
        RrReport { fraction, lost, extended_max: ext, jitter: self.jitter() }
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

pub const TWCC_MAX_STATUS: usize = 2_001;
pub const TWCC_BACK_WINDOW_US: i64 = 500_000;
pub const TWCC_HISTORY_MAX: usize = 1 << 15;

#[derive(Debug, Clone, Default)]
pub struct TwccRecv {
    arrivals: BTreeMap<i64, i64>,
    last_unwrapped: Option<i64>,
    window_start: Option<i64>,
    fb_count: u8,
}

fn round_250(us: i64) -> i64 {
    (us + 125).div_euclid(250)
}

impl TwccRecv {
    pub fn new() -> TwccRecv {
        TwccRecv::default()
    }

    fn unwrap(&mut self, seq: u16) -> i64 {
        let s = match self.last_unwrapped {
            None => i64::from(seq),
            Some(last) => last + i64::from(seq.wrapping_sub(last as u16) as i16),
        };
        if self.last_unwrapped.is_none_or(|l| s > l) {
            self.last_unwrapped = Some(s);
        }
        s
    }

    pub fn record(&mut self, seq: u16, arrival_us: i64) {
        let s = self.unwrap(seq);
        if self.arrivals.contains_key(&s) {
            return;
        }
        let floor = arrival_us - TWCC_BACK_WINDOW_US;
        let stale: Vec<i64> = self.arrivals.iter().take_while(|(k, t)| **t < floor && self.window_start.is_some_and(|w| **k < w)).map(|(k, _)| *k).collect();
        for k in stale {
            self.arrivals.remove(&k);
        }
        while self.arrivals.len() >= TWCC_HISTORY_MAX {
            let Some(k) = self.arrivals.keys().next().copied() else { break };
            self.arrivals.remove(&k);
        }
        self.arrivals.insert(s, arrival_us);
        if let Some(w) = self.window_start
            && s < w
        {
            self.window_start = Some(s);
        }
    }

    pub fn pending(&self) -> bool {
        let start = self.window_start.unwrap_or(i64::MIN);
        self.arrivals.range(start..).next().is_some()
    }

    pub fn build(&mut self) -> Vec<Feedback> {
        let start = match self.window_start {
            Some(w) => w,
            None => match self.arrivals.keys().next() {
                Some(k) => *k,
                None => return Vec::new(),
            },
        };
        let Some(end) = self.arrivals.range(start..).next_back().map(|(k, _)| *k) else { return Vec::new() };
        let mut out = Vec::new();
        let mut seq = start;
        while seq <= end {
            let base = seq;
            let Some(first_at) = self.arrivals.range(base..=end).next().map(|(_, t)| *t) else { break };
            let reference = (first_at.div_euclid(64_000)) & 0x00ff_ffff;
            let mut prev_ticks: i64 = 0;
            let ref_us = reference * 64_000;
            let mut symbols = Vec::new();
            let mut deltas = Vec::new();
            let mut next = base;
            while next <= end && symbols.len() < TWCC_MAX_STATUS {
                match self.arrivals.get(&next) {
                    None => symbols.push(Sym::NotReceived),
                    Some(t) => {
                        let ticks = round_250(t - ref_us);
                        let d = ticks - prev_ticks;
                        if !(i64::from(i16::MIN)..=i64::from(i16::MAX)).contains(&d) {
                            break;
                        }
                        prev_ticks = ticks;
                        symbols.push(if (0..=255).contains(&d) { Sym::Small } else { Sym::Large });
                        deltas.push(d as i32);
                    }
                }
                next += 1;
            }
            while symbols.last() == Some(&Sym::NotReceived) {
                symbols.pop();
                next -= 1;
            }
            if symbols.is_empty() {
                break;
            }
            out.push(Feedback { base_seq: base as u16, status_count: symbols.len() as u16, reference_time: reference as u32, fb_count: self.fb_count, symbols, deltas_250us: deltas });
            self.fb_count = self.fb_count.wrapping_add(1);
            seq = next;
        }
        self.window_start = Some(end + 1);
        out
    }
}

pub fn encode_twcc(sender_ssrc: u32, media_ssrc: u32, fb: &Feedback) -> Vec<u8> {
    let mut body = Vec::with_capacity(32 + fb.symbols.len());
    body.extend_from_slice(&sender_ssrc.to_be_bytes());
    body.extend_from_slice(&media_ssrc.to_be_bytes());
    body.extend_from_slice(&fb.base_seq.to_be_bytes());
    body.extend_from_slice(&fb.status_count.to_be_bytes());
    let rt = fb.reference_time & 0x00ff_ffff;
    body.extend_from_slice(&[(rt >> 16) as u8, (rt >> 8) as u8, rt as u8, fb.fb_count]);
    let code = |s: Sym| match s {
        Sym::NotReceived => 0u16,
        Sym::Small => 1,
        Sym::Large => 2,
    };
    let mut i = 0;
    while i < fb.symbols.len() {
        let s = fb.symbols[i];
        let mut run = 1;
        while i + run < fb.symbols.len() && fb.symbols[i + run] == s && run < 0x1fff {
            run += 1;
        }
        let chunk: u16 = (code(s) << 13) | (run as u16);
        body.extend_from_slice(&chunk.to_be_bytes());
        i += run;
    }
    let mut di = 0;
    for s in &fb.symbols {
        match s {
            Sym::NotReceived => {}
            Sym::Small => {
                body.push(fb.deltas_250us[di] as u8);
                di += 1;
            }
            Sym::Large => {
                body.extend_from_slice(&(fb.deltas_250us[di] as i16).to_be_bytes());
                di += 1;
            }
        }
    }
    let mut pad: u8 = 0;
    while !body.len().is_multiple_of(4) {
        body.push(0);
        pad += 1;
    }
    if pad > 0 {
        let n = body.len();
        body[n - 1] = pad;
    }
    let words = (body.len() / 4) as u16;
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(0x80 | if pad > 0 { 0x20 } else { 0 } | 15);
    out.push(205);
    out.extend_from_slice(&words.to_be_bytes());
    out.extend_from_slice(&body);
    out
}

pub const NACK_MAX_DIFF: i32 = 128;
pub const NACK_RETRY_MS: u64 = 200;
pub const NACK_GIVE_UP_MS: u64 = 1_000;
pub const NACK_TRACK_MAX: usize = 128;

#[derive(Debug, Clone, Copy)]
struct Missing {
    first_ms: u64,
    sent_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NackUp {
    last_seq: Option<u16>,
    missing: BTreeMap<u16, Missing>,
}

impl NackUp {
    pub fn new() -> NackUp {
        NackUp::default()
    }

    pub fn on_packet(&mut self, seq: u16, now_ms: u64) {
        let Some(last) = self.last_seq else {
            self.last_seq = Some(seq);
            return;
        };
        let diff = i32::from(seq.wrapping_sub(last) as i16);
        if diff <= 0 {
            self.missing.remove(&seq);
            return;
        }
        if diff > 1 && diff < NACK_MAX_DIFF {
            for k in 1..diff {
                if self.missing.len() >= NACK_TRACK_MAX {
                    break;
                }
                self.missing.insert(last.wrapping_add(k as u16), Missing { first_ms: now_ms, sent_ms: None });
            }
        }
        self.missing.remove(&seq);
        self.last_seq = Some(seq);
    }

    pub fn due(&mut self, now_ms: u64) -> Vec<u16> {
        self.missing.retain(|_, m| now_ms.saturating_sub(m.first_ms) < NACK_GIVE_UP_MS);
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
    fn nack_up_rules() {
        let mut n = NackUp::new();
        n.on_packet(10, 0);
        n.on_packet(13, 0);
        assert_eq!(n.due(0), vec![11, 12]);
        assert!(n.due(100).is_empty());
        n.on_packet(11, 150);
        assert_eq!(n.due(200), vec![12]);
        assert!(n.due(1_000).is_empty());
        n.on_packet(500, 1_000);
        assert!(n.due(1_000).is_empty(), "128 이상 점프는 NACK 하지 않는다");
    }

    #[test]
    fn twcc_encodes_header() {
        let fb = Feedback { base_seq: 10, status_count: 3, reference_time: 15, fb_count: 0, symbols: vec![Sym::Small; 3], deltas_250us: vec![160, 80, 80] };
        let b = encode_twcc(1, 2, &fb);
        assert_eq!(b[1], 205);
        assert_eq!(b[0] & 0x1f, 15);
        assert_eq!(b.len() % 4, 0);
    }
}
