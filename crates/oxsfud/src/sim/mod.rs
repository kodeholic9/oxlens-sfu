// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-SIM-MODES · 정:IMPL-SIM-SWITCH · 정:RULE-SIM-TICK · 정:CAND-SIM-PROBE-SHAPE · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-RTCP-TERMINATE · model: claude-opus-5-5
//! 시뮬캐스트 · 자동 판단 — 구독 전송로마다: v1 = REMB + RR 손실 · v2 = 서버 교체 스탬핑 TWCC 피드백 → trendline · AIMD(신호 1s 불신선이면 v1) · RTX 패딩 프로브. 판단은 `core::Policy::tick`(순수) · 여기는 신호를 모아 넣고 결과를 다운링크에 건다.

pub mod core;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::types::Kind;

use self::core::{Aimd, Decision, Layer, Policy, Signals, Trendline, Usage};
use crate::fwd::hot::Downlink;
use crate::link::Transport;

#[derive(Debug, Clone)]
pub struct Probe {
    pub start_ms: u64,
    pub until_ms: u64,
    pub judge_ms: u64,
    pub rate_bps: f64,
    pub demand_bps: f64,
    pub seqs: Vec<u16>,
    pub acked_bytes: u64,
    pub first_us: Option<i64>,
    pub last_us: Option<i64>,
}

impl Probe {
    pub fn measured_bps(&self) -> Option<f64> {
        let (a, b) = (self.first_us?, self.last_us?);
        if b <= a {
            return None;
        }
        Some(self.acked_bytes as f64 * 8.0 * 1_000_000.0 / (b - a) as f64)
    }
}

#[derive(Debug)]
pub struct State {
    pub aimd: Aimd,
    pub trend: Trendline,
    pub est: Option<(f64, u64)>,
    pub policy: Policy,
    pub acked: VecDeque<(i64, usize)>,
    pub prev: Option<(i64, i64)>,
    pub probe: Option<Probe>,
    pub last: Decision,
}

impl State {
    pub fn new(now_ms: u64) -> State {
        State { aimd: Aimd::new(core::START_BPS), trend: Trendline::new(), est: None, policy: Policy::new(now_ms), acked: VecDeque::new(), prev: None, probe: None, last: Decision::Stay }
    }
}

pub fn parse_feedback(raw: &[u8]) -> Vec<(u16, Option<i64>)> {
    if raw.len() < 20 {
        return Vec::new();
    }
    let base = u16::from_be_bytes([raw[12], raw[13]]);
    let count = usize::from(u16::from_be_bytes([raw[14], raw[15]]));
    let ref_time = (i64::from(raw[16]) << 16) | (i64::from(raw[17]) << 8) | i64::from(raw[18]);
    let mut syms: Vec<u8> = Vec::with_capacity(count);
    let mut i = 20;
    while syms.len() < count && i + 2 <= raw.len() {
        let c = u16::from_be_bytes([raw[i], raw[i + 1]]);
        i += 2;
        if c & 0x8000 == 0 {
            let s = ((c >> 13) & 0x3) as u8;
            let run = usize::from(c & 0x1fff);
            for _ in 0..run.min(count - syms.len()) {
                syms.push(s);
            }
        } else if c & 0x4000 == 0 {
            for k in (0..14).rev() {
                if syms.len() < count {
                    syms.push(((c >> k) & 1) as u8);
                }
            }
        } else {
            for k in (0..7).rev() {
                if syms.len() < count {
                    syms.push(((c >> (k * 2)) & 3) as u8);
                }
            }
        }
    }
    let mut at = ref_time * 64_000;
    let mut out = Vec::with_capacity(count);
    for (k, s) in syms.iter().enumerate() {
        let seq = base.wrapping_add(k as u16);
        match s {
            1 if i < raw.len() => {
                at += i64::from(raw[i]) * 250;
                i += 1;
                out.push((seq, Some(at)));
            }
            2 if i + 1 < raw.len() => {
                at += i64::from(i16::from_be_bytes([raw[i], raw[i + 1]])) * 250;
                i += 2;
                out.push((seq, Some(at)));
            }
            _ => out.push((seq, None)),
        }
    }
    out
}

