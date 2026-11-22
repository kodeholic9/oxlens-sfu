// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-SIM-TICK · 정:RULE-SIM-MODES · 정:IMPL-SIM-SWITCH · 정:IMPL-PUB-RID-SPATIAL · model: claude-fable-5-2
//! 시뮬캐스트 · 자동 층 — 전환 기계(pending 10s · Pause 드롭 · target 키프레임 = 확정 · current 릴레이 · in-band demote) · 수동 상한과 자동 판단의 합침 · v1(RR 손실 · NACK · 드롭) · v2(TWCC 송신측 대역 추정 → 목표 속도 한 값 → 층 실측 비트레이트로 배분 · RTX 패딩 탐색 묶음).

pub mod alloc;
pub mod bwe;
pub mod core;
pub mod dd;
pub mod strategy;
pub mod temporal;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::Kind;

use crate::fwd::rt::{Worker, media, mono_us};
use crate::fwd::{DownRow, LAYER_PAUSE, LAYER_TOP};
use crate::link::Transport;
use crate::r#pub::PubTrack;
use alloc::{Allocator, Candidate, Grant, Point};
use bwe::{CONGESTION_PROCESS_US, CONGESTION_START_BPS, Congestion, Fed, Tag};
use core::{Policy, Signals, Usage};

pub const LOSS_FRESH_MS: u64 = 6_000;
pub const PENDING_MS: u64 = 10_000;
pub const PENDING_PLI_RETRY_MS: u64 = 250;
pub const INBAND_MS: u64 = 1_000;
pub const PAD_BYTES: usize = 224;
pub const PATH_OVERHEAD: i64 = 28;
pub const PAD_OVERHEAD: i64 = 66;
pub const LAYER_WINDOW_MS: u64 = 2_500;
pub const LAYER_SLOT_MS: u64 = 100;
pub const LAYER_SLOTS: usize = 25;
pub const TAG_KEEP: usize = 32;

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
        1 => "m",
        LAYER_TOP => "h",
        _ => "pause",
    }
}

