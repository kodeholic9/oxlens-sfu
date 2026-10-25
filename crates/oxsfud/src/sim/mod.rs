// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-SIM-MODES 정:RULE-SIM-TICK 정:IMPL-SIM-SWITCH 정:CAND-SIM-PROBE-SHAPE 정:IMPL-RTCP-TERMINATE 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! 시뮬캐스트 자동 판단 — 구독 전송로마다(`off` · v1 REMB + RR 손실 · v2 TWCC GCC + RTX 패딩 프로브) · 판단은 `core` 순수 함수 · 여기는 신호 모으기와 집행.

pub mod core;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::policy::MediaAutolayer;
use oxsig::msg::Kind;
use rtcp::transport_feedbacks::transport_layer_cc::{SymbolTypeTcc, TransportLayerCc};

use self::core::{Aimd, Decision, LAYER_H_BPS, LAYER_L_BPS, Layer, Policy, Signals, Trendline, Usage};
use crate::fwd::Conn;
use crate::ops::Sfu;

const SENT_KEEP: usize = 4096;

struct Sent {
    seq: u16,
    t_us: u64,
    size: usize,
    probe: bool,
}

struct ProbeRun {
    started: u64,
    rate_bps: f64,
    acked_bytes: usize,
    first_arrival_us: Option<i64>,
    last_arrival_us: Option<i64>,
}

pub struct Bwe {
    pub policy: Policy,
    aimd: Aimd,
    trend: Trendline,
    remb: Option<(f64, u64)>,
    loss: HashMap<u8, (f64, u64)>,
    nacks: VecDeque<u64>,
    sent: VecDeque<Sent>,
    est: Option<(f64, u64)>,
    overuse: bool,
    last_fb_ms: u64,
    group: Option<(u64, f64)>,
    probe: Option<ProbeRun>,
}

impl Default for Bwe {
    fn default() -> Self {
        Bwe {
            policy: Policy::default(),
            aimd: Aimd::new(core::START_BPS),
            trend: Trendline::default(),
            remb: None,
            loss: HashMap::new(),
            nacks: VecDeque::new(),
            sent: VecDeque::new(),
            est: None,
            overuse: false,
            last_fb_ms: 0,
            group: None,
            probe: None,
        }
    }
}

impl Bwe {
    pub fn on_sent(&mut self, seq: u16, t_us: u64, size: usize, probe: bool) {
        self.sent.push_back(Sent { seq, t_us, size, probe });
        while self.sent.len() > SENT_KEEP {
            self.sent.pop_front();
        }
    }

    pub fn on_loss(&mut self, mid: u8, pct: f64, now: u64) {
        self.loss.insert(mid, (pct, now));
    }

    pub fn on_remb(&mut self, bps: f64, now: u64) {
        self.remb = Some((bps, now));
    }

    pub fn on_nack(&mut self, now: u64) {
        self.nacks.push_back(now);
    }