pub fn on_feedback(t: &Arc<Transport>, raw: &[u8], now_ms: u64) {
    let items = parse_feedback(raw);
    if items.is_empty() {
        return;
    }
    let sent: Vec<(u16, i64, usize)> = t.ingress.lock().map(|i| i.sent.iter().copied().collect()).unwrap_or_default();
    let Ok(mut st) = t.sim.lock() else { return };
    let mut usage = st.trend.usage();
    for (seq, arr) in items {
        let Some(arr) = arr else { continue };
        let Some(&(_, send_us, bytes)) = sent.iter().rev().find(|(s, _, _)| *s == seq) else { continue };
        if let Some((ps, pa)) = st.prev {
            let sd = (send_us - ps) as f64 / 1_000.0;
            let rd = (arr - pa) as f64 / 1_000.0;
            if sd >= f64::from(core::BURST_MS as i32) || sd < 0.0 {
                usage = st.trend.update(rd, sd, arr as f64 / 1_000.0);
                st.prev = Some((send_us, arr));
            }
        } else {
            st.prev = Some((send_us, arr));
        }
        st.acked.push_back((arr, bytes));
        if let Some(p) = st.probe.as_mut()
            && p.seqs.contains(&seq)
        {
            p.acked_bytes += bytes as u64;
            p.first_us = Some(p.first_us.map_or(arr, |f| f.min(arr)));
            p.last_us = Some(p.last_us.map_or(arr, |l| l.max(arr)));
        }
    }
    let newest = st.acked.back().map(|a| a.0).unwrap_or(0);
    while st.acked.front().is_some_and(|(a, _)| newest - *a > 500_000) {
        st.acked.pop_front();
    }
    let span = st.acked.front().map(|a| (newest - a.0).max(1)).unwrap_or(1);
    let bytes: usize = st.acked.iter().map(|a| a.1).sum();
    let acked_bps = bytes as f64 * 8.0 * 1_000_000.0 / span.max(100_000) as f64;
    let rate = st.aimd.update(usage, acked_bps, now_ms);
    st.est = Some((rate as f64, now_ms));
}

fn sim_downlinks(t: &Transport) -> Vec<Arc<Downlink>> {
    t.downlinks.load().values().filter(|d| d.kind == Kind::Video && !d.slot && d.ssrc != 0).filter(|d| d.carry.load().is_some()).cloned().collect::<Vec<_>>().into_iter().filter(|d| simulcast_of(d)).fold(Vec::new(), |mut v, d| {
        if !v.iter().any(|x: &Arc<Downlink>| Arc::ptr_eq(x, &d)) {
            v.push(d);
        }
        v
    })
}

fn simulcast_of(d: &Downlink) -> bool {
    d.rewrite.lock().ok().and_then(|r| r.src.as_ref().and_then(|w| w.upgrade())).and_then(|u| u.stream.upgrade()).is_some_and(|s| s.simulcast)
}