fn sibling(t: &PubTrack, layer: u8) -> Option<Arc<PubTrack>> {
    t.stream.physical().into_iter().find(|x| x.rid == Some(layer))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerState {
    pub layer: u8,
    pub target: u8,
    pub since: u64,
}

const SINCE_MASK: u64 = (1 << 60) - 1;

const PAUSE_CODE: u64 = 3;

fn code(l: u8) -> u64 {
    if l > LAYER_TOP { PAUSE_CODE } else { l as u64 }
}

fn layer_of(c: u64) -> u8 {
    if c == PAUSE_CODE { LAYER_PAUSE } else { c as u8 }
}

pub fn live_layers(at: &[u64], first: u64, now: u64) -> u8 {
    if first == 0 || now.saturating_sub(first) <= INBAND_MS {
        return 0;
    }
    at.iter().enumerate().filter(|(_, t)| **t != 0 && now.saturating_sub(**t) <= INBAND_MS).fold(0u8, |m, (i, _)| m | (1 << i))
}

pub fn fit(w: u8, live: u8) -> u8 {
    let has = |x: u8| live & (1 << x) != 0;
    if w > LAYER_TOP || live == 0 || has(w) {
        return w;
    }
    (0..w).rev().find(|&x| has(x)).or_else(|| (w + 1..=LAYER_TOP).find(|&x| has(x))).unwrap_or(w)
}

pub fn v1_layer(level: u8) -> u8 {
    if level == 0 { 0 } else { LAYER_TOP }
}

impl LayerState {
    pub fn pack(&self) -> u64 {
        ((self.since & SINCE_MASK) << 4) | (code(self.layer) << 2) | code(self.target)
    }

    pub fn unpack(w: u64) -> LayerState {
        LayerState { layer: layer_of((w >> 2) & 3), target: layer_of(w & 3), since: w >> 4 }
    }
}

pub fn layer_state(row: &DownRow) -> LayerState {
    LayerState::unpack(row.lstate.load(Ordering::Acquire))
}

fn put(row: &DownRow, s: LayerState) {
    row.lstate.store(s.pack(), Ordering::Release);
}

pub fn merge(cap: u8, auto_cap: u8) -> u8 {
    if auto_cap == LAYER_PAUSE { LAYER_PAUSE } else { cap.min(auto_cap) }
}

pub fn want(row: &DownRow) -> u8 {
    merge(row.cap.load(Ordering::Acquire), row.auto_cap.load(Ordering::Acquire))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pli {
    None,
    Retry(u8),
    Here,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub next: Option<LayerState>,
    pub fwd: bool,
    pub pli: Pli,
}

pub fn step(s: LayerState, l: u8, keyframe: bool, now: u64, retry_due: bool, fall: bool) -> Step {
    let (cur, tgt) = (s.layer, s.target);
    let keep = |fwd, pli| Step { next: None, fwd, pli };
    if cur != tgt {
        if tgt == LAYER_PAUSE {
            return Step { next: Some(LayerState { layer: LAYER_PAUSE, target: LAYER_PAUSE, since: now }), fwd: false, pli: Pli::None };
        }
        if cur != LAYER_PAUSE && now.saturating_sub(s.since) > PENDING_MS {
            return Step { next: Some(LayerState { target: cur, ..s }), fwd: l == cur, pli: Pli::None };
        }
        if l == tgt && keyframe {
            return Step { next: Some(LayerState { layer: tgt, ..s }), fwd: true, pli: Pli::None };
        }
        let pli = if retry_due { Pli::Retry(tgt) } else { Pli::None };
        if l == cur {
            return keep(true, pli);
        }
        if cur == LAYER_PAUSE && l != tgt && keyframe && now.saturating_sub(s.since) > PENDING_MS {
            return Step { next: Some(LayerState { layer: l, target: l, ..s }), fwd: true, pli };
        }
        return keep(false, pli);
    }
    if l == cur {
        return keep(true, Pli::None);
    }
    if cur != LAYER_PAUSE && l < cur && fall {
        if keyframe {
            return Step { next: Some(LayerState { layer: l, target: l, since: now }), fwd: true, pli: Pli::None };
        }
        return Step { next: Some(LayerState { target: l, since: now, ..s }), fwd: false, pli: Pli::Here };
    }
    keep(false, Pli::None)
}

fn layer_at(t: &PubTrack) -> [u64; LAYER_TOP as usize + 1] {
    std::array::from_fn(|i| t.stream.layer_at[i].load(Ordering::Acquire))
}

pub fn live(t: &PubTrack, now: u64) -> u8 {
    live_layers(&layer_at(t), t.stream.first_rtp_ms.load(Ordering::Acquire), now)
}

pub fn adopt_state(row: &DownRow, now: u64, live: u8) -> Option<u8> {
    let s = layer_state(row);
    let w = fit(want(row), live);
    if s.target == w {
        return None;
    }
    put(row, LayerState { target: w, since: now, ..s });
    Some(w)
}

pub fn adopt_asks(adopted: Option<u8>, after: LayerState) -> Option<u8> {
    adopted.filter(|&w| w != LAYER_PAUSE && after.layer != after.target)
}

pub fn adopt(row: &DownRow, any: &PubTrack, now: u64) {
    let adopted = adopt_state(row, now, live(any, now));
    if let Some(w) = adopt_asks(adopted, layer_state(row))
        && let Some(t) = sibling(any, w)
    {
        row.pending_pli_at.store(now, Ordering::Relaxed);
        crate::rtcp::request_pli(&t, crate::rtcp::PLI_ADOPT);
    }
}

pub fn request(row: &DownRow, any: &PubTrack) {
    let w = fit(want(row), live(any, common::now_ms()));
    if layer_state(row).target != w
        && let Some(t) = sibling(any, w)
    {
        crate::rtcp::request_pli(&t, crate::rtcp::PLI_LAYER);
    }
}

pub fn reset(row: &DownRow, now: u64) {
    put(row, LayerState { layer: LAYER_PAUSE, target: want(row), since: now });
    row.t_auto.store(temporal::TL_TOP, Ordering::Release);
    row.tsel.store(temporal::TemporalSel::default().pack(), Ordering::Release);
}

pub fn fall(l: u8, cur: u8, stale: impl Fn(u8) -> Option<bool>) -> bool {
    l < cur && stale(cur) == Some(true) && (l + 1..cur).all(|x| stale(x) != Some(false))
}

fn fall_to(t: &PubTrack, l: u8, cur: u8, now: u64) -> bool {
    let at = layer_at(t);
    fall(l, cur, |x| at.get(x as usize).filter(|v| **v != 0).map(|v| now.saturating_sub(*v) > INBAND_MS))
}

pub fn judge(row: &DownRow, l: u8, keyframe: bool, now: u64, retry_due: bool, live: u8, fall: impl Fn(LayerState) -> bool) -> (Step, Option<u8>) {
    let adopted = adopt_state(row, now, live);
    let s = layer_state(row);
    let st = step(s, l, keyframe, now, retry_due && adopted.is_none(), fall(s));
    if let Some(n) = st.next {
        put(row, n);
    }
    (st, adopted)
}

pub fn admit(row: &DownRow, t: &Arc<PubTrack>, keyframe: bool, now: u64) -> bool {
    if row.paused.load(Ordering::Acquire) {
        return false;
    }
    let l = t.rid.unwrap_or(LAYER_TOP);
    let retry_due = now.saturating_sub(row.pending_pli_at.load(Ordering::Relaxed)) >= PENDING_PLI_RETRY_MS;
    let (st, adopted) = judge(row, l, keyframe, now, retry_due, live(t, now), |s| s.layer == s.target && s.layer != LAYER_PAUSE && l < s.layer && fall_to(t, l, s.layer, now));
    if let Some(w) = adopt_asks(adopted, layer_state(row))
        && let Some(ts) = sibling(t, w)
    {
        row.pending_pli_at.store(now, Ordering::Relaxed);
        crate::rtcp::request_pli(&ts, crate::rtcp::PLI_ADOPT);
    }
    match st.pli {
        Pli::Retry(x) => {
            row.pending_pli_at.store(now, Ordering::Relaxed);
            if let Some(ts) = sibling(t, x) {
                crate::rtcp::request_pli(&ts, crate::rtcp::PLI_RETRY);
            }
        }
        Pli::Here => crate::rtcp::request_pli(t, crate::rtcp::PLI_HERE),
        Pli::None => {}
    }
    st.fwd
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageSeen {
    pub normal: u64,
    pub overuse: u64,
    pub underuse: u64,
}

impl UsageSeen {
    pub fn note(&mut self, u: Usage) {
        match u {
            Usage::Normal => self.normal += 1,
            Usage::Overuse => self.overuse += 1,
            Usage::Underuse => self.underuse += 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LayerRate {
    slots: [u64; LAYER_SLOTS],
    head: u64,
    total: u64,
}

impl LayerRate {
    fn roll(&mut self, now_ms: u64) {
        let slot = now_ms / LAYER_SLOT_MS;
        if slot <= self.head {
            return;
        }
        let from = (self.head + 1).max(slot.saturating_sub(LAYER_SLOTS as u64 - 1));
        if from > self.head + 1 {
            self.slots = [0; LAYER_SLOTS];
            self.total = 0;
        }
        for i in from..=slot {
            let k = (i % LAYER_SLOTS as u64) as usize;
            self.total = self.total.saturating_sub(self.slots[k]);
            self.slots[k] = 0;
        }
        self.head = slot;
    }

    pub fn add(&mut self, now_ms: u64, bytes: u64) {
        self.roll(now_ms);
        let k = (self.head % LAYER_SLOTS as u64) as usize;
        self.slots[k] += bytes;
        self.total += bytes;
    }

    pub fn bitrate(&mut self, now_ms: u64) -> i64 {
        self.roll(now_ms);
        (self.total * 8 * 1000 / LAYER_WINDOW_MS) as i64
    }
}

pub struct ConnSim {
    pub policy: Policy,
    pub prev_rep: HashMap<usize, u64>,
    pub prev_sent: HashMap<usize, u64>,
    pub prev_drops: HashMap<usize, u64>,
    pub congestion: Option<Congestion>,
    pub alloc: Allocator,
    pub priced: Vec<u64>,
    pub allocated: HashMap<usize, bool>,
    pub tags: VecDeque<Tag>,
    pub next_process_us: u64,
    pub usage_seen: UsageSeen,
    pub padding: bool,
    pub twccless: bool,
}

impl Default for ConnSim {
    fn default() -> Self {
        ConnSim {
            policy: Policy::default(),
            prev_rep: HashMap::new(),
            prev_sent: HashMap::new(),
            prev_drops: HashMap::new(),
            congestion: None,
            alloc: Allocator::default(),
            priced: Vec::new(),
            allocated: HashMap::new(),
            tags: VecDeque::new(),
            next_process_us: 0,
            usage_seen: UsageSeen::default(),
            padding: false,
            twccless: false,
        }
    }
}

type Rows = Vec<(Arc<DownRow>, Arc<PubTrack>)>;

fn video_rows(tr: &Transport) -> (Rows, Rows) {
    let Some(t) = crate::ops::table_of(tr.id) else { return (Vec::new(), Vec::new()) };
    let (mut sim, mut fixed) = (Vec::new(), Vec::new());
    for r in t.rows() {
        let Some(s) = r.src_track.load_full() else { continue };
        if r.kind == Kind::Video && s.stream.simulcast {
            sim.push((r, s));
        } else {
            fixed.push((r, s));
        }
    }
    (sim, fixed)
}

fn sim_rows(tr: &Transport) -> Rows {
    video_rows(tr).0
}

pub fn delta(prev: &mut HashMap<usize, u64>, key: usize, total: u64) -> u64 {
    let p = prev.insert(key, total).unwrap_or(total);
    total.saturating_sub(p)
}

fn log_v1(tr: &Transport, sig: &Signals, d: &core::Decision) {
    let Some(l) = d.to.map(v1_layer) else { return };
    if d.reasons.is_empty() {
        tracing::info!(
            kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
            "in.to" = layer_name(l),
            "in.loss" = format!("{:.3}", sig.loss), "in.nack_ratio" = format!("{:.3}", d.nack_ratio), "in.drops" = sig.drops, "in.v2" = false,
            "auto layer → {}", layer_name(l)
        );
        return;
    }
    tracing::info!(
        kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
        "in.to" = layer_name(l), "in.reasons" = d.reasons.iter().map(|r| r.name()).collect::<Vec<_>>().join(","),
        "in.loss" = format!("{:.3}", sig.loss), "in.nack_ratio" = format!("{:.3}", d.nack_ratio), "in.drops" = sig.drops, "in.v2" = false,
        "auto layer → {}", layer_name(l)
    );
}

fn tick_v1(tr: &Arc<Transport>, now: u64) {
    let rows = sim_rows(tr);
    if rows.is_empty() {
        return;
    }
    let Ok(mut s) = tr.rtc.sim.lock() else { return };
    let mut loss = 0.0f64;
    let (mut sent, mut rep) = (0u64, 0u64);
    let mut drops = 0u64;
    for (r, _) in &rows {
        let key = Arc::as_ptr(r) as usize;
        if now.saturating_sub(r.rr_at.load(Ordering::Relaxed)) <= LOSS_FRESH_MS {
            loss = loss.max(r.rr_loss.load(Ordering::Relaxed) as f64 / 256.0);
        }
        rep += delta(&mut s.prev_rep, key, r.nack.lock().map(|l| l.repeats).unwrap_or(0));
        sent += delta(&mut s.prev_sent, key, r.sent.load(Ordering::Relaxed));
        let d = r.drops.load(Ordering::Relaxed);
        let pd = s.prev_drops.insert(key, d).unwrap_or(d);
        drops += d.saturating_sub(pd);
    }
    let sig = Signals { now, loss, sent, repeats: rep, drops };
    let d = s.policy.tick(&sig);
    let level = v1_layer(s.policy.level);
    drop(s);
    log_v1(tr, &sig, &d);
    for (r, t) in rows {
        r.t_auto.store(temporal::TL_TOP, Ordering::Release);
        r.auto_cap.store(level, Ordering::Release);
        adopt(&r, &t, now);
    }
}

fn log_step(tr: &Transport, c: &Congestion, st: &bwe::Step) {
    for o in &st.opened {
        tracing::info!(
            kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
            "in.cluster" = o.id, "in.rate" = o.bitrate, "in.why" = o.why.name(), "in.target" = c.available(),
            "probe open"
        );
    }
    if let Some(j) = st.judged {
        tracing::info!(
            kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
            "in.cluster" = j.id, "in.send" = j.send, "in.recv" = j.recv, "in.result" = j.result.map(|x| x.to_string()).unwrap_or_else(|| "none".into()), "in.valid" = j.result.is_some(),
            "in.target" = c.available(),
            "probe judged"
        );
    }
}

fn spatial_of(code: i8) -> u8 {
    strategy::point_of(code).map(|(s, _)| s).unwrap_or(LAYER_PAUSE)
}

pub fn layer_strategy() -> common::policy::MediaLayerStrategy {
    crate::ops::policy().media.layer_strategy
}

fn price_tl(t: &PubTrack, now_ms: u64) -> [i64; strategy::TL_SLOTS] {
    t.rate_tl.lock().map(|mut r| std::array::from_fn(|i| r[i].bitrate(now_ms))).unwrap_or([0; strategy::TL_SLOTS])
}

pub fn current_point(points: &[Point], spatial: u8, temporal: u8) -> Option<usize> {
    points.iter().rposition(|p| strategy::point_of(p.layer).is_some_and(|(s, t)| s == spatial && t <= temporal))
}

fn price(t: &PubTrack, now_ms: u64) -> i64 {
    t.rate.lock().map(|mut r| r.bitrate(now_ms)).unwrap_or(0)
}

fn shape(cands: &[Candidate]) -> Vec<u64> {
    cands.iter().flat_map(|c| c.points.iter().map(|p| c.id.wrapping_mul(16).wrapping_add((p.layer + 1) as u64))).collect()
}

fn candidates(s: &ConnSim, sim: &Rows, fixed: &Rows, now_ms: u64) -> Vec<Candidate> {
    let mut out = Vec::with_capacity(sim.len() + fixed.len());
    for (r, t) in sim {
        let key = Arc::as_ptr(r) as usize;
        if r.paused.load(Ordering::Acquire) {
            continue;
        }
        let cap = r.cap.load(Ordering::Acquire);
        let raw: Vec<[i64; strategy::TL_SLOTS]> = (0..=LAYER_TOP).map(|l| sibling(t, l).map(|x| price_tl(&x, now_ms)).unwrap_or([0; strategy::TL_SLOTS])).collect();
        let points = strategy::layer_points(layer_strategy(), &raw, cap, r.t_cap.load(Ordering::Acquire));
        let now_s = r.auto_cap.load(Ordering::Acquire);
        let current = if !s.allocated.get(&key).copied().unwrap_or(false) {
            None
        } else if now_s == LAYER_PAUSE {
            Some(0)
        } else {
            current_point(&points, now_s, r.t_auto.load(Ordering::Acquire))
        };
        out.push(Candidate { id: key as u64, weight: 1, points, current });
    }
    for (r, t) in fixed {
        let b = price(t, now_ms);
        out.push(Candidate { id: Arc::as_ptr(r) as usize as u64, weight: 1, points: vec![Point { bitrate: b, layer: 0 }], current: None });
    }
    out
}

fn log_grant(tr: &Transport, g: &Grant, before: u8, available: i64, left: i64, by: &str) {
    tracing::info!(
        kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
        "in.to" = layer_name(spatial_of(g.layer)), "in.to_t" = strategy::point_of(g.layer).map(|(_, t)| t as i64).unwrap_or(-1), "in.from" = layer_name(before), "in.target" = available, "in.left" = left, "in.need" = g.need,
        "in.reason" = g.reason.name(), "in.by" = by, "in.v2" = true,
        "auto layer → {}", layer_name(spatial_of(g.layer))
    );
}

fn desired(sim: &Rows, fixed: &Rows, now_ms: u64) -> i64 {
    let mut total = 0i64;
    for (_, t) in sim {
        total += t.stream.physical().iter().map(|x| price(x, now_ms)).max().unwrap_or(0);
    }
    for (_, t) in fixed {
        total += price(t, now_ms);
    }
    total
}

fn distribute(tr: &Transport, s: &mut ConnSim, sim: &Rows, fixed: &Rows, by: &str, now_ms: u64, now_us: i64) {
    let available = s.congestion.as_ref().map(|c| c.available()).unwrap_or(CONGESTION_START_BPS);
    let cands = candidates(s, sim, fixed, now_ms);
    s.priced = shape(&cands);
    let (grants, left) = s.alloc.distribute(available, &cands);
    for g in &grants {
        let Some((r, t)) = sim.iter().find(|(r, _)| Arc::as_ptr(r) as usize as u64 == g.id) else { continue };
        s.allocated.insert(g.id as usize, true);
        let to = spatial_of(g.layer);
        if let Some((_, tt)) = strategy::point_of(g.layer) {
            r.t_auto.store(tt, Ordering::Release);
        }
        let before = r.auto_cap.swap(to, Ordering::AcqRel);
        if before != to {
            log_grant(tr, g, before, available, left, by);
        }
        adopt(r, t, now_ms);
    }
    wish(tr, s, sim, fixed, now_ms, now_us);
}

fn wish(tr: &Transport, s: &mut ConnSim, sim: &Rows, fixed: &Rows, now_ms: u64, now_us: i64) {
    let want = desired(sim, fixed, now_ms);
    if let Some(c) = s.congestion.as_mut() {
        let st = c.set_desired(want, now_us);
        remember(&mut s.tags, &st);
        if let Some(c) = s.congestion.as_ref() {
            log_step(tr, c, &st);
        }
    }
}

fn remember(tags: &mut VecDeque<Tag>, st: &bwe::Step) {
    for o in &st.opened {
        tags.push_back(Tag { id: o.id, min_probes: o.min_probes, min_bytes: (o.bitrate * o.duration_us + 4_000_000) / 8_000_000, bitrate: o.bitrate });
        while tags.len() > TAG_KEEP {
            tags.pop_front();
        }
    }
}

fn apply(tr: &Transport, s: &mut ConnSim, sim: &Rows, fixed: &Rows, st: bwe::Step, now_ms: u64, now_us: i64) {
    remember(&mut s.tags, &st);
    if let Some(c) = s.congestion.as_ref() {
        log_step(tr, c, &st);
    }
    if st.target.is_some() {
        distribute(tr, s, sim, fixed, st.by.name(), now_ms, now_us);
    }
}

pub fn pad_payload(len: usize) -> Vec<u8> {
    let len = len.clamp(1, PAD_BYTES);
    let mut payload = vec![0u8; len];
    payload[len - 1] = len as u8;
    payload
}

fn pad(w: &Worker, row: &DownRow, len: usize, cluster: u32) -> Option<usize> {
    let m = media()?;
    let rtx_ssrc = row.rtx_ssrc?;
    let rtx_pt = row.rtx_pt.load(Ordering::Relaxed);
    if rtx_pt == 0 {
        return None;
    }
    let ts = {
        let rw = row.rw.lock().ok()?;
        rw.last?.ts
    };
    let payload = pad_payload(len);
    let seq = row.rtx_seq.fetch_add(1, Ordering::AcqRel);
    let b = crate::rtcp::Built { pt: rtx_pt, seq, ts, ssrc: rtx_ssrc, csrc: None, marker: false, payload: &payload, padding: true, cluster };
    crate::rtcp::send_built(w, m, row, b)
}

fn shoot(w: &Worker, tr: &Arc<Transport>, row: &DownRow) -> Option<u64> {
    loop {
        let now_us = mono_us() as i64;
        let shot = {
            let mut s = tr.rtc.sim.lock().ok()?;
            let c = s.congestion.as_mut()?;
            match c.prober.shot(now_us) {
                Some(x) => x,
                None => return c.prober.next_time(now_us).map(|t| t.max(0) as u64),
            }
        };
        let mut bytes = 0i64;
        if shot.first
            && let Some(n) = pad(w, row, 1, shot.tag.id as u32)
        {
            bytes += n as i64 + PATH_OVERHEAD;
        }
        let len = (shot.bytes - bytes - PAD_OVERHEAD).clamp(1, PAD_BYTES as i64) as usize;
        if let Some(n) = pad(w, row, len, shot.tag.id as u32) {
            bytes += n as i64 + PATH_OVERHEAD;
        }
        let mut s = tr.rtc.sim.lock().ok()?;
        let c = s.congestion.as_mut()?;
        if bytes == 0 {
            c.prober = bwe::Prober::default();
            return None;
        }
        c.prober.sent(mono_us() as i64, bytes);
    }
}

fn fill(w: &Worker, tr: &Arc<Transport>, row: &DownRow) {
    let due = match tr.rtc.sim.lock() {
        Ok(mut s) => s.congestion.as_mut().map(|c| c.padding(mono_us() as i64)).unwrap_or(0),
        Err(_) => return,
    };
    let mut left = due;
    while left > 0 {
        let len = (left - PAD_OVERHEAD).clamp(1, PAD_BYTES as i64) as usize;
        let Some(n) = pad(w, row, len, 0) else { return };
        left -= n as i64 + PATH_OVERHEAD;
    }
}

pub fn drive(w: &Worker, tr: &Arc<Transport>, now_ms: u64, now_us: u64) -> Option<u64> {
    match mode() {
        Mode::Off => return None,
        Mode::V1 => {
            if now_ms >= tr.rtc.next_sim.load(Ordering::Relaxed) {
                tr.rtc.next_sim.store(now_ms + crate::rtcp::SIM_MS, Ordering::Relaxed);
                tick_v1(tr, now_ms);
            }
            return None;
        }
        Mode::V2 => {}
    }
    let (sim, fixed) = video_rows(tr);
    if sim.is_empty() {
        return None;
    }
    let twccless = {
        let mut s = tr.rtc.sim.lock().ok()?;
        let now = s.congestion.as_ref().is_some_and(|c| c.twccless(now_us as i64));
        if now != s.twccless {
            s.twccless = now;
            s.priced.clear();
            tracing::info!(
                kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
                "in.twccless" = now, "in.v2" = !now,
                "layer decision by {}", if now { "v1 (no transport-cc feedback from this subscriber)" } else { "v2" }
            );
        }
        now
    };
    if twccless {
        if now_ms >= tr.rtc.next_sim.load(Ordering::Relaxed) {
            tr.rtc.next_sim.store(now_ms + crate::rtcp::SIM_MS, Ordering::Relaxed);
            tick_v1(tr, now_ms);
        }
        return None;
    }
    let carrier = sim.iter().map(|(r, _)| r).find(|r| r.rw.lock().map(|rw| rw.last.is_some()).unwrap_or(false));
    {
        let mut s = tr.rtc.sim.lock().ok()?;
        let known: Vec<usize> = sim.iter().map(|(r, _)| Arc::as_ptr(r) as usize).collect();
        let had = s.allocated.len();
        for k in &known {
            s.allocated.entry(*k).or_insert(false);
        }
        let fresh = s.allocated.len() != had;
        s.allocated.retain(|k, _| known.contains(k));
        let due = now_us >= s.next_process_us;
        if due {
            s.next_process_us = now_us + CONGESTION_PROCESS_US as u64;
        }
        if s.congestion.is_none() {
            if carrier.is_some() {
                let mut c = Congestion::new(CONGESTION_START_BPS, now_us as i64);
                let st = c.set_network(true, now_us as i64);
                s.congestion = Some(c);
                s.next_process_us = now_us + CONGESTION_PROCESS_US as u64;
                apply(tr, &mut s, &sim, &fixed, st, now_ms, now_us as i64);
            }
        } else if due
            && let Some(st) = s.congestion.as_mut().map(|c| c.process(now_us as i64))
        {
            apply(tr, &mut s, &sim, &fixed, st, now_ms, now_us as i64);
            wish(tr, &mut s, &sim, &fixed, now_ms, now_us as i64);
        }
        if (fresh || due) && shape(&candidates(&s, &sim, &fixed, now_ms)) != s.priced {
            distribute(tr, &mut s, &sim, &fixed, "rows", now_ms, now_us as i64);
        }
        let padding = s.congestion.as_ref().is_some_and(|c| c.rate.loss_state == bwe::LossState::Padding);
        if padding != s.padding {
            s.padding = padding;
            let target = s.congestion.as_ref().map(|c| c.available()).unwrap_or(0);
            tracing::info!(
                kind = "diag", spec = "SV-RULE-SIM-TICK", session = %tr.session, user = %tr.user,
                "in.padding" = padding, "in.target" = target, "in.v2" = true,
                "padding to the target {}", if padding { "on" } else { "off" }
            );
        }
    }
    let carrier = carrier?;
    let probe = shoot(w, tr, carrier);
    fill(w, tr, carrier);
    probe
}

pub fn on_rtt(tr: &Transport, rtt_us: i64) {
    if mode() != Mode::V2 {
        return;
    }
    if let Ok(mut s) = tr.rtc.sim.lock()
        && let Some(c) = s.congestion.as_mut()
    {
        c.on_rtt(rtt_us);
    }
}

pub fn on_sent(tr: &Transport, size: usize, now_us: u64, cluster: u32) -> u32 {
    if mode() != Mode::V2 {
        return cluster;
    }
    let Ok(mut s) = tr.rtc.sim.lock() else { return cluster };
    let Some(c) = s.congestion.as_mut() else { return cluster };
    let bytes = size as i64 + PATH_OVERHEAD;
    c.on_sent(bytes, now_us as i64, now_us as i64);
    if cluster != 0 {
        return cluster;
    }
    c.prober.media(now_us as i64, bytes).map(|t| t.id as u32).unwrap_or(0)
}

pub fn on_twcc(tr: &Arc<Transport>, _t: &crate::sub::Table, body: &[u8]) {
    let Some((base, arrivals)) = crate::rtcp::parse_twcc(body) else {
        crate::ops::drop_count("rtcp_malformed");
        return;
    };
    if mode() != Mode::V2 {
        return;
    }
    let now_us = mono_us() as i64;
    let now_ms = common::now_ms();
    let sent: Vec<crate::rtcp::Sent> = match tr.rtc.sent.lock() {
        Ok(g) => g.iter().copied().collect(),
        Err(_) => return,
    };
    let by_seq: HashMap<u16, crate::rtcp::Sent> = sent.into_iter().map(|x| (x.seq, x)).collect();
    let (sim, fixed) = video_rows(tr);
    let Ok(mut s) = tr.rtc.sim.lock() else { return };
    if s.congestion.is_none() {
        return;
    }
    let mut fed: Vec<Fed> = Vec::with_capacity(arrivals.len());
    let mut base_us: Option<i64> = None;
    for (k, a) in arrivals.iter().enumerate() {
        let seq = base.wrapping_add(k as u16);
        let Some(x) = by_seq.get(&seq) else { continue };
        if let Some(at) = a {
            base_us = Some(base_us.map(|b| b.min(*at)).unwrap_or(*at));
        }
        let tag = if x.cluster != 0 { s.tags.iter().find(|t| t.id == x.cluster as i64).copied() } else { None };
        fed.push(Fed { seq: seq as i64, send_us: x.at_us as i64, size: x.size as i64 + PATH_OVERHEAD, recv_us: *a, tag });
    }
    if fed.is_empty() {
        return;
    }
    let base_us = base_us.map(|b| b.div_euclid(64_000) * 64_000).unwrap_or(0);
    let Some((st, usage)) = s.congestion.as_mut().map(|c| (c.on_feedback(&fed, now_us, base_us), c.delay.trend.state())) else { return };
    s.usage_seen.note(usage);
    apply(tr, &mut s, &sim, &fixed, st, now_ms, now_us);
}

pub fn wait_window_ms(target: u8) -> u64 {
    let m = &crate::ops::policy().media;
    2 * if target == 0 { m.pli_throttle_low_ms as u64 } else { m.pli_throttle_high_ms as u64 }
}

pub fn temporal_keep(row: &DownRow, tl: temporal::Tl, key: bool, now_ms: u64) -> bool {
    let mut s = temporal::TemporalSel::unpack(row.tsel.load(Ordering::Acquire));
    let ls = layer_state(row);
    let (auto, wait) = strategy::wait_temporal(
        layer_strategy(),
        ls.layer,
        ls.target,
        LAYER_PAUSE,
        now_ms.saturating_sub(ls.since),
        wait_window_ms(ls.target),
        s.current,
        row.t_auto.load(Ordering::Acquire),
    );
    let target = temporal::goal(row.t_cap.load(Ordering::Acquire), auto);
    let before = s.current;
    let (send, why) = s.step(tl, key, target);
    row.tsel.store(s.pack(), Ordering::Release);
    if let Some(w) = why {
        tracing::info!(
            kind = "diag", spec = "SV-IMPL-SIM-SWITCH", session = %row.conn.session, user = %row.user,
            "in.mid" = %row.mid, "in.from" = before, "in.to" = s.current, "in.target" = target, "in.why" = w.name(), "in.wait" = wait.name(), "in.tid" = tl.tid, "in.sync" = tl.sync,
            "temporal layer → {}", s.current
        );
    }
    if !send {
        row.tskip.fetch_add(1, Ordering::Relaxed);
    }
    send
}

pub fn subscribe_layer(row: &DownRow, any: &Arc<PubTrack>, spatial: Option<u8>, temporal: Option<u8>, paused: Option<bool>, priority: Option<u8>) {
    if let Some(sp) = spatial {
        row.cap.store(sp.min(LAYER_TOP), Ordering::Release);
    }
    if let Some(t) = temporal {
        row.t_cap.store(t.min(temporal::TL_TOP), Ordering::Release);
    }
    if let Some(p) = priority {
        row.priority.store(p, Ordering::Release);
    }
    if let Some(p) = paused {
        let was = row.paused.swap(p, Ordering::AcqRel);
        if was && !p {
            if let Some(t) = sibling(any, fit(want(row), live(any, common::now_ms()))).or_else(|| Some(any.clone())) {
                crate::rtcp::request_pli(&t, crate::rtcp::PLI_LAYER);
            }
            return;
        }
    }
    request(row, any);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(layer_name(0), "l");
        assert_eq!(layer_name(LAYER_PAUSE), "pause");
    }

    #[test]
    fn padding_carries_no_osn() {
        for len in [0usize, 1, 2, 100, PAD_BYTES, PAD_BYTES + 50] {
            let p = pad_payload(len);
            let n = len.clamp(1, PAD_BYTES);
            assert_eq!(p.len(), n, "정:RULE-SIM-TICK padding only");
            assert_eq!(p[n - 1] as usize, n, "정:RULE-SIM-TICK RTP padding count covers the whole payload — no OSN left for the receiver");
            assert!(p[..n - 1].iter().all(|b| *b == 0));
        }
    }

    #[test]
    fn v1_reads_no_subscriber_estimate() {
        let needle = concat!("re", "mb");
        for (name, src) in [("sim/mod.rs", include_str!("mod.rs")), ("sim/core.rs", include_str!("core.rs"))] {
            assert!(!src.to_lowercase().contains(needle), "정:RULE-SIM-TICK v1 has no subscriber bandwidth estimate input · {name}");
        }
    }

    #[test]
    fn delta_is_a_per_tick_window() {
        let mut prev = HashMap::new();
        assert_eq!(delta(&mut prev, 1, 0), 0);
        assert_eq!(delta(&mut prev, 1, 40), 40, "정:RULE-SIM-TICK a burst counts in its tick");
        assert_eq!(delta(&mut prev, 1, 40), 0, "정:RULE-SIM-TICK quiet tick after a burst is zero — the window decays");
        assert_eq!(delta(&mut prev, 2, 500), 0, "정:RULE-SIM-TICK a newly seen row starts its window, not its history");
    }

    #[test]
    fn auto_pause_wins_over_the_manual_cap_and_takes_effect_at_once() {
        assert_eq!(merge(1, LAYER_PAUSE), LAYER_PAUSE, "정:RULE-SIM-TICK automatic pause is not a higher layer");
        assert_eq!(merge(0, 1), 0);
        assert_eq!(merge(1, 0), 0);
        let s = LayerState { layer: 0, target: LAYER_PAUSE, since: 5 };
        let st = step(s, 0, false, 10, false, false);
        assert_eq!(st.next, Some(LayerState { layer: LAYER_PAUSE, target: LAYER_PAUSE, since: 10 }), "정:RULE-SIM-TICK pause needs no key frame");
        assert!(!st.fwd);
        let s = LayerState { layer: LAYER_PAUSE, target: 0, since: 10 };
        assert!(!step(s, 0, false, 20, false, false).fwd, "정:IMPL-SIM-SWITCH resuming waits for the key frame of the target layer");
        assert_eq!(step(s, 0, true, 20, false, false).next, Some(LayerState { layer: 0, target: 0, since: 10 }));
    }

    #[test]
    fn layer_rate_is_the_bytes_inside_the_window() {
        let mut r = LayerRate::default();
        assert_eq!(r.bitrate(1_000), 0);
        r.add(1_000, 125_000);
        r.add(1_950, 125_000);
        assert_eq!(r.bitrate(2_000), 800_000, "정:RULE-SIM-TICK a window not yet full reads low rather than extrapolating");
        r.add(3_300, 250_000);
        assert_eq!(r.bitrate(3_499), 1_600_000);
        assert_eq!(r.bitrate(3_500), 1_200_000, "정:RULE-SIM-TICK bytes older than the window leave it");
        assert_eq!(r.bitrate(4_400), 800_000);
        assert_eq!(r.bitrate(9_000), 0, "정:RULE-SIM-TICK a layer that stopped arriving has no price");
        r.add(1_000_000, 312_500);
        assert_eq!(r.bitrate(1_000_050), 1_000_000);
    }
}
