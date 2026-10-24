// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-CONC-SANS-IO · 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! RTCP 산수(sans-IO) — 수신 통계(RFC 3550 A.1 · A.3 · A.8) · TWCC 피드백 창(draft-holmer §3.1 · libwebrtc 수신 쪽) · NACK 결손 추적 · 패킷 꼴은 고정 부품 `rtcp` 로 짓고 읽는다.

use std::collections::BTreeMap;

use bytes::Bytes;
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::receiver_report::ReceiverReport;
use rtcp::reception_report::ReceptionReport;
use rtcp::sender_report::SenderReport;
use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, RecvDelta, RunLengthChunk, StatusChunkTypeTcc, SymbolTypeTcc, TransportLayerCc};
use rtcp::transport_feedbacks::transport_layer_nack::{NackPair, TransportLayerNack};
use webrtc_util::marshal::{Marshal, Unmarshal};

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

fn bytes<P: Marshal>(p: &P) -> Vec<u8> {
    p.marshal().map(|b| b.to_vec()).unwrap_or_default()
}

fn sym(s: Sym) -> SymbolTypeTcc {
    match s {
        Sym::N => SymbolTypeTcc::PacketNotReceived,
        Sym::S => SymbolTypeTcc::PacketReceivedSmallDelta,
        Sym::L => SymbolTypeTcc::PacketReceivedLargeDelta,
    }
}

pub fn encode_twcc(fb: &Feedback, media_ssrc: u32) -> Vec<u8> {
    let mut packet_chunks = Vec::new();
    let mut i = 0;
    while i < fb.symbols.len() {
        let s = fb.symbols[i];
        let mut n = 1;
        while i + n < fb.symbols.len() && fb.symbols[i + n] == s && n < 0x1fff {
            n += 1;
        }
        packet_chunks.push(PacketStatusChunk::RunLengthChunk(RunLengthChunk { type_tcc: StatusChunkTypeTcc::RunLengthChunk, packet_status_symbol: sym(s), run_length: n as u16 }));
        i += n;
    }
    let recv_deltas = fb.symbols.iter().filter(|s| **s != Sym::N).zip(&fb.deltas).map(|(s, d)| RecvDelta { type_tcc_packet: sym(*s), delta: *d as i64 * 250 }).collect();
    bytes(&TransportLayerCc {
        sender_ssrc: SENDER_SSRC,
        media_ssrc,
        base_sequence_number: fb.base_seq,
        packet_status_count: fb.status_count,
        reference_time: fb.reference_time & 0x00ff_ffff,
        fb_pkt_count: fb.fb_count,
        packet_chunks,
        recv_deltas,
    })
}

pub fn parse_twcc(raw: &[u8]) -> Option<Vec<(u16, i64)>> {
    let p = TransportLayerCc::unmarshal(&mut &raw[..]).ok()?;
    let mut syms: Vec<SymbolTypeTcc> = Vec::with_capacity(p.packet_status_count as usize);
    for c in &p.packet_chunks {
        match c {
            PacketStatusChunk::RunLengthChunk(r) => syms.extend(std::iter::repeat_n(r.packet_status_symbol, r.run_length as usize)),
            PacketStatusChunk::StatusVectorChunk(v) => syms.extend(v.symbol_list.iter().copied()),
        }
    }
    syms.truncate(p.packet_status_count as usize);
    let mut at = p.reference_time as i64 * 64_000;
    let mut deltas = p.recv_deltas.iter();
    let mut out = Vec::new();
    for (k, s) in syms.iter().enumerate() {
        if matches!(s, SymbolTypeTcc::PacketReceivedSmallDelta | SymbolTypeTcc::PacketReceivedLargeDelta) {
            at += deltas.next()?.delta;
            out.push((p.base_sequence_number.wrapping_add(k as u16), at));
        }
    }
    Some(out)
}

pub fn encode_rr(ssrc: u32, r: &Report, lsr: u32, dlsr: u32) -> Vec<u8> {
    let block = ReceptionReport { ssrc, fraction_lost: r.fraction, total_lost: (r.lost as u32) & 0x00ff_ffff, last_sequence_number: r.extended_max, jitter: r.jitter, last_sender_report: lsr, delay: dlsr };
    bytes(&ReceiverReport { ssrc: SENDER_SSRC, reports: vec![block], profile_extensions: Bytes::new() })
}

