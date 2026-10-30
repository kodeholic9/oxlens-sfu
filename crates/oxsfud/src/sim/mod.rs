// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-SIM-MODES · 정:IMPL-SIM-SWITCH · 정:IMPL-PUB-RID-SPATIAL · model: claude-opus-5-5
//! 시뮬캐스트 · 자동 층 — 전환 기계(pending 10s · Pause 드롭 · target 키프레임 = 확정 · current 릴레이 · in-band demote) · 수동 상한과 자동 판단의 `min` · v1(RR 손실 · NACK · 드롭) · v2(TWCC 송신측 GCC + RTX 패딩 프로브).

pub mod core;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::Kind;

use crate::fwd::rt::{Worker, media};
use crate::fwd::{DownRow, LAYER_PAUSE};
use crate::link::Transport;
use crate::r#pub::PubTrack;
use core::{Aimd, Trendline, Usage};

pub const H_BPS: u64 = 1_650_000;
pub const L_BPS: u64 = 250_000;
pub const HEADROOM: f64 = 1.2;
pub const DEMOTE_LOSS: f64 = 0.08;
pub const CLEAN_LOSS: f64 = 0.02;
pub const DEMOTE_NACK_PER_S: f64 = 20.0;
pub const DEMOTE_TICKS: u32 = 2;
pub const BACKOFF_MS: u64 = 15_000;
pub const BACKOFF_MAX_MS: u64 = 60_000;
pub const COLLAPSE_MS: u64 = 5_000;
pub const GRACE_MS: u64 = 10_000;
pub const LOSS_FRESH_MS: u64 = 6_000;
pub const PENDING_MS: u64 = 10_000;
pub const INBAND_MS: u64 = 1_000;
pub const PROBE_RUN_MS: u64 = 2_500;
pub const PROBE_JUDGE_MS: u64 = 4_000;
pub const PROBE_GAIN: f64 = 1.35;
pub const PROBE_MAX_BPS: f64 = 3_000_000.0;
pub const V2_STALE_MS: u64 = 1_000;
pub const START_BPS: u64 = 300_000;
pub const DECREASE_GAP_MS: u64 = 300;
pub const PAD_BYTES: usize = 224;
pub const BURST_US: i64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    V1,
    V2,
}

pub fn mode() -> Mode {
    match crate::ops::policy().media.auto_layer {
        common::policy::MediaAutolayer::V1 => Mode::V1,
        common::policy::MediaAutolayer::V2 => Mode::V2,
        common::policy::MediaAutolayer::Off => Mode::Off,
    }
}

pub fn layer_name(l: u8) -> &'static str {
    match l {
        0 => "l",
        1 => "h",
        _ => "pause",
    }
}

fn sibling(t: &PubTrack, layer: u8) -> Option<Arc<PubTrack>> {
    t.stream.physical().into_iter().find(|x| x.rid == Some(layer))
}

pub fn retarget(row: &DownRow, any: &PubTrack, now: u64) {
    let want = row.cap.load(Ordering::Acquire).min(row.auto_cap.load(Ordering::Acquire));
    if row.target.load(Ordering::Acquire) == want {
        return;
    }
    row.target.store(want, Ordering::Release);
    row.pending_since.store(now, Ordering::Release);
    if let Some(t) = sibling(any, want) {
        crate::rtcp::request_pli(&t, true);
    }
}

pub fn pass(row: &DownRow, t: &Arc<PubTrack>, keyframe: bool, now: u64) -> bool {
    if row.paused.load(Ordering::Acquire) {
        return false;
    }
    let l = t.rid.unwrap_or(1);
    let cur = row.layer.load(Ordering::Acquire);
    let tgt = row.target.load(Ordering::Acquire);
    if cur != tgt {
        let since = row.pending_since.load(Ordering::Acquire);
        if cur != LAYER_PAUSE && now.saturating_sub(since) > PENDING_MS {
            row.target.store(cur, Ordering::Release);
            return l == cur;
        }
        if l == tgt && keyframe {
            row.layer.store(tgt, Ordering::Release);
            return true;
        }
        if l == cur {
            return true;
        }
        if cur == LAYER_PAUSE && l != tgt && keyframe && now.saturating_sub(since) > PENDING_MS {
            row.layer.store(l, Ordering::Release);
            row.target.store(l, Ordering::Release);
            return true;
        }
        return false;
    }
    if l == cur {
        return true;
    }
    if cur == 1
        && l == 0
        && let Some(h) = sibling(t, 1)
        && now.saturating_sub(h.last_rtp_ms.load(Ordering::Acquire)) > INBAND_MS
    {
        row.target.store(0, Ordering::Release);
        row.pending_since.store(now, Ordering::Release);
        if keyframe {
            row.layer.store(0, Ordering::Release);
            return true;
        }
        crate::rtcp::request_pli(t, true);
    }
    false
}

pub struct ConnSim {
    pub level: u8,
    pub bad: u32,
    pub last_demote: u64,
    pub last_promote: u64,
    pub backoff: u64,
    pub prev_nacks: HashMap<usize, u64>,
    pub prev_drops: HashMap<usize, u64>,
    pub aimd: Aimd,
    pub trend: Trendline,
    pub fb_at: u64,
    pub last_decrease: u64,
    pub acked: VecDeque<(u64, usize)>,
    pub group: Option<(i64, i64, i64, i64)>,
    pub probe: Option<Probe>,
}

