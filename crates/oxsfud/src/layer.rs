// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-SIM-MODES 정:IMPL-SIM-SWITCH 정:RULE-SIM-TICK 정:IMPL-PUB-RID-SPATIAL 정:RULE-CONC-SIX 6 · model: claude-opus-5-5
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

#[derive(Debug, Default)]
pub struct Bwe {
    pub remb: Option<(u64, u64)>,
    pub loss: VecDeque<(f64, u64)>,
    pub nacks: VecDeque<u64>,
    pub sent: BTreeMap<u16, (usize, u64)>,
    pub delivered: VecDeque<(u64, usize)>,
    pub twcc_at: u64,
    pub state: Tick,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Tick {
    pub layer: Layer,
    pub bad: u32,
    pub backoff: u64,
    pub demoted_at: u64,
    pub promoted_at: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    pub remb: Option<u64>,
    pub loss: Option<f64>,
    pub loss_prev: Option<f64>,
    pub nack_rate: f64,
    pub drops: u64,
    pub demand_h: u64,
}

pub fn policy_tick(now: u64, st: Tick, sig: Signals) -> Tick {
    let mut t = st;
    if t.backoff == 0 {
        t.backoff = BACKOFF_INIT;
    }
    let in_grace = t.promoted_at > 0 && now.saturating_sub(t.promoted_at) < REMB_GRACE_MS;
    let remb_bad = !in_grace && sig.remb.is_some_and(|r| (r as f64) < sig.demand_h as f64 * HEADROOM);
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
                    t.bad = 0;
                }
            } else {
                t.bad = 0;
            }
        }
        Layer::Low | Layer::Pause => {
            let clean = sig.loss.is_none_or(|l| l <= LOSS_CLEAN) && !nack_bad && !drop_bad;
            let fits = sig.remb.is_none_or(|r| (r as f64) >= sig.demand_h as f64 * HEADROOM);
            if clean && fits && now.saturating_sub(t.demoted_at) >= t.backoff {
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

    pub fn sent(&mut self, seq: u16, size: usize, now_ms: u64) {
        self.sent.insert(seq, (size, now_ms));
        while self.sent.len() > 4096 {
            let Some((&k, _)) = self.sent.iter().next() else { break };
            self.sent.remove(&k);
        }
    }

    pub fn on_twcc(&mut self, fb: &TransportLayerCc, now_ms: u64) {
        let mut seq = fb.base_sequence_number;
        let mut statuses: Vec<bool> = Vec::new();
        for c in &fb.packet_chunks {
            match c {
                PacketStatusChunk::RunLengthChunk(r) => {
                    for _ in 0..r.run_length {
                        statuses.push(r.packet_status_symbol != SymbolTypeTcc::PacketNotReceived);
                    }
                }
                PacketStatusChunk::StatusVectorChunk(v) => {
                    for s in &v.symbol_list {
                        statuses.push(*s != SymbolTypeTcc::PacketNotReceived);
                    }
                }
            }
        }
        let mut bytes = 0usize;
        for got in statuses.into_iter().take(fb.packet_status_count as usize) {
            if got && let Some((sz, _)) = self.sent.remove(&seq) {
                bytes += sz;
            }
            seq = seq.wrapping_add(1);
        }
        self.delivered.push_back((now_ms, bytes));
        while self.delivered.front().is_some_and(|(t, _)| now_ms.saturating_sub(*t) > 1_000) {
            self.delivered.pop_front();
        }
        self.twcc_at = now_ms;
    }

    fn twcc_rate(&self, now_ms: u64) -> Option<u64> {
        if self.twcc_at == 0 || now_ms.saturating_sub(self.twcc_at) > V2_STALE_MS {
            return None;
        }
        let bytes: usize = self.delivered.iter().map(|(_, b)| *b).sum();
        Some(bytes as u64 * 8)
    }

    pub fn signals(&self, now: u64, now_wall: u64, mode: MediaAutolayer, demand_h: u64) -> Signals {
        let remb = match mode {
            MediaAutolayer::V2 => self.twcc_rate(now_wall).or_else(|| self.remb.filter(|(_, at)| now.saturating_sub(*at) <= FRESH_REMB_MS).map(|(b, _)| b)),
            _ => self.remb.filter(|(_, at)| now.saturating_sub(*at) <= FRESH_REMB_MS).map(|(b, _)| b),
        };
        let fresh: Vec<f64> = self.loss.iter().filter(|(_, at)| now.saturating_sub(*at) <= FRESH_LOSS_MS).map(|(l, _)| *l).collect();
        Signals {
            remb,
            loss: fresh.last().copied(),
            loss_prev: fresh.len().checked_sub(2).map(|i| fresh[i]),
            nack_rate: self.nacks.len() as f64,
            drops: 0,
            demand_h,
        }
    }
}

pub fn tick(sfu: &Sfu) {
    let mode = sfu.policy.media.auto_layer;
    if matches!(mode, MediaAutolayer::Off) {
        return;
    }
    let now = mono_ms();
    let wall = crate::sfu::now_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let conn = p.sub_conn.clone();
        let sims = lock(&conn.table).rows.iter().filter(|r| r.stream().is_some_and(|s| s.simulcast)).count() as u64;
        if sims == 0 {
            continue;
        }
        let mut b = lock(&conn.bwe);
        let sig = b.signals(now, wall, mode, sims * H_BPS);
        let next = policy_tick(now, b.state, sig);
        if next.layer != b.state.layer {
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
        b.state = next;
        conn.auto_layer.store(next.layer.to_u8(), Ordering::Relaxed);
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