    pub fn on_twcc(&mut self, t: &TransportLayerCc, now: u64) {
        let mut arrival_us = i64::from(t.reference_time) * 64_000;
        let mut seq = t.base_sequence_number;
        let mut d = t.recv_deltas.iter();
        let mut acked_bytes = 0usize;
        let mut first_arr: Option<i64> = None;
        let mut last_arr: Option<i64> = None;
        let mut statuses: Vec<SymbolTypeTcc> = Vec::new();
        for c in &t.packet_chunks {
            match c {
                rtcp::transport_feedbacks::transport_layer_cc::PacketStatusChunk::RunLengthChunk(r) => {
                    statuses.extend(std::iter::repeat_n(r.packet_status_symbol, r.run_length as usize));
                }
                rtcp::transport_feedbacks::transport_layer_cc::PacketStatusChunk::StatusVectorChunk(v) => statuses.extend(v.symbol_list.iter().copied()),
            }
        }
        statuses.truncate(t.packet_status_count as usize);
        for s in statuses {
            let got = matches!(s, SymbolTypeTcc::PacketReceivedSmallDelta | SymbolTypeTcc::PacketReceivedLargeDelta);
            if got && let Some(rd) = d.next() {
                arrival_us += rd.delta;
                if let Some(p) = self.sent.iter().find(|x| x.seq == seq) {
                    acked_bytes += p.size;
                    first_arr.get_or_insert(arrival_us);
                    last_arr = Some(arrival_us);
                    let send_ms = p.t_us as f64 / 1000.0;
                    let arr_ms = arrival_us as f64 / 1000.0;
                    match self.group {
                        Some((g_send, g_arr)) if (p.t_us / 1000).saturating_sub(g_send) >= 5 => {
                            let usage = self.trend.update(arr_ms - g_arr, send_ms - g_send as f64, arr_ms);
                            self.overuse = usage == Usage::Overuse;
                            self.group = Some((p.t_us / 1000, arr_ms));
                        }
                        None => self.group = Some((p.t_us / 1000, arr_ms)),
                        _ => {}
                    }
                    if p.probe
                        && let Some(pr) = self.probe.as_mut()
                    {
                        pr.acked_bytes += p.size;
                        pr.first_arrival_us.get_or_insert(arrival_us);
                        pr.last_arrival_us = Some(arrival_us);
                    }
                }
            }
            seq = seq.wrapping_add(1);
        }
        let span_us = match (first_arr, last_arr) {
            (Some(a), Some(b)) if b > a => (b - a) as f64,
            _ => 0.0,
        };
        let acked_bps = if span_us > 0.0 { acked_bytes as f64 * 8.0 * 1_000_000.0 / span_us } else { 0.0 };
        let usage = if self.overuse { Usage::Overuse } else { Usage::Normal };
        let (rate, _) = self.aimd.update(now, usage, acked_bps);
        self.est = Some((rate as f64, now));
        self.last_fb_ms = now;
    }

    fn signals(&mut self, now: u64, v2: bool, demand: f64) -> Signals {
        while self.nacks.front().is_some_and(|t| now.saturating_sub(*t) > 1000) {
            self.nacks.pop_front();
        }
        let v2_fresh = v2 && now.saturating_sub(self.last_fb_ms) <= core::V2_STALE_MS;
        let estimate = if v2_fresh { self.est } else { self.remb };
        let loss = self.loss.values().copied().max_by(|a, b| a.0.total_cmp(&b.0));
        Signals { estimate_bps: estimate, overuse: v2_fresh && self.overuse, loss_pct: loss, nack_per_s: self.nacks.len() as f64, drops: 0, demand_bps: demand }
    }
}

pub fn mode(sfu: &Sfu) -> MediaAutolayer {
    sfu.cfg.policy.media.auto_layer
}

pub fn auto_layer(sfu: &Sfu, e: &crate::fwd::Egress) -> u8 {
    if mode(sfu) == MediaAutolayer::Off {
        return crate::sub::MAX_SPATIAL;
    }
    let Some(c) = e.conn.upgrade() else { return crate::sub::MAX_SPATIAL };
    match c.bwe.lock().map(|b| b.policy.layer) {
        Ok(Layer::High) => 1,
        _ => 0,
    }
}

fn sim_rows(conn: &Conn) -> usize {
    conn.egress.load().iter().filter(|e| e.kind == Kind::Video && e.load.load().as_ref().and_then(|l| l.stream.as_ref()).is_some_and(|s| s.simulcast)).count()
}