#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub started: u64,
    pub rate: f64,
    pub bytes_acked: u64,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
}

impl Default for ConnSim {
    fn default() -> Self {
        ConnSim {
            level: 1,
            bad: 0,
            last_demote: 0,
            last_promote: 0,
            backoff: BACKOFF_MS,
            prev_nacks: HashMap::new(),
            prev_drops: HashMap::new(),
            aimd: Aimd::new(START_BPS, 200),
            trend: Trendline::new(),
            fb_at: 0,
            last_decrease: 0,
            acked: VecDeque::new(),
            group: None,
            probe: None,
        }
    }
}

fn sim_rows(tr: &Transport) -> Vec<(Arc<DownRow>, Arc<PubTrack>)> {
    let Some(t) = crate::ops::table_of(tr.id) else { return Vec::new() };
    t.rows()
        .into_iter()
        .filter(|r| r.kind == Kind::Video)
        .filter_map(|r| {
            let s = r.src_track.load_full()?;
            if s.stream.simulcast { Some((r, s)) } else { None }
        })
        .collect()
}

pub fn tick(tr: &Arc<Transport>, now: u64) {
    let m = mode();
    if m == Mode::Off {
        return;
    }
    let rows = sim_rows(tr);
    if rows.is_empty() {
        return;
    }
    let Ok(mut s) = tr.rtc.sim.lock() else { return };
    let v2 = m == Mode::V2 && now.saturating_sub(s.fb_at) <= V2_STALE_MS;
    let mut loss = 0.0f64;
    let mut nack_rate = 0.0f64;
    let mut drops = 0u64;
    for (r, _) in &rows {
        let key = Arc::as_ptr(r) as usize;
        if now.saturating_sub(r.rr_at.load(Ordering::Relaxed)) <= LOSS_FRESH_MS {
            loss = loss.max(r.rr_loss.load(Ordering::Relaxed) as f64 / 256.0);
        }
        let n = r.nacks.load(Ordering::Relaxed);
        let p = s.prev_nacks.insert(key, n).unwrap_or(n);
        nack_rate += n.saturating_sub(p) as f64 * 1000.0 / super::rtcp::SIM_MS as f64;
        let d = r.drops.load(Ordering::Relaxed);
        let pd = s.prev_drops.insert(key, d).unwrap_or(d);
        drops += d.saturating_sub(pd);
    }
    let n_rows = rows.len() as f64;
    let demand_h = n_rows * H_BPS as f64 * HEADROOM;
    let demand_now = n_rows * if s.level == 1 { H_BPS } else { L_BPS } as f64 * HEADROOM;
    let est = s.aimd.bps() as f64;
    let in_grace = now.saturating_sub(s.last_promote) < GRACE_MS;
    let short = v2 && !in_grace && s.level == 1 && est < demand_now;
    let bad = short || loss >= DEMOTE_LOSS || nack_rate > DEMOTE_NACK_PER_S || drops > 0;
    if bad {
        s.bad += 1;
    } else {
        s.bad = 0;
    }
    let mut changed = None;
    if s.level == 1 && s.bad >= DEMOTE_TICKS {
        if now.saturating_sub(s.last_promote) < COLLAPSE_MS && !short {
            s.backoff = (s.backoff * 2).min(BACKOFF_MAX_MS);
        }
        s.level = 0;
        s.bad = 0;
        s.last_demote = now;
        changed = Some(0u8);
    } else if s.level == 0 && !bad && loss <= CLEAN_LOSS && now.saturating_sub(s.last_demote) >= s.backoff {
        let usage_ok = s.trend.usage != Some(Usage::Overuse);
        if !v2 || (usage_ok && est >= demand_h) {
            s.level = 1;
            s.last_promote = now;
            changed = Some(1u8);
        } else if v2 && s.probe.is_none() {
            s.probe = Some(Probe { started: now, rate: (demand_h * PROBE_GAIN).min(PROBE_MAX_BPS), bytes_acked: 0, first_at: None, last_at: None });
        }
    }
    if let Some(p) = s.probe
        && now.saturating_sub(p.started) >= PROBE_JUDGE_MS
    {
        let measured = match (p.first_at, p.last_at) {
            (Some(a), Some(b)) if b > a => Some((p.bytes_acked as f64 * 8.0 * 1_000_000.0 / (b - a) as f64) as u64),
            _ => None,
        };
        s.probe = None;
        match measured {
            Some(x) if x as f64 >= demand_h => s.aimd.set_estimate(x, now),
            _ => s.last_demote = now,
        }
    }
    let level = s.level;
    drop(s);
    if let Some(l) = changed {
        tracing::info!(kind = "diag", user = %tr.user, "auto layer → {} (loss {:.3} nack/s {:.1} drops {} v2 {})", layer_name(l), loss, nack_rate, drops, v2);
    }
    for (r, t) in rows {
        r.auto_cap.store(level, Ordering::Release);
        retarget(&r, &t, now);
    }
}