pub fn encode_nack(media: u32, pairs: &[(u16, u16)]) -> Vec<u8> {
    let nacks = pairs.iter().map(|&(packet_id, lost_packets)| NackPair { packet_id, lost_packets }).collect();
    bytes(&TransportLayerNack { sender_ssrc: SENDER_SSRC, media_ssrc: media, nacks })
}

pub fn encode_pli(media: u32) -> Vec<u8> {
    bytes(&PictureLossIndication { sender_ssrc: SENDER_SSRC, media_ssrc: media })
}

pub fn encode_remb(bps: u64, ssrcs: &[u32]) -> Vec<u8> {
    bytes(&ReceiverEstimatedMaximumBitrate { sender_ssrc: SENDER_SSRC, bitrate: bps as f32, ssrcs: ssrcs.to_vec() })
}

pub fn encode_sr(ssrc: u32, ntp: u64, rtp_ts: u32, packets: u32, octets: u32) -> Vec<u8> {
    bytes(&SenderReport { ssrc, ntp_time: ntp, rtp_time: rtp_ts, packet_count: packets, octet_count: octets, reports: Vec::new(), profile_extensions: Bytes::new() })
}

pub fn sr(raw: &[u8]) -> Option<(u32, u64, u32)> {
    let p = SenderReport::unmarshal(&mut &raw[..]).ok()?;
    Some((p.ssrc, p.ntp_time, p.rtp_time))
}

pub fn rr_blocks(raw: &[u8]) -> Vec<(u32, u8)> {
    ReceiverReport::unmarshal(&mut &raw[..]).map(|p| p.reports.iter().map(|r| (r.ssrc, r.fraction_lost)).collect()).unwrap_or_default()
}

pub fn nack_lost(raw: &[u8]) -> Option<(u32, Vec<u16>)> {
    let p = TransportLayerNack::unmarshal(&mut &raw[..]).ok()?;
    Some((p.media_ssrc, p.nacks.iter().flat_map(|n| n.packet_list()).collect()))
}

pub fn key_media(raw: &[u8], fir: bool) -> Option<u32> {
    if fir {
        FullIntraRequest::unmarshal(&mut &raw[..]).ok()?.fir.first().map(|e| e.ssrc)
    } else {
        Some(PictureLossIndication::unmarshal(&mut &raw[..]).ok()?.media_ssrc)
    }
}

pub fn remb_bps(raw: &[u8]) -> Option<u64> {
    Some(ReceiverEstimatedMaximumBitrate::unmarshal(&mut &raw[..]).ok()?.bitrate as u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pkt<'a> {
    pub count: u8,
    pub pt: u8,
    pub raw: &'a [u8],
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
        out.push(Pkt { count: b[i] & 0x1f, pt: b[i + 1], raw: &b[i..end] });
        i = end;
    }
    out
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
        let got = parse_twcc(p[0].raw).unwrap_or_default();
        assert_eq!(got.iter().map(|x| x.0).collect::<Vec<_>>(), vec![10, 12, 13]);
        assert_eq!(got[0].1, 5 * 64_000 + 1000);
    }

    #[test]
    fn remb_mantissa_round_trip() {
        let b = encode_remb(800_000, &[7]);
        let p = split(&b);
        assert_eq!(remb_bps(p[0].raw), Some(800_000));
    }

    #[test]
    fn nack_and_key_round_trip() {
        let b = encode_nack(9, &nack_pairs(&[11, 12, 14]));
        assert_eq!(nack_lost(split(&b)[0].raw), Some((9, vec![11, 12, 14])));
        assert_eq!(key_media(split(&encode_pli(5))[0].raw, false), Some(5));
        let rr = encode_rr(4, &Report { fraction: 7, lost: 1, extended_max: 9, jitter: 2 }, 0, 0);
        assert_eq!(rr_blocks(split(&rr)[0].raw), vec![(4, 7)]);
        assert_eq!(sr(split(&encode_sr(3, 1 << 40, 8, 1, 2))[0].raw), Some((3, 1 << 40, 8)));
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