pub fn tick(sfu: &Arc<Sfu>, conn: &Arc<Conn>, now: u64) {
    let m = mode(sfu);
    if m == MediaAutolayer::Off {
        return;
    }
    let n = sim_rows(conn);
    if n == 0 {
        return;
    }
    let demand = n as f64 * LAYER_H_BPS;
    let Ok(mut b) = conn.bwe.lock() else { return };
    let sig = b.signals(now, m == MediaAutolayer::V2, demand);
    let before = b.policy.layer;
    let d = b.policy.tick(now, &sig);
    match d {
        Decision::Stay => {}
        Decision::Demote(r) => tracing::info!(
            user = conn.user.as_str(),
            "[SIM:AUTO] demote reason={r:?} est={:?} loss={:?} nack={}",
            sig.estimate_bps,
            sig.loss_pct,
            sig.nack_per_s
        ),
        Decision::Promote => tracing::info!(user = conn.user.as_str(), "[SIM:AUTO] promote est={:?}", sig.estimate_bps),
        Decision::ProbeStart { rate_bps } => {
            if m == MediaAutolayer::V2 {
                b.probe = Some(ProbeRun { started: now, rate_bps, acked_bytes: 0, first_arrival_us: None, last_arrival_us: None });
                tracing::info!(user = conn.user.as_str(), "[SIM:PROBE] start rate_bps={rate_bps:.0}");
            } else {
                b.policy.probing = None;
            }
        }
    }
    if before != b.policy.layer {
        sfu.drops.sim_switch.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn probe(sfu: &Arc<Sfu>, conn: &Arc<Conn>, now: u64) {
    let (rate, started, judge) = {
        let Ok(mut b) = conn.bwe.lock() else { return };
        let Some(p) = b.probe.as_ref() else { return };
        let (rate, started) = (p.rate_bps, p.started);
        if now.saturating_sub(started) >= core::PROBE_JUDGE_MS {
            let measured = match (p.first_arrival_us, p.last_arrival_us) {
                (Some(a), Some(z)) if z > a => Some(p.acked_bytes as f64 * 8.0 * 1_000_000.0 / (z - a) as f64),
                _ => None,
            };
            b.probe = None;
            let demand = sim_rows(conn) as f64 * LAYER_H_BPS;
            let r = b.policy.probe_result(now, measured, demand);
            if let Some(m) = r {
                b.est = Some((m, now));
            }
            tracing::info!(user = conn.user.as_str(), "[SIM:PROBE] judge measured={measured:?} promoted_est={r:?}");
            return;
        }
        (rate, started, false)
    };
    let _ = judge;
    if now.saturating_sub(started) >= core::PROBE_DURATION_MS {
        return;
    }
    let Some(e) = conn.egress.load().iter().find(|e| e.kind == Kind::Video && e.rtx_ssrc.is_some() && e.load.load().is_some()).cloned() else { return };
    let bytes = ((rate - LAYER_L_BPS).max(0.0) * core::PROBE_CHUNK_MS as f64 / 1000.0 / 8.0) as usize;
    let count = bytes / core::PROBE_PAD_BYTES;
    let ext = crate::sub::table::SubExt::of(Kind::Video, &conn.ext.load());
    for _ in 0..count.max(1) {
        let (seq, osn, ts) = match e.rw.lock() {
            Ok(mut rw) => {
                rw.rtx_seq = rw.rtx_seq.wrapping_add(1);
                let last = rw.rewriter.last.map(|l| (l.seq, l.ts)).unwrap_or((0, 0));
                (rw.rtx_seq, last.0, last.1)
            }
            Err(_) => return,
        };
        let tw = conn.twcc_out.fetch_add(1, Ordering::Relaxed);
        let t_us = crate::fwd::shell::now_us();
        let mut ext_buf = Vec::new();
        if !ext.is_empty() {
            crate::sub::ext::build(&ext, Some(&e.mid_str), None, t_us, tw, &mut ext_buf);
        }
        let mut out = Vec::new();
        let w = crate::fwd::rtp::Rewrite { pt: e.rtx_pt.load(Ordering::Relaxed), seq, ts, ssrc: e.rtx_ssrc.unwrap_or(0), ext: &ext_buf };
        crate::fwd::rtp::padding_probe(&w, osn, core::PROBE_PAD_BYTES, &mut out);
        if conn.link.send_rtp(&out) {
            if let Ok(mut b) = conn.bwe.lock() {
                b.on_sent(tw as u16, t_us, out.len(), true);
            }
            sfu.drops.probe_sent.fetch_add(1, Ordering::Relaxed);
        }
    }
}
