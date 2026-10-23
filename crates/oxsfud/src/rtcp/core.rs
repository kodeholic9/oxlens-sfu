// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! RTCP 산수(sans-IO) — 수신 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 피드백 창(draft-holmer §3.1 · libwebrtc 수신 쪽) · NACK 결손 추적.

use std::collections::BTreeMap;

pub const MIN_SEQUENTIAL: u32 = 2;
pub const MAX_DROPOUT: u32 = 3000;
pub const MAX_MISORDER: u32 = 100;
pub const SEQ_MOD: u32 = 1 << 16;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecvStats {
    pub max_seq: u16,
    pub cycles: u32,
    pub base_seq: u32,
    pub bad_seq: u32,
    pub probation: u32,
    pub received: u32,
    pub expected_prior: u32,
    pub received_prior: u32,
    pub transit: Option<i64>,
    pub jitter: f64,
    pub started: bool,
    pub last_sr_mid: u32,
    pub last_sr_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub fraction: u8,
    pub lost: i32,
    pub extended_max: u32,
    pub jitter: u32,
}

impl RecvStats {
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
                self.cycles = self.cycles.wrapping_add(SEQ_MOD);
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

    pub fn update(&mut self, seq: u16, ts: u32, arrival_ms: u64, clock: u32) -> bool {
        if !self.started {
            self.started = true;
            self.init_seq(seq);
            self.max_seq = seq.wrapping_sub(1);
            self.probation = MIN_SEQUENTIAL;
        }
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

    pub fn extended_max(&self) -> u32 {
        self.cycles.wrapping_add(self.max_seq as u32)
    }

    pub fn report(&mut self) -> Report {
        let extended_max = self.extended_max();
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feedback {
    pub base_seq: u16,
    pub status_count: u16,
    pub reference_time: u32,
    pub fb_count: u8,
    pub symbols: Vec<Sym>,
    pub deltas: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sym {
    N,
    S,
    L,
}

pub const TWCC_HISTORY_US: u64 = 500_000;
pub const TWCC_MAX_PACKETS: usize = 2001;
const REF_UNIT_US: i64 = 64_000;
const REF_WRAP_US: i64 = 64_000 << 24;

#[derive(Clone, Debug, Default)]
pub struct TwccRecv {
    arrivals: BTreeMap<i64, u64>,
    last_unwrapped: Option<i64>,
    window_start: Option<i64>,
    fb_count: u8,
}

impl TwccRecv {
    fn unwrap(&mut self, seq: u16) -> i64 {
        let u = match self.last_unwrapped {
            None => seq as i64,
            Some(prev) => {
                let d = seq.wrapping_sub(prev as u16) as i16 as i64;
                prev + d
            }
        };
        if self.last_unwrapped.map(|p| u > p).unwrap_or(true) {
            self.last_unwrapped = Some(u);
        }
        u
    }

    pub fn record(&mut self, seq: u16, arrival_us: u64) {
        let u = self.unwrap(seq);
        self.arrivals.entry(u).or_insert(arrival_us);
        match self.window_start {
            None => self.window_start = Some(u),
            Some(ws) if u < ws => self.window_start = Some(u),
            _ => {}
        }
        if let Some((_, last)) = self.arrivals.iter().next_back() {
            let floor = last.saturating_sub(TWCC_HISTORY_US);
            let keep_from = self.window_start.unwrap_or(i64::MIN);
            self.arrivals.retain(|s, a| *s >= keep_from || *a >= floor);
        }
    }

    pub fn pending(&self) -> bool {
        match (self.window_start, self.arrivals.keys().next_back()) {
            (Some(ws), Some(end)) => *end >= ws,
            _ => false,
        }
    }

    pub fn build(&mut self) -> Vec<Feedback> {
        let mut out = Vec::new();
        let (Some(mut start), Some(&end)) = (self.window_start, self.arrivals.keys().next_back()) else { return out };
        while start <= end {
            let Some((&first_seq, &first_at)) = self.arrivals.range(start..=end).next() else { break };
            let _ = first_seq;
            let ref_time = ((first_at as i64 % REF_WRAP_US) / REF_UNIT_US) as u32;
            let mut base_us = (ref_time as i64) * REF_UNIT_US + (first_at as i64 - (first_at as i64 % REF_WRAP_US));
            let mut fb = Feedback { base_seq: start as u16, status_count: 0, reference_time: ref_time, fb_count: self.fb_count, symbols: Vec::new(), deltas: Vec::new() };
            let mut seq = start;
            let mut cut = false;
            while seq <= end && fb.symbols.len() < TWCC_MAX_PACKETS {
                match self.arrivals.get(&seq) {
                    None => fb.symbols.push(Sym::N),
                    Some(&at) => {
                        let d_us = at as i64 - base_us;
                        let d = ((d_us as f64) / 250.0).round() as i64;
                        if !(-32768..=32767).contains(&d) {
                            cut = true;
                            break;
                        }
                        fb.symbols.push(if (0..=255).contains(&d) { Sym::S } else { Sym::L });
                        fb.deltas.push(d as i32);
                        base_us += d * 250;
                    }
                }
                seq += 1;
            }
            while fb.symbols.last() == Some(&Sym::N) {
                fb.symbols.pop();
                seq -= 1;
            }
            fb.status_count = fb.symbols.len() as u16;
            if fb.status_count > 0 {
                out.push(fb);
                self.fb_count = self.fb_count.wrapping_add(1);
            }
            start = seq;
            if !cut && seq > end {
                break;
            }
        }
        self.window_start = Some(end + 1);
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct NackUp {
    pub last_seq: Option<u16>,
    pub missing: BTreeMap<u16, (u64, Option<u64>)>,
}

pub const NACK_MAX_GAP: u16 = 128;
pub const NACK_TRACK_MAX: usize = 128;
pub const NACK_INTERVAL_MS: u64 = 200;
pub const NACK_GIVE_UP_MS: u64 = 1000;

impl NackUp {
    pub fn on_packet(&mut self, seq: u16, now_ms: u64) {
        let Some(last) = self.last_seq else {
            self.last_seq = Some(seq);
            return;
        };
        let diff = seq.wrapping_sub(last) as i16;
        if diff <= 0 {
            self.missing.remove(&seq);
            return;
        }
        if diff > 1 && (diff as u16) < NACK_MAX_GAP {
            let mut s = last.wrapping_add(1);
            while s != seq && self.missing.len() < NACK_TRACK_MAX {
                self.missing.insert(s, (now_ms, None));
                s = s.wrapping_add(1);
            }
        }
        self.last_seq = Some(seq);
    }

    pub fn due(&mut self, now_ms: u64) -> Vec<u16> {
        self.missing.retain(|_, (first, _)| now_ms.saturating_sub(*first) < NACK_GIVE_UP_MS);
        let mut out = Vec::new();
        for (s, (_, sent)) in self.missing.iter_mut() {
            if sent.map(|t| now_ms.saturating_sub(t) >= NACK_INTERVAL_MS).unwrap_or(true) {
                sent.replace(now_ms);
                out.push(*s);
            }
        }
        out
    }
}

pub fn nack_pairs(seqs: &[u16]) -> Vec<(u16, u16)> {
    let mut v: Vec<u16> = seqs.to_vec();
    v.sort_by(|a, b| if crate::fwd::core::seq_after(*a, *b) { std::cmp::Ordering::Greater } else if a == b { std::cmp::Ordering::Equal } else { std::cmp::Ordering::Less });
    let mut out: Vec<(u16, u16)> = Vec::new();
    for s in v {
        if let Some((pid, blp)) = out.last_mut() {
            let d = s.wrapping_sub(*pid);
            if (1..=16).contains(&d) {
                blp.clone_from(&(*blp | (1 << (d - 1))));
                continue;
            }
        }
        out.push((s, 0));
    }
    out
}

pub const SENDER_SSRC: u32 = 1;

fn head(out: &mut Vec<u8>, count: u8, pt: u8) {
    out.push(0x80 | (count & 0x1f));
    out.push(pt);
    out.extend_from_slice(&[0, 0]);
}

fn finish(out: &mut [u8], start: usize) {
    let words = ((out.len() - start) / 4 - 1) as u16;
    out[start + 2..start + 4].copy_from_slice(&words.to_be_bytes());
}

pub fn encode_twcc(fb: &Feedback, media_ssrc: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    head(&mut out, 15, 205);
    out.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    out.extend_from_slice(&media_ssrc.to_be_bytes());
    out.extend_from_slice(&fb.base_seq.to_be_bytes());
    out.extend_from_slice(&fb.status_count.to_be_bytes());
    let rt = fb.reference_time & 0x00ff_ffff;
    out.extend_from_slice(&[(rt >> 16) as u8, (rt >> 8) as u8, rt as u8, fb.fb_count]);
    let code = |s: Sym| match s {
        Sym::N => 0u16,
        Sym::S => 1,
        Sym::L => 2,
    };
    let mut i = 0;
    while i < fb.symbols.len() {
        let s = fb.symbols[i];
        let mut n = 1;
        while i + n < fb.symbols.len() && fb.symbols[i + n] == s && n < 0x1fff {
            n += 1;
        }
        let chunk = (code(s) << 13) | n as u16;
        out.extend_from_slice(&chunk.to_be_bytes());
        i += n;
    }
    let mut di = 0;
    for s in &fb.symbols {
        match s {
            Sym::N => {}
            Sym::S => {
                out.push(fb.deltas[di] as u8);
                di += 1;
            }
            Sym::L => {
                out.extend_from_slice(&(fb.deltas[di] as i16).to_be_bytes());
                di += 1;
            }
        }
    }
    let pad = (4 - out.len() % 4) % 4;
    if pad > 0 {
        out.extend(std::iter::repeat_n(0u8, pad - 1));
        out.push(pad as u8);
        out[0] |= 0x20;
    }
    finish(&mut out, 0);
    out
}

pub fn parse_twcc(fci: &[u8]) -> Option<Vec<(u16, i64)>> {
    if fci.len() < 8 {
        return None;
    }
    let base = u16::from_be_bytes([fci[0], fci[1]]);
    let count = u16::from_be_bytes([fci[2], fci[3]]) as usize;
    let rt = ((fci[4] as i64) << 16) | ((fci[5] as i64) << 8) | fci[6] as i64;
    let mut syms: Vec<u8> = Vec::with_capacity(count);
    let mut i = 8;
    while syms.len() < count {
        let c = u16::from_be_bytes([*fci.get(i)?, *fci.get(i + 1)?]);
        i += 2;
        if c & 0x8000 == 0 {
            let s = ((c >> 13) & 0x3) as u8;
            let n = (c & 0x1fff) as usize;
            for _ in 0..n.min(count - syms.len()) {
                syms.push(s);
            }
        } else if c & 0x4000 == 0 {
            for b in (0..14).rev() {
                if syms.len() < count {
                    syms.push(((c >> b) & 1) as u8);
                }
            }
        } else {
            for b in (0..7).rev() {
                if syms.len() < count {
                    syms.push(((c >> (b * 2)) & 3) as u8);
                }
            }
        }
    }
    let mut at = rt * 64_000;
    let mut out = Vec::new();
    for (k, s) in syms.iter().enumerate() {
        let seq = base.wrapping_add(k as u16);
        match s {
            1 => {
                at += *fci.get(i)? as i64 * 250;
                i += 1;
                out.push((seq, at));
            }
            2 => {
                at += i16::from_be_bytes([*fci.get(i)?, *fci.get(i + 1)?]) as i64 * 250;
                i += 2;
                out.push((seq, at));
            }
            _ => {}
        }
    }
    Some(out)
}

pub fn encode_rr(ssrc: u32, r: &Report, lsr: u32, dlsr: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    head(&mut out, 1, 201);
    out.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    let lost = (r.lost as u32) & 0x00ff_ffff;
    out.push(r.fraction);
    out.extend_from_slice(&[(lost >> 16) as u8, (lost >> 8) as u8, lost as u8]);
    out.extend_from_slice(&r.extended_max.to_be_bytes());
    out.extend_from_slice(&r.jitter.to_be_bytes());
    out.extend_from_slice(&lsr.to_be_bytes());
    out.extend_from_slice(&dlsr.to_be_bytes());
    finish(&mut out, 0);
    out
}

pub fn encode_nack(media: u32, pairs: &[(u16, u16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + pairs.len() * 4);
    head(&mut out, 1, 205);
    out.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    for (p, b) in pairs {
        out.extend_from_slice(&p.to_be_bytes());
        out.extend_from_slice(&b.to_be_bytes());
    }
    finish(&mut out, 0);
    out
}

pub fn encode_pli(media: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    head(&mut out, 1, 206);
    out.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    out.extend_from_slice(&media.to_be_bytes());
    finish(&mut out, 0);
    out
}

pub fn encode_remb(bps: u64, ssrcs: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + ssrcs.len() * 4);
    head(&mut out, 15, 206);
    out.extend_from_slice(&SENDER_SSRC.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"REMB");
    let mut exp = 0u32;
    let mut m = bps;
    while m > 0x3ffff {
        m >>= 1;
        exp += 1;
    }
    out.push(ssrcs.len() as u8);
    let v = (exp << 18) | m as u32;
    out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    for s in ssrcs {
        out.extend_from_slice(&s.to_be_bytes());
    }
    finish(&mut out, 0);
    out
}

pub fn encode_sr(ssrc: u32, ntp: u64, rtp_ts: u32, packets: u32, octets: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(28);
    head(&mut out, 0, 200);
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&ntp.to_be_bytes());
    out.extend_from_slice(&rtp_ts.to_be_bytes());
    out.extend_from_slice(&packets.to_be_bytes());
    out.extend_from_slice(&octets.to_be_bytes());
    finish(&mut out, 0);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pkt<'a> {
    pub count: u8,
    pub pt: u8,
    pub body: &'a [u8],
}

pub fn split(b: &[u8]) -> Vec<Pkt<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        if b[i] >> 6 != 2 {
            break;
        }
        let words = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let end = i + 4 + words * 4;
        if end > b.len() {
            break;
        }
        let mut body = &b[i + 4..end];
        if b[i] & 0x20 != 0
            && let Some(&n) = body.last() {
                body = &body[..body.len().saturating_sub(n as usize)];
            }
        out.push(Pkt { count: b[i] & 0x1f, pt: b[i + 1], body });
        i = end;
    }
    out
}

pub fn remb_bps(fci: &[u8]) -> Option<u64> {
    if fci.len() < 8 || &fci[0..4] != b"REMB" {
        return None;
    }
    let exp = (fci[5] >> 2) as u32;
    let m = (((fci[5] & 0x03) as u64) << 16) | ((fci[6] as u64) << 8) | fci[7] as u64;
    Some(m << exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twcc_encode_parse_round_trip() {
        let fb = Feedback { base_seq: 10, status_count: 4, reference_time: 5, fb_count: 3, symbols: vec![Sym::S, Sym::N, Sym::L, Sym::S], deltas: vec![4, 300, 2] };
        let b = encode_twcc(&fb, 9);
        assert_eq!(b.len() % 4, 0);
        let p = split(&b);
        assert_eq!(p.len(), 1);
        let fci = &p[0].body[8..];
        let got = parse_twcc(fci).unwrap_or_default();
        assert_eq!(got.iter().map(|x| x.0).collect::<Vec<_>>(), vec![10, 12, 13]);
        assert_eq!(got[0].1, 5 * 64_000 + 1000);
    }

    #[test]
    fn remb_mantissa_round_trip() {
        let b = encode_remb(800_000, &[7]);
        let p = split(&b);
        assert_eq!(remb_bps(&p[0].body[8..]), Some(800_000));
    }

    #[test]
    fn nack_gap_under_128_only() {
        let mut n = NackUp::default();
        n.on_packet(10, 0);
        n.on_packet(13, 0);
        assert_eq!(n.due(0), vec![11, 12]);
        assert!(n.due(100).is_empty());
        assert_eq!(n.due(250), vec![11, 12]);
        n.on_packet(12, 260);
        assert_eq!(n.due(500), vec![11]);
        let mut big = NackUp::default();
        big.on_packet(0, 0);
        big.on_packet(200, 0);
        assert!(big.due(0).is_empty());
        assert!(n.due(2_000).is_empty());
    }

    #[test]
    fn nack_pairs_pack_blp() {
        assert_eq!(nack_pairs(&[11, 12, 14]), vec![(11, 0b101)]);
        assert_eq!(nack_pairs(&[65535, 0]), vec![(65535, 1)]);
    }
}