pub fn tick(t: &Arc<Transport>, now_ms: u64, mode: common::policy::MediaAutolayer) {
    if mode == common::policy::MediaAutolayer::Off {
        return;
    }
    let dls = sim_downlinks(t);
    if dls.is_empty() {
        return;
    }
    let (remb, loss, nacks) = t.ingress.lock().map(|i| (i.remb_bps, i.loss, i.nacks.len())).unwrap_or((None, None, 0));
    let Ok(mut st) = t.sim.lock() else { return };
    let v2_fresh = st.est.filter(|(_, at)| now_ms.saturating_sub(*at) <= core::V2_STALE_MS);
    let (est, est_at) = match (mode, v2_fresh, remb) {
        (common::policy::MediaAutolayer::V2, Some((e, at)), _) => (Some(e), at),
        (_, _, Some((b, at))) => (Some(b as f64), at),
        _ => (None, 0),
    };
    let sig = Signals {
        estimate_bps: est,
        estimate_at_ms: est_at,
        loss_pct: loss.map(|l| l.0),
        loss_at_ms: loss.map(|l| l.1).unwrap_or(0),
        nack_per_s: nacks as f64,
        drops: 0,
        overusing: st.trend.usage() == Usage::Overuse,
        streams: dls.len() as u32,
    };
    let before = st.policy.layer;
    let mut d = st.policy.tick(now_ms, &sig);
    if let Some(p) = st.probe.clone()
        && now_ms >= p.judge_ms
    {
        st.probe = None;
        match p.measured_bps() {
            Some(m) if m >= p.demand_bps => {
                st.est = Some((m, now_ms));
            }
            _ => st.policy.restart_clean_window(now_ms),
        }
    }
    let demand = f64::from(sig.streams) * core::LAYER_H_BPS * core::HEADROOM;
    if mode == common::policy::MediaAutolayer::V2 && st.policy.layer == Layer::Low && d == Decision::Stay && st.probe.is_none() && est.is_some_and(|e| e < demand) && sig.loss_pct.is_none_or(|l| l <= core::LOSS_CLEAN) {
        let rate = (demand * core::PROBE_FACTOR).min(core::PROBE_MAX_BPS);
        st.probe = Some(Probe { start_ms: now_ms, until_ms: now_ms + core::PROBE_DURATION_MS, judge_ms: now_ms + core::PROBE_JUDGE_MS, rate_bps: rate, demand_bps: demand, seqs: Vec::new(), acked_bytes: 0, first_us: None, last_us: None });
    }
    let after = st.policy.layer;
    if before == after {
        d = Decision::Stay;
    }
    st.last = d;
    drop(st);
    tracing::info!(transport = t.id, est = ?sig.estimate_bps, loss = ?sig.loss_pct, nack = sig.nack_per_s, overuse = sig.overusing, decision = ?d, "자동 판단 입력");
    if d != Decision::Stay {
        let target = if after == Layer::High { 1 } else { 0 };
        for dl in dls {
            dl.auto_layer.store(target, Ordering::Release);
            crate::fwd::egress::target_layer_pli(&dl, now_ms);
        }
    }
}

pub fn probe_chunk(t: &Arc<Transport>, now_ms: u64) {
    let (rate, active) = match t.sim.lock() {
        Ok(s) => match &s.probe {
            Some(p) if now_ms < p.until_ms => (p.rate_bps, true),
            _ => (0.0, false),
        },
        Err(_) => return,
    };
    if !active {
        return;
    }
    let Some(dl) = sim_downlinks(t).into_iter().next() else { return };
    let bits = ((rate - core::LAYER_L_BPS).max(0.0) * core::PROBE_CHUNK_MS as f64 / 1_000.0) as usize;
    let count = (bits / 8 / 240).clamp(1, 64);
    for _ in 0..count {
        if let Some(seq) = crate::fwd::egress::send_probe(t, &dl)
            && let Ok(mut s) = t.sim.lock()
            && let Some(p) = s.probe.as_mut()
        {
            p.seqs.push(seq);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_parse_round_trip() {
        let fb = crate::rtcp::core::Feedback {
            base_seq: 10,
            status_count: 4,
            reference_time: 15,
            fb_count: 0,
            symbols: vec![crate::rtcp::core::Sym::Small, crate::rtcp::core::Sym::NotReceived, crate::rtcp::core::Sym::Large, crate::rtcp::core::Sym::Small],
            deltas_250us: vec![160, -40, 80],
        };
        let raw = crate::rtcp::core::encode_twcc(1, 2, &fb);
        let p = parse_feedback(&raw);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], (10, Some(15 * 64_000 + 160 * 250)));
        assert_eq!(p[1], (11, None));
        assert_eq!(p[2].1, Some(15 * 64_000 + 120 * 250));
    }
}
