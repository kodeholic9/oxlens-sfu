// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 정:RULE-SIM-MODES 정:IMPL-SIM-SWITCH 정:RULE-SIM-TICK 정:IMPL-PUB-RID-SPATIAL 정:RULE-CONC-SIX 6 · model: claude-opus-5-5
//! 시뮬캐스트 — 전환 기계(키프레임에서만)는 늘 돌고, 자동 판단(`policy_tick`)은 순수 함수다. 대역 판단은 구독자 전송 단위.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::policy::MediaAutolayer;
use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, SymbolTypeTcc, TransportLayerCc};

use crate::media::{LAYER_H, LAYER_L, PubStream, PubTrack};
use crate::sfu::{Sfu, lock, mono_ms};
use crate::table::Row;

pub const H_BPS: u64 = 1_650_000;
const HEADROOM: f64 = 1.2;
const LOSS_BAD: f64 = 0.08;
const LOSS_CLEAN: f64 = 0.02;
const NACK_BAD: f64 = 20.0;
const DEMOTE_TICKS: u32 = 2;
const BACKOFF_INIT: u64 = 15_000;
const BACKOFF_MAX: u64 = 60_000;
const COLLAPSE_MS: u64 = 5_000;
const REMB_GRACE_MS: u64 = 10_000;
const FRESH_REMB_MS: u64 = 5_000;
const FRESH_LOSS_MS: u64 = 6_000;
const PENDING_MS: u64 = 10_000;
const V2_STALE_MS: u64 = 1_000;
const V2_INIT_BPS: u64 = 300_000;
const V2_FLOOR_BPS: u64 = 30_000;
const PROBE_RUN_MS: u64 = 2_500;
const PROBE_JUDGE_MS: u64 = 4_000;
const PROBE_CHUNK_MS: u64 = 20;
const PROBE_GAIN: f64 = 1.35;
const PROBE_CAP: u64 = 3_000_000;
const PROBE_MIN_SPAN_US: i64 = 1_000_000;
const GROUP_US: u64 = 5_000;
const TREND_WINDOW: usize = 20;
const TREND_GAIN: f64 = 4.0;
const OVERUSE_THRESHOLD: f64 = 12.5;
const OVERUSE_HOLD_MS: u64 = 1_000;
const SMOOTH: f64 = 0.9;
const DECREASE: f64 = 0.85;
const INCREASE_PER_S: f64 = 1.08;
const ACKED_CAP: f64 = 1.5;
const DECREASE_GAP_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    #[default]
    High,
    Low,
    Pause,
}

impl Layer {
    pub fn index(self) -> Option<u8> {
        match self {
            Layer::High => Some(LAYER_H),
            Layer::Low => Some(LAYER_L),
            Layer::Pause => None,
        }
    }
}

#[derive(Debug)]
pub struct SimSel {
    pub cap: u8,
    pub cur: Option<u8>,
    pub target: Option<u8>,
    pub pending_at: u64,
    pub last_h_at: u64,
    pub last_l_at: u64,
}

impl Default for SimSel {
    fn default() -> Self {
        SimSel { cap: LAYER_H, cur: None, target: None, pending_at: 0, last_h_at: 0, last_l_at: 0 }
    }
}

pub fn sim_pass(sfu: &Sfu, row: &Arc<Row>, stream: &PubStream, track: &PubTrack, key: bool, now: u64) -> bool {
    let Some(conn) = row.conn.upgrade() else { return false };
    let auto = Layer::from_u8(conn.auto_layer.load(Ordering::Relaxed));
    let Some(auto_idx) = auto.index() else {
        sfu.counters.drop("sim_pause");
        return false;
    };
    let layer = track.layer;
    let mut s = lock(&row.sim);
    let desired = s.cap.min(auto_idx);
    if layer == LAYER_L {
        s.last_l_at = now;
    } else {
        s.last_h_at = now;
    }
    if s.target != s.cur && s.pending_at > 0 && now.saturating_sub(s.pending_at) > PENDING_MS {
        tracing::info!(kind = "diag", track = stream.track_id.as_str(), "[SIM:PENDING] expired");
        s.target = s.cur;
        s.pending_at = 0;
    }
    if s.target != Some(desired) && s.pending_at == 0 {
        s.target = Some(desired);
        s.pending_at = now;
        drop(s);
        crate::media::request_keyframe(sfu, stream, Some(desired), false);
        s = lock(&row.sim);
    }
    if s.cur == Some(LAYER_H) && layer == LAYER_L && now.saturating_sub(s.last_h_at) > 1_000 && s.target != Some(LAYER_L) {
        s.target = Some(LAYER_L);
        s.pending_at = now;
    }
    if s.cur.is_none() && Some(layer) != s.target {
        let target_seen = match s.target {
            Some(LAYER_L) => s.last_l_at,
            _ => s.last_h_at,
        };
        if now.saturating_sub(target_seen.max(s.pending_at)) > 1_000 {
            s.target = Some(layer);
            s.pending_at = now;
        }
    }
    if Some(layer) == s.target && s.cur != s.target {
        if key {
            s.cur = s.target;
            s.pending_at = 0;
            return true;
        }
        return false;
    }
    Some(layer) == s.cur
}

