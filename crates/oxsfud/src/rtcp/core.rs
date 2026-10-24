// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-RTCP-NACK-DOWN · 정:CAND-RTCP-TWCC-HISTORY · 정:CAND-RTCP-NACK-BUDGET-UNIT · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! RTCP 산수(순수) — 수신 통계(RFC 3550 부록 A) · TWCC 피드백 창(draft-holmer §3.1 · libwebrtc 수신 쪽) · 상향 NACK 결손 추적 · 하향 NACK 예산 · PLI 스로틀.

use std::collections::{BTreeMap, VecDeque};

pub const RTP_SEQ_MOD: u32 = 1 << 16;
pub const MIN_SEQUENTIAL: u32 = 2;
pub const MAX_DROPOUT: u32 = 3000;
pub const MAX_MISORDER: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub fraction: u8,
    pub lost: i32,
    pub extended_max: u32,
    pub jitter: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RecvStats {
    started: bool,
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    probation: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    transit: Option<i64>,
    jitter: f64,
    pub last_sr_ntp_mid: u32,
    pub last_sr_at_ms: u64,
}

impl RecvStats {
    fn init_seq(&mut self, seq: u16) {
        self.base_seq = seq as u32;
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD + 1;
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
        } else if udelta < MAX_DROPOUT {
            if seq < self.max_seq {
                self.cycles = self.cycles.wrapping_add(RTP_SEQ_MOD);
            }
            self.max_seq = seq;
        } else if udelta <= RTP_SEQ_MOD - MAX_MISORDER {
            if seq as u32 == self.bad_seq {
                self.init_seq(seq);
            } else {
                self.bad_seq = (seq as u32 + 1) & (RTP_SEQ_MOD - 1);
                return false;
            }
        }
        self.received += 1;
        true
    }

    pub fn on_packet(&mut self, seq: u16, ts: u32, arrival_ms: u64, clock: u32) -> bool {
        let valid = self.update_seq(seq);
        let arrival = ((arrival_ms as u128 * clock as u128) / 1000) as i64;
        let transit = arrival - ts as i64;
        if let Some(prev) = self.transit {
            let d = (transit - prev).abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = Some(transit);
        valid
    }

    pub fn report(&mut self) -> Report {
        let extended_max = self.cycles.wrapping_add(self.max_seq as u32);
        let expected = extended_max.wrapping_sub(self.base_seq).wrapping_add(1);
        let lost = (expected as i64 - self.received as i64).clamp(-0x80_0000, 0x7f_ffff) as i32;
        let expected_interval = expected.wrapping_sub(self.expected_prior);
        self.expected_prior = expected;
        let received_interval = self.received.wrapping_sub(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = expected_interval as i64 - received_interval as i64;
        let fraction = if expected_interval == 0 || lost_interval <= 0 { 0 } else { ((lost_interval << 8) / expected_interval as i64).min(255) as u8 };
        Report { fraction, lost, extended_max, jitter: self.jitter as u32 }
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
    pub deltas: Vec<i32>,
}

pub const TWCC_HISTORY_US: i64 = 500_000;
pub const TWCC_MAX_PACKETS: usize = 2001;
const REF_UNIT_US: i64 = 64_000;
const REF_WRAP_US: i64 = REF_UNIT_US << 24;
const DELTA_UNIT_US: i64 = 250;

#[derive(Debug, Clone, Default)]
pub struct TwccRecv {
    arrivals: BTreeMap<i64, i64>,
    last_unwrapped: Option<i64>,
    start: Option<i64>,
    fb_count: u8,
}

fn round_delta(us: i64) -> i64 {
    let half = DELTA_UNIT_US / 2;
    if us < 0 { (us - half) / DELTA_UNIT_US } else { (us + half) / DELTA_UNIT_US }
}

impl TwccRecv {
    fn unwrap(&mut self, seq: u16) -> i64 {
        let v = match self.last_unwrapped {
            None => seq as i64,
            Some(last) => {
                let diff = seq.wrapping_sub(last as u16) as i16 as i64;
                last + diff
            }
        };
        if self.last_unwrapped.map(|l| v > l).unwrap_or(true) {
            self.last_unwrapped = Some(v);
        }
        v
    }

    pub fn on_packet(&mut self, seq: u16, arrival_us: i64) {
        let s = self.unwrap(seq);
        if let Some(st) = self.start
            && s < st
        {
            let newest = self.arrivals.iter().next_back().map(|(_, t)| *t).unwrap_or(arrival_us);
            if newest - arrival_us <= TWCC_HISTORY_US || self.arrivals.is_empty() {
                self.start = Some(s);
            }
        }
        self.arrivals.entry(s).or_insert(arrival_us);
        let newest = self.arrivals.values().copied().max().unwrap_or(arrival_us);
        while let Some((&k, &t)) = self.arrivals.iter().next() {
            if newest - t > TWCC_HISTORY_US && Some(k) < self.start {
                self.arrivals.remove(&k);
            } else {
                break;
            }
        }
    }

    pub fn pending(&self) -> bool {
        let Some(&max) = self.arrivals.keys().next_back() else { return false };
        match self.start {
            None => true,
            Some(st) => max >= st,
        }
    }

    pub fn build(&mut self) -> Vec<Feedback> {
        let mut out = Vec::new();
        let Some(&max) = self.arrivals.keys().next_back() else { return out };
        let mut from = match self.start {
            Some(st) => st,
            None => *self.arrivals.keys().next().unwrap_or(&max),
        };
        if from > max {
            return out;
        }
        while from <= max {
            let Some((&first_rx, &first_t)) = self.arrivals.range(from..=max).next() else { break };
            let _ = first_rx;
            let ref_time = (first_t.rem_euclid(REF_WRAP_US)) / REF_UNIT_US;
            let mut base_t = first_t - (first_t.rem_euclid(REF_WRAP_US) - ref_time * REF_UNIT_US);
            let mut fb = Feedback { base_seq: (from & 0xffff) as u16, status_count: 0, reference_time: ref_time as u32, fb_count: self.fb_count, symbols: Vec::new(), deltas: Vec::new() };
            let mut seq = from;
            let mut cut = None;
            while seq <= max {
                if fb.symbols.len() >= TWCC_MAX_PACKETS {
                    cut = Some(seq);
                    break;
                }
                match self.arrivals.get(&seq) {
                    None => fb.symbols.push(Sym::NotReceived),
                    Some(&t) => {
                        let d = round_delta(t - base_t);
                        if !(-32768..=32767).contains(&d) {
                            cut = Some(seq);
                            break;
                        }
                        base_t += d * DELTA_UNIT_US;
                        fb.symbols.push(if (0..=255).contains(&d) { Sym::Small } else { Sym::Large });
                        fb.deltas.push(d as i32);
                    }
                }
                seq += 1;
            }
            while fb.symbols.last() == Some(&Sym::NotReceived) {
                fb.symbols.pop();
            }
            fb.status_count = fb.symbols.len() as u16;
            self.fb_count = self.fb_count.wrapping_add(1);
            out.push(fb);
            match cut {
                Some(c) => from = c,
                None => break,
            }
        }
        self.start = Some(max + 1);
        out
    }
}

pub const NACK_GAP_MAX: u16 = 128;
pub const NACK_RETRY_MS: u64 = 200;
pub const NACK_GIVEUP_MS: u64 = 1000;
pub const NACK_TRACK_MAX: usize = 128;

#[derive(Debug, Clone, Copy)]
struct Missing {
    first_ms: u64,
    sent_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NackUp {
    last: Option<i64>,
    missing: BTreeMap<i64, Missing>,
}

impl NackUp {
    pub fn on_packet(&mut self, seq: u16, now_ms: u64) {
        let Some(last) = self.last else {
            self.last = Some(seq as i64);
            return;
        };
        let diff = seq.wrapping_sub(last as u16) as i16 as i64;
        let ext = last + diff;
        if diff <= 0 {
            self.missing.remove(&ext);
            return;
        }
        if diff > 1 && diff < NACK_GAP_MAX as i64 {
            for k in last + 1..ext {
                if self.missing.len() >= NACK_TRACK_MAX {
                    break;
                }
                self.missing.entry(k).or_insert(Missing { first_ms: now_ms, sent_ms: None });
            }
        }
        self.last = Some(ext);
    }

    pub fn due(&mut self, now_ms: u64) -> Vec<u16> {
        self.missing.retain(|_, m| now_ms.saturating_sub(m.first_ms) < NACK_GIVEUP_MS);
        let mut out = Vec::new();
        for (k, m) in self.missing.iter_mut() {
            let ready = match m.sent_ms {
                None => true,
                Some(t) => now_ms.saturating_sub(t) >= NACK_RETRY_MS,
            };
            if ready {
                m.sent_ms = Some(now_ms);
                out.push((k.rem_euclid(RTP_SEQ_MOD as i64)) as u16);
            }
        }
        out
    }

    pub fn tracked(&self) -> usize {
        self.missing.len()
    }
}

pub fn nack_pairs(seqs: &[u16]) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for &s in seqs {
        if let Some((pid, blp)) = out.last_mut() {
            let d = s.wrapping_sub(pid.to_owned());
            if (1..=16).contains(&d) {
                let bit = 1u16 << (d - 1);
                let merged = blp.to_owned() | bit;
                let _ = std::mem::replace(blp, merged);
                continue;
            }
        }
        out.push((s, 0));
    }
    out
}

pub fn nack_expand(pid: u16, blp: u16) -> Vec<u16> {
    let mut out = vec![pid];
    for i in 0..16 {
        if blp & (1 << i) != 0 {
            out.push(pid.wrapping_add(i + 1));
        }
    }
    out
}

pub const RTX_BUDGET: usize = 200;
pub const RTX_BUDGET_WINDOW_MS: u64 = 3000;

#[derive(Debug, Clone, Default)]
pub struct Budget {
    spent: VecDeque<u64>,
}

impl Budget {
    pub fn take(&mut self, now_ms: u64) -> bool {
        while let Some(&t) = self.spent.front() {
            if now_ms.saturating_sub(t) >= RTX_BUDGET_WINDOW_MS {
                self.spent.pop_front();
            } else {
                break;
            }
        }
        if self.spent.len() >= RTX_BUDGET {
            return false;
        }
        self.spent.push_back(now_ms);
        true
    }
}

pub const PLI_THROTTLE_H_MS: u64 = 300;
pub const PLI_THROTTLE_L_MS: u64 = 100;

#[derive(Debug, Clone, Default)]
pub struct PliGate {
    last_ms: Option<u64>,
}

impl PliGate {
    pub fn pass(&mut self, now_ms: u64, low: bool, infra: bool) -> bool {
        let gap = if low { PLI_THROTTLE_L_MS } else { PLI_THROTTLE_H_MS };
        let ok = infra || self.last_ms.map(|t| now_ms.saturating_sub(t) >= gap).unwrap_or(true);
        if ok {
            self.last_ms = Some(now_ms);
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_up_rules() {
        let mut n = NackUp::default();
        n.on_packet(10, 0);
        n.on_packet(13, 0);
        assert_eq!(n.due(0), vec![11, 12]);
        assert!(n.due(100).is_empty(), "재전송 간격 200ms");
        n.on_packet(11, 120);
        assert_eq!(n.due(200), vec![12]);
        assert!(n.due(1000).is_empty(), "1,000ms 경과 시 만료");
        n.on_packet(13 + 200, 1000);
        assert_eq!(n.tracked(), 0, "128 이상 점프는 NACK 하지 않는다");
        let mut w = NackUp::default();
        w.on_packet(65534, 0);
        w.on_packet(65535, 0);
        w.on_packet(2, 0);
        assert_eq!(w.due(0), vec![0, 1], "감김 너머 결손");
    }

    #[test]
    fn nack_pack_and_budget() {
        assert_eq!(nack_pairs(&[5, 6, 8, 30]), vec![(5, 0b101), (30, 0)]);
        assert_eq!(nack_expand(5, 0b101), vec![5, 6, 8]);
        let mut b = Budget::default();
        for _ in 0..RTX_BUDGET {
            assert!(b.take(0));
        }
        assert!(!b.take(10), "200회 / 3초 / 구독자");
        assert!(b.take(3000));
    }

    #[test]
    fn pli_throttle() {
        let mut g = PliGate::default();
        assert!(g.pass(0, false, false));
        assert!(!g.pass(100, false, false));
        assert!(g.pass(150, false, true), "인프라 PLI 는 bypass");
        assert!(g.pass(250, true, false));
    }
}