pub fn probe(w: &Worker, tr: &Arc<Transport>, now_us: u64) {
    let active = tr.rtc.sim.lock().ok().and_then(|s| s.probe).filter(|p| common::now_ms().saturating_sub(p.started) < PROBE_RUN_MS);
    let Some(p) = active else { return };
    let rows = sim_rows(tr);
    let Some((row, _)) = rows.first() else { return };
    let Some(m) = media() else { return };
    let Some(rtx_ssrc) = row.rtx_ssrc else { return };
    let rtx_pt = row.rtx_pt.load(Ordering::Relaxed);
    if rtx_pt == 0 {
        return;
    }
    let extra = (p.rate - L_BPS as f64 * rows.len() as f64).max(0.0);
    let bytes = (extra * (crate::rtcp::PROBE_US as f64 / 1_000_000.0) / 8.0) as usize;
    let n = bytes / (PAD_BYTES + 2) + 1;
    let (osn, ts) = match row.rw.lock() {
        Ok(rw) => match rw.last {
            Some(l) => (l.seq, l.ts),
            None => return,
        },
        Err(_) => return,
    };
    let mut payload = Vec::with_capacity(PAD_BYTES + 2);
    payload.extend_from_slice(&osn.to_be_bytes());
    payload.extend(std::iter::repeat_n(0u8, PAD_BYTES - 1));
    payload.push(PAD_BYTES as u8);
    for _ in 0..n {
        let seq = row.rtx_seq.fetch_add(1, Ordering::AcqRel);
        let b = crate::rtcp::Built { pt: rtx_pt, seq, ts, ssrc: rtx_ssrc, marker: false, payload: &payload, probe: true, padding: true };
        crate::rtcp::send_built(w, m, row, b);
    }
    let _ = now_us;
}

pub fn on_twcc(tr: &Arc<Transport>, _t: &crate::sub::Table, body: &[u8]) {
    let Some((base, arrivals)) = crate::rtcp::parse_twcc(body) else { return };
    if mode() != Mode::V2 {
        return;
    }
    let now = common::now_ms();
    let sent: Vec<crate::rtcp::Sent> = match tr.rtc.sent.lock() {
        Ok(g) => g.iter().copied().collect(),
        Err(_) => return,
    };
    let by_seq: HashMap<u16, crate::rtcp::Sent> = sent.into_iter().map(|x| (x.seq, x)).collect();
    let Ok(mut s) = tr.rtc.sim.lock() else { return };
    s.fb_at = now;
    let mut usage = s.trend.usage.unwrap_or(Usage::Normal);
    for (k, a) in arrivals.iter().enumerate() {
        let seq = base.wrapping_add(k as u16);
        let (Some(at), Some(x)) = (a, by_seq.get(&seq)) else { continue };
        let send = x.at_us as i64;
        s.acked.push_back((now, x.size));
        if x.probe
            && let Some(p) = s.probe.as_mut()
        {
            p.bytes_acked += x.size as u64;
            p.first_at.get_or_insert(*at);
            p.last_at = Some(*at);
        }
        match s.group {
            None => s.group = Some((send, *at, send, *at)),
            Some((g_send0, g_recv0, _g_send, _g_recv)) if send - g_send0 <= BURST_US => {
                s.group = Some((g_send0, g_recv0, send, *at));
            }
            Some((_, _, ps, pr)) => {
                let recv_d = (*at - pr) as f64 / 1000.0;
                let send_d = (send - ps) as f64 / 1000.0;
                usage = s.trend.update(recv_d, send_d, *at as f64 / 1000.0);
                s.group = Some((send, *at, send, *at));
            }
        }
    }
    while s.acked.front().map(|(t, _)| now.saturating_sub(*t) > 1_000).unwrap_or(false) {
        s.acked.pop_front();
    }
    let acked_bps = s.acked.iter().map(|(_, b)| *b as u64).sum::<u64>() * 8;
    if usage == Usage::Overuse && now.saturating_sub(s.last_decrease) < DECREASE_GAP_MS {
        return;
    }
    if usage == Usage::Overuse {
        s.last_decrease = now;
    }
    s.aimd.update(now, usage, acked_bps);
}

pub fn subscribe_layer(row: &DownRow, any: &Arc<PubTrack>, spatial: Option<u8>, paused: Option<bool>, priority: Option<u8>, now: u64) {
    if let Some(sp) = spatial {
        row.cap.store(sp.min(1), Ordering::Release);
    }
    if let Some(p) = priority {
        row.priority.store(p, Ordering::Release);
    }
    if let Some(p) = paused {
        let was = row.paused.swap(p, Ordering::AcqRel);
        if was && !p {
            let want = row.cap.load(Ordering::Acquire).min(row.auto_cap.load(Ordering::Acquire));
            if let Some(t) = sibling(any, want).or_else(|| Some(any.clone())) {
                crate::rtcp::request_pli(&t, true);
            }
        }
    }
    retarget(row, any, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(layer_name(0), "l");
        assert_eq!(layer_name(LAYER_PAUSE), "pause");
    }
}