impl Layer {
    pub fn from_u8(v: u8) -> Layer {
        match v {
            1 => Layer::Low,
            2 => Layer::Pause,
            _ => Layer::High,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Layer::High => 0,
            Layer::Low => 1,
            Layer::Pause => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProbeVerdict {
    #[default]
    Idle,
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Probe {
    pub start: u64,
    pub start_us: u64,
    pub end_us: u64,
    pub target: u64,
    pub need: u64,
    base: u64,
    bytes: u64,
    send: Option<(u64, u64)>,
    arr: Option<(i64, i64)>,
}

impl Probe {
    fn credit(&mut self, send_us: u64, arr_us: i64, size: usize) {
        if send_us < self.start_us || send_us > self.end_us {
            return;
        }
        self.bytes += size as u64;
        self.send = Some(self.send.map_or((send_us, send_us), |(a, b)| (a.min(send_us), b.max(send_us))));
        self.arr = Some(self.arr.map_or((arr_us, arr_us), |(a, b)| (a.min(arr_us), b.max(arr_us))));
    }

    pub fn measured(&self) -> Option<u64> {
        let (s0, s1) = self.send?;
        let (a0, a1) = self.arr?;
        let span = ((s1 - s0) as i64).max(a1 - a0);
        (span >= PROBE_MIN_SPAN_US).then(|| self.bytes * 8 * 1_000_000 / span as u64)
    }
}

#[derive(Debug, Default)]
pub struct Trend {
    group: Option<(u64, u64, i64)>,
    prev: Option<(u64, i64)>,
    acc: f64,
    smooth: f64,
    base: Option<i64>,
    pts: VecDeque<(f64, f64)>,
    over: u32,
    pub overuse_at: u64,
}

impl Trend {
    fn push(&mut self, send_us: u64, arr_us: i64, now: u64) {
        let Some((first, last, last_arr)) = self.group else {
            self.group = Some((send_us, send_us, arr_us));
            return;
        };
        if send_us.saturating_sub(first) <= GROUP_US {
            self.group = Some((first, last.max(send_us), last_arr.max(arr_us)));
            return;
        }
        if let Some((ps, pa)) = self.prev {
            let d = ((last_arr - pa) - (last as i64 - ps as i64)) as f64 / 1_000.0;
            self.acc += d;
            self.smooth = SMOOTH * self.smooth + (1.0 - SMOOTH) * self.acc;
            let base = *self.base.get_or_insert(last_arr);
            self.pts.push_back(((last_arr - base) as f64 / 1_000.0, self.smooth));
            while self.pts.len() > TREND_WINDOW {
                self.pts.pop_front();
            }
            if self.pts.len() == TREND_WINDOW {
                let m = slope(&self.pts) * TREND_GAIN * TREND_WINDOW as f64;
                if m > OVERUSE_THRESHOLD {
                    self.over += 1;
                    if self.over >= 2 {
                        self.overuse_at = now;
                    }
                } else {
                    self.over = 0;
                }
            }
        }
        self.prev = Some((last, last_arr));
        self.group = Some((send_us, send_us, arr_us));
    }

    fn overusing(&self, now: u64) -> bool {
        self.overuse_at > 0 && now.saturating_sub(self.overuse_at) < OVERUSE_HOLD_MS
    }
}

fn slope(pts: &VecDeque<(f64, f64)>) -> f64 {
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let num: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let den: f64 = pts.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    if den == 0.0 { 0.0 } else { num / den }
}

#[derive(Debug, Default)]
pub struct Bwe {
    pub remb: Option<(u64, u64)>,
    pub loss: VecDeque<(f64, u64)>,
    pub nacks: VecDeque<u64>,
    pub sent: BTreeMap<u16, (usize, u64)>,
    pub delivered: VecDeque<(u64, usize)>,
    pub twcc_at: u64,
    pub estimate: u64,
    pub trend: Trend,
    pub probe: Option<Probe>,
    last_change: u64,
    pub state: Tick,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Tick {
    pub layer: Layer,
    pub bad: u32,
    pub backoff: u64,
    pub demoted_at: u64,
    pub promoted_at: u64,
    pub clean_since: u64,
    pub want_probe: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    pub remb: Option<u64>,
    pub loss: Option<f64>,
    pub loss_prev: Option<f64>,
    pub nack_rate: f64,
    pub drops: u64,
    pub demand_h: u64,
    pub v2: bool,
    pub overuse: bool,
    pub probe: ProbeVerdict,
}

pub fn policy_tick(now: u64, st: Tick, sig: Signals) -> Tick {
    let mut t = st;
    t.want_probe = false;
    if t.backoff == 0 {
        t.backoff = BACKOFF_INIT;
    }
    let need = sig.demand_h as f64 * HEADROOM;
    let in_grace = t.promoted_at > 0 && now.saturating_sub(t.promoted_at) < REMB_GRACE_MS;
    let remb_bad = !in_grace && sig.remb.is_some_and(|r| (r as f64) < need);
    let loss_bad = sig.loss.is_some_and(|l| l > LOSS_BAD) && sig.loss_prev.is_some_and(|l| l > LOSS_BAD);
    let nack_bad = sig.nack_rate > NACK_BAD;
    let drop_bad = sig.drops > 0;
    let bad = remb_bad || loss_bad || nack_bad || drop_bad;
    match t.layer {
        Layer::High => {
            if bad {
                t.bad += 1;
                if t.bad >= DEMOTE_TICKS {
                    let collapsed = t.promoted_at > 0 && now.saturating_sub(t.promoted_at) < COLLAPSE_MS;
                    if collapsed && !remb_bad {
                        t.backoff = (t.backoff * 2).min(BACKOFF_MAX);
                    }
                    t.layer = Layer::Low;
                    t.demoted_at = now;
                    t.clean_since = now;
                    t.bad = 0;
                }
            } else {
                t.bad = 0;
            }
        }
        Layer::Low | Layer::Pause => {
            let clean = sig.loss.is_none_or(|l| l <= LOSS_CLEAN) && !nack_bad && !drop_bad && !sig.overuse;
            if !clean {
                t.clean_since = now;
            }
            let held = clean && now.saturating_sub(t.demoted_at.max(t.clean_since)) >= t.backoff;
            let promote = if sig.v2 {
                match sig.probe {
                    ProbeVerdict::Passed => clean,
                    ProbeVerdict::Failed => {
                        t.clean_since = now;
                        false
                    }
                    ProbeVerdict::Running => false,
                    ProbeVerdict::Idle => {
                        t.want_probe = held;
                        false
                    }
                }
            } else {
                held && sig.remb.is_none_or(|r| (r as f64) >= need)
            };
            if promote {
                t.layer = Layer::High;
                t.promoted_at = now;
            }
        }
    }
    t
}

impl Bwe {
    pub fn on_remb(&mut self, bps: u64, now: u64) {
        self.remb = Some((bps, now));
    }

    pub fn on_loss(&mut self, fraction: u8, now: u64) {
        self.loss.push_back((fraction as f64 / 256.0, now));
        while self.loss.len() > 4 {
            self.loss.pop_front();
        }
    }

    pub fn on_nack(&mut self, now: u64) {
        self.nacks.push_back(now);
        while self.nacks.front().is_some_and(|t| now.saturating_sub(*t) > 1_000) {
            self.nacks.pop_front();
        }
    }

    pub fn sent(&mut self, seq: u16, size: usize, now_us: u64) {
        self.sent.insert(seq, (size, now_us));
        while self.sent.len() > 8192 {
            let Some((&k, _)) = self.sent.iter().next() else { break };
            self.sent.remove(&k);
        }
    }

    pub fn on_twcc(&mut self, fb: &TransportLayerCc, now: u64) {
        let mut statuses: Vec<SymbolTypeTcc> = Vec::new();
        for c in &fb.packet_chunks {
            match c {
                PacketStatusChunk::RunLengthChunk(r) => statuses.extend(std::iter::repeat_n(r.packet_status_symbol, r.run_length as usize)),
                PacketStatusChunk::StatusVectorChunk(v) => statuses.extend(v.symbol_list.iter().copied()),
            }
        }
        let mut seq = fb.base_sequence_number;
        let mut arr = fb.reference_time as i64 * 64_000;
        let mut deltas = fb.recv_deltas.iter();
        let mut bytes = 0usize;
        for sym in statuses.into_iter().take(fb.packet_status_count as usize) {
            if sym != SymbolTypeTcc::PacketNotReceived {
                arr += deltas.next().map(|d| d.delta).unwrap_or_default();
                if let Some((sz, sent_us)) = self.sent.remove(&seq) {
                    bytes += sz;
                    self.trend.push(sent_us, arr, now);
                    if let Some(p) = self.probe.as_mut() {
                        p.credit(sent_us, arr, sz);
                    }
                }
            } else {
                self.sent.remove(&seq);
            }
            seq = seq.wrapping_add(1);
        }
        self.delivered.push_back((now, bytes));
        while self.delivered.front().is_some_and(|(t, _)| now.saturating_sub(*t) > 1_000) {
            self.delivered.pop_front();
        }
        self.twcc_at = now;
        self.update_estimate(now);
    }

    pub fn acked_bps(&self) -> u64 {
        self.delivered.iter().map(|(_, b)| *b as u64).sum::<u64>() * 8
    }

    fn update_estimate(&mut self, now: u64) {
        if self.estimate == 0 {
            self.estimate = V2_INIT_BPS;
            self.last_change = now;
            return;
        }
        let acked = self.acked_bps();
        if self.trend.overuse_at == now {
            if now.saturating_sub(self.last_change) >= DECREASE_GAP_MS {
                self.estimate = self.estimate.min((acked as f64 * DECREASE) as u64).max(V2_FLOOR_BPS);
                self.last_change = now;
            }
            return;
        }
        if self.trend.overusing(now) {
            return;
        }
        let dt = now.saturating_sub(self.last_change) as f64 / 1_000.0;
        let grown = (self.estimate as f64 * INCREASE_PER_S.powf(dt)) as u64;
        let cap = self.estimate.max((acked as f64 * ACKED_CAP) as u64);
        self.estimate = grown.min(cap);
        self.last_change = now;
    }

    fn v2_fresh(&self, now: u64) -> bool {
        self.twcc_at > 0 && now.saturating_sub(self.twcc_at) <= V2_STALE_MS
    }

    fn verdict(&mut self, now: u64) -> ProbeVerdict {
        let Some(p) = self.probe else { return ProbeVerdict::Idle };
        if p.measured().is_some_and(|m| m >= p.need) {
            self.estimate = self.estimate.max(p.measured().unwrap_or_default());
            return ProbeVerdict::Passed;
        }
        if now.saturating_sub(p.start) >= PROBE_JUDGE_MS {
            return ProbeVerdict::Failed;
        }
        ProbeVerdict::Running
    }

    pub fn start_probe(&mut self, now: u64, now_us: u64, need: u64) {
        let target = ((need as f64 * PROBE_GAIN) as u64).min(PROBE_CAP);
        let base = self.acked_bps().min(target);
        self.probe = Some(Probe { start: now, start_us: now_us, end_us: now_us + PROBE_RUN_MS * 1_000, target, need, base, ..Probe::default() });
    }

    pub fn probe_padding(&self, now: u64) -> usize {
        match self.probe {
            Some(p) if now.saturating_sub(p.start) < PROBE_RUN_MS => {
                (p.target.saturating_sub(p.base) * PROBE_CHUNK_MS / 1_000 / 8) as usize
            }
            _ => 0,
        }
    }

    pub fn signals(&mut self, now: u64, mode: MediaAutolayer, demand_h: u64) -> Signals {
        let v2 = matches!(mode, MediaAutolayer::V2) && self.v2_fresh(now);
        let remb = if v2 {
            Some(self.estimate)
        } else {
            self.remb.filter(|(_, at)| now.saturating_sub(*at) <= FRESH_REMB_MS).map(|(b, _)| b)
        };
        let fresh: Vec<f64> = self.loss.iter().filter(|(_, at)| now.saturating_sub(*at) <= FRESH_LOSS_MS).map(|(l, _)| *l).collect();
        let probe = if v2 { self.verdict(now) } else { ProbeVerdict::Idle };
        Signals {
            remb,
            loss: fresh.last().copied(),
            loss_prev: fresh.len().checked_sub(2).map(|i| fresh[i]),
            nack_rate: self.nacks.len() as f64,
            drops: 0,
            demand_h,
            v2,
            overuse: v2 && self.trend.overusing(now),
            probe,
        }
    }
}

pub fn tick(sfu: &Sfu) {
    let mode = sfu.policy.media.auto_layer;
    if matches!(mode, MediaAutolayer::Off) {
        return;
    }
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let conn = p.sub_conn.clone();
        let sims = lock(&conn.table).rows.iter().filter(|r| r.stream().is_some_and(|s| s.simulcast)).count() as u64;
        if sims == 0 {
            continue;
        }
        let mut b = lock(&conn.bwe);
        let sig = b.signals(now, mode, sims * H_BPS);
        let next = policy_tick(now, b.state, sig);
        if matches!(sig.probe, ProbeVerdict::Passed | ProbeVerdict::Failed) {
            tracing::info!(
                kind = "diag",
                spec = "RULE-SIM-TICK",
                user = p.user_id.as_str(),
                verdict = format!("{:?}", sig.probe).as_str(),
                measured = format!("{:?}", b.probe.and_then(|x| x.measured())).as_str(),
                "probe"
            );
            b.probe = None;
        }
        if next.layer != b.state.layer {
            b.probe = None;
            tracing::info!(
                kind = "diag",
                spec = "RULE-SIM-TICK",
                user = p.user_id.as_str(),
                from = format!("{:?}", b.state.layer).as_str(),
                to = format!("{:?}", next.layer).as_str(),
                remb = format!("{:?}", sig.remb).as_str(),
                loss = format!("{:?}", sig.loss).as_str(),
                nack = sig.nack_rate,
                "auto layer"
            );
        }
        if next.want_probe && b.probe.is_none() {
            let need = (sims as f64 * H_BPS as f64 * HEADROOM) as u64;
            b.start_probe(now, crate::sfu::mono_us(), need);
        }
        b.state = next;
        conn.auto_layer.store(next.layer.to_u8(), Ordering::Relaxed);
    }
}

pub fn probe_tick(sfu: &Sfu) {
    if !matches!(sfu.policy.media.auto_layer, MediaAutolayer::V2) {
        return;
    }
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let conn = p.sub_conn.clone();
        let mut budget = lock(&conn.bwe).probe_padding(now);
        if budget == 0 {
            continue;
        }
        let row = lock(&conn.table).rows.iter().find(|r| r.rtx_ssrc.is_some() && r.stream().is_some_and(|s| s.simulcast) && lock(&r.rw).started).cloned();
        let Some(row) = row else { continue };
        while budget > 0 {
            let Some(size) = crate::rtcp_io::send_padding(&row, &conn) else { break };
            budget = budget.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demote_needs_two_ticks_and_promote_needs_backoff() {
        let bad = Signals { loss: Some(0.2), loss_prev: Some(0.2), demand_h: H_BPS, ..Signals::default() };
        let t1 = policy_tick(1_000, Tick::default(), bad);
        assert_eq!(t1.layer, Layer::High);
        let t2 = policy_tick(2_000, t1, bad);
        assert_eq!(t2.layer, Layer::Low);
        let clean = Signals { loss: Some(0.0), loss_prev: Some(0.0), demand_h: H_BPS, ..Signals::default() };
        assert_eq!(policy_tick(10_000, t2, clean).layer, Layer::Low);
        assert_eq!(policy_tick(2_000 + BACKOFF_INIT, t2, clean).layer, Layer::High);
    }
}
