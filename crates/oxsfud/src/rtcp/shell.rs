// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-RTCP-NACK-DOWN · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-SIM-TICK · model: claude-opus-5-5
//! RTCP 종단 — 발행자 축(SR 받아 번역 · RR 1s · TWCC 100ms · REMB 1s · NACK 상향) · 구독자 축(RR 소비 · NACK → RTX 관문 넷 · PLI/FIR 스로틀 · REMB · TWCC 피드백 → v2) · `1pc` 는 평문에서 패킷 단위로 가른다.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use oxsig::trk::MediaKind;

use super::core::{self as rc, nack_pairs};
use crate::fwd::core::{self as fc, Egress, Sr};
use crate::link::table::Class;
use crate::link::{lock, Down, Peer, Transport};
use crate::r#pub::PubTrack;
use crate::sim::core::{policy_tick, Layer, Probe, Signals, HEADROOM, H_BPS, L_BPS, PLI_H_MS, PLI_L_MS, PROBE_CHUNK_MS};

pub const NACK_BUDGET: u32 = 200;
pub const NACK_WINDOW_MS: u64 = 3000;

fn now_ms() -> u64 {
    common::clock::wall_ms()
}

pub fn ingress(t: &Arc<Transport>, b: &[u8]) {
    let Some(peer) = t.peer.upgrade() else { return };
    for p in rc::split(b) {
        match (p.pt, p.count) {
            (200, _) => on_sr(t, p.body),
            (201, _) => on_rr(t, &peer, p.body, p.count),
            (205, 1) => on_nack(&peer, p.body),
            (205, 15) => on_twcc(t, p.body),
            (206, 1) | (206, 4) => on_pli(&peer, p.body, p.pt == 206 && p.count == 4),
            (206, 15) => {
                if let Some(bps) = p.body.get(8..).and_then(rc::remb_bps) {
                    lock(&t.sim).remb = Some((bps as f64, now_ms()));
                }
            }
            _ => {}
        }
    }
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

fn on_sr(t: &Arc<Transport>, body: &[u8]) {
    let (Some(ssrc), Some(hi), Some(lo), Some(ts)) = (u32_at(body, 0), u32_at(body, 4), u32_at(body, 8), u32_at(body, 12)) else { return };
    let Some(phys) = t.ingress.get(&ssrc).map(|p| p.clone()) else { return };
    let sr = Sr { ssrc, ntp: ((hi as u64) << 32) | lo as u64, rtp_ts: ts };
    lock(&phys.sr).replace((sr, now_ms()));
    let Some(stream) = phys.stream.upgrade() else { return };
    for w in stream.subs.load().iter() {
        if let Some(d) = w.upgrade() {
            send_sr(&d, &phys, sr);
        }
    }
    if stream.half.load(Ordering::Acquire)
        && let Some(ctx) = crate::cfg::get()
            && let Some(room) = stream.room.lock().ok().and_then(|r| ctx.rooms.get(&r)) {
                let slot = room.slot(stream.kind);
                for w in slot.subs.load().iter() {
                    if let Some(d) = w.upgrade() {
                        send_sr(&d, &phys, sr);
                    }
                }
            }
}

fn send_sr(d: &Arc<Down>, phys: &Arc<PubTrack>, sr: Sr) {
    let off = {
        let st = lock(&d.st);
        if st.src_ssrc != phys.ssrc || st.rw.last.is_none() {
            return;
        }
        st.rw.ts_off
    };
    let Some(p) = d.peer.upgrade() else { return };
    let x = fc::translate_sr(sr, d.ssrc, off);
    let pkt = rc::encode_sr(x.ssrc, x.ntp, x.rtp_ts, d.sent.load(Ordering::Relaxed) as u32, d.bytes.load(Ordering::Relaxed) as u32);
    p.sub_transport().send_rtcp(&pkt);
}

fn on_rr(t: &Arc<Transport>, peer: &Arc<Peer>, body: &[u8], count: u8) {
    let mut worst: Option<f64> = None;
    for k in 0..count as usize {
        let i = 4 + k * 24;
        let Some(ssrc) = u32_at(body, i) else { break };
        let Some(&frac) = body.get(i + 4) else { break };
        if lock(&peer.table).row_by_ssrc(ssrc).is_some() {
            let f = frac as f64 / 256.0;
            worst = Some(worst.map(|w: f64| w.max(f)).unwrap_or(f));
        }
    }
    if let Some(w) = worst {
        lock(&t.sim).loss = Some((w, now_ms()));
    }
}

fn down_of(peer: &Arc<Peer>, ssrc: u32) -> Option<(Arc<Down>, bool)> {
    let (mid, rtx) = {
        let t = lock(&peer.table);
        let r = t.row_by_ssrc(ssrc)?;
        (r.mid, r.rtx_ssrc == Some(ssrc))
    };
    Some((peer.down(mid)?, rtx))
}

fn on_nack(peer: &Arc<Peer>, body: &[u8]) {
    let Some(media) = u32_at(body, 4) else { return };
    let Some((d, _)) = down_of(peer, media) else { return };
    let Some(ctx) = crate::cfg::get() else { return };
    let mut lost: Vec<u16> = Vec::new();
    let mut i = 8;
    while i + 4 <= body.len() {
        let pid = u16::from_be_bytes([body[i], body[i + 1]]);
        let blp = u16::from_be_bytes([body[i + 2], body[i + 3]]);
        lost.push(pid);
        for b in 0..16 {
            if blp & (1 << b) != 0 {
                lost.push(pid.wrapping_add(b + 1));
            }
        }
        i += 4;
    }
    lock(&peer.sub_transport().sim).nack(lost.len() as u32, now_ms());
    for out_seq in lost {
        let (src, phys, ts_off) = {
            let st = lock(&d.st);
            let src = if st.nack_closed { None } else { st.rw.nack(out_seq) };
            (src, st.src_phys.as_ref().and_then(|w| w.upgrade()), st.rw.ts_off)
        };
        let Some(src) = src else {
            ctx.drops.rtx_gate.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let Some(phys) = phys else { continue };
        let Some(pkt) = lock(&phys.cache).get(src) else {
            ctx.drops.rtx_miss.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        {
            let now = now_ms();
            let mut b = lock(&peer.nack_budget);
            if now.saturating_sub(b.0) >= NACK_WINDOW_MS {
                b.clone_from(&(now, 0));
            }
            if b.1 >= NACK_BUDGET {
                ctx.drops.rtx_budget.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            b.1 += 1;
        }
        rtx(&d, peer, &pkt, out_seq, ts_off);
    }
}

fn rtx(d: &Arc<Down>, peer: &Arc<Peer>, pkt: &[u8], out_seq: u16, ts_off: u32) {
    let (Some(rs), rpt) = (d.rtx_ssrc, d.rtx_pt.load(Ordering::Acquire)) else { return };
    if rpt == 0 || d.kind != MediaKind::Video {
        return;
    }
    let Some(r) = fc::parse(pkt) else { return };
    let seq = {
        let mut st = lock(&d.st);
        st.rtx_seq = st.rtx_seq.wrapping_add(1);
        st.rtx_seq
    };
    let mut payload = Vec::with_capacity(r.payload.len() + 2);
    payload.extend_from_slice(&out_seq.to_be_bytes());
    payload.extend_from_slice(r.payload);
    let t = peer.sub_transport();
    let table = crate::fwd::shell::ext_table(peer);
    let tb = crate::sub::ext::ExtTable { audio_level: None, ..table };
    let twcc_seq = if tb.twcc.is_some() { t.twcc_out.fetch_add(1, Ordering::AcqRel) } else { 0 };
    let mid = d.mid.to_string();
    let mut eb = Vec::new();
    crate::sub::ext::build(&tb, tb.form(), &crate::sub::ext::ExtValues { mid: Some(&mid), audio_level: None, t_us: common::clock::wall_us(), twcc_seq }, &mut eb);
    let mut out = Vec::new();
    fc::build(&Egress { byte1: (r.marker as u8) << 7, pt: rpt, seq, ts: r.ts.wrapping_add(ts_off), ssrc: rs, csrc: r.csrc, ext: &eb, payload: &payload, padding: &[] }, &mut out);
    if t.send_rtp(&out) && tb.twcc.is_some() {
        lock(&t.sim).sent_packet(twcc_seq, common::clock::wall_us(), out.len());
    }
}

fn on_twcc(t: &Arc<Transport>, body: &[u8]) {
    let Some(fci) = body.get(8..) else { return };
    let Some(acks) = rc::parse_twcc(fci) else { return };
    lock(&t.sim).feedback(&acks, now_ms());
}

fn on_pli(peer: &Arc<Peer>, body: &[u8], fir: bool) {
    let media = if fir { u32_at(body, 8) } else { u32_at(body, 4) };
    let Some(media) = media else { return };
    let Some((d, _)) = down_of(peer, media) else { return };
    if d.class == Class::Slot {
        crate::fwd::shell::key_for(&d);
        return;
    }
    let phys = lock(&d.st).src_phys.as_ref().and_then(|w| w.upgrade());
    match phys {
        Some(p) => pli(&p, false),
        None => crate::fwd::shell::key_for(&d),
    }
}

pub fn pli(phys: &Arc<PubTrack>, infra: bool) {
    let now = now_ms();
    let gap = if phys.layer.load(Ordering::Acquire) == 0 && phys.rid.is_some() { PLI_L_MS } else { PLI_H_MS };
    let last = phys.last_pli_ms.load(Ordering::Acquire);
    if !infra && now.saturating_sub(last) < gap {
        return;
    }
    phys.last_pli_ms.store(now, Ordering::Release);
    let Some(stream) = phys.stream.upgrade() else { return };
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(p) = ctx.links.peer(&stream.user_id) else { return };
    p.pub_t.send_rtcp(&rc::encode_pli(phys.ssrc));
}

fn lsr_dlsr(phys: &PubTrack, now: u64) -> (u32, u32) {
    match *lock(&phys.sr) {
        Some((sr, at)) => (((sr.ntp >> 16) & 0xffff_ffff) as u32, ((now.saturating_sub(at)) * 65536 / 1000) as u32),
        None => (0, 0),
    }
}

pub fn spawn_tick() {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(20));
        let mut n: u64 = 0;
        loop {
            iv.tick().await;
            n += 1;
            let Some(ctx) = crate::cfg::get() else { continue };
            let now = now_ms();
            let peers = ctx.links.all();
            for p in &peers {
                let pubs: Vec<Arc<crate::r#pub::PubStream>> = lock(&p.pubs).iter().filter(|s| !s.gone.load(Ordering::Acquire)).cloned().collect();
                for s in &pubs {
                    for ph in s.phys.load().iter() {
                        if s.kind == MediaKind::Video {
                            let due = lock(&ph.nack).due(now);
                            if !due.is_empty() {
                                p.pub_t.send_rtcp(&rc::encode_nack(ph.ssrc, &nack_pairs(&due)));
                            }
                        }
                        if n.is_multiple_of(50) && ph.packets.load(Ordering::Relaxed) > 0 {
                            let r = lock(&ph.stats).report();
                            let (lsr, dlsr) = lsr_dlsr(ph, now);
                            p.pub_t.send_rtcp(&rc::encode_rr(ph.ssrc, &r, lsr, dlsr));
                        }
                    }
                }
                if n.is_multiple_of(5) {
                    for t in p.transports() {
                        if t.twcc_id.load(Ordering::Acquire) == 0 {
                            continue;
                        }
                        let media = t.ingress.iter().next().map(|e| *e.key()).unwrap_or(0);
                        let fbs = lock(&t.twcc_in).build();
                        for fb in fbs {
                            t.send_rtcp(&rc::encode_twcc(&fb, media));
                        }
                    }
                }
                if n.is_multiple_of(50) {
                    let ssrcs: Vec<u32> = p.pub_t.ingress.iter().map(|e| *e.key()).collect();
                    if !ssrcs.is_empty() {
                        let bps = ctx.boot.policy.media.max_bitrate_bps as u64;
                        p.pub_t.send_rtcp(&rc::encode_remb(bps, &ssrcs));
                    }
                    layer_tick(ctx, p, now);
                }
            }
        }
    });
}

async fn probe_run(p: Arc<Peer>) {
    let mut iv = tokio::time::interval(Duration::from_millis(PROBE_CHUNK_MS));
    loop {
        iv.tick().await;
        let now = now_ms();
        let rate = {
            let g = lock(&p.sub_transport().sim);
            match g.probe.as_ref() {
                Some(pr) if pr.sending(now) => pr.rate,
                _ => return,
            }
        };
        let bytes = ((rate - L_BPS).max(0.0) * PROBE_CHUNK_MS as f64 / 1000.0 / 8.0) as usize;
        let sent = crate::fwd::shell::probe_send(&p, bytes);
        let mut g = lock(&p.sub_transport().sim);
        if let Some(pr) = g.probe.as_mut() {
            pr.seqs.extend(sent.iter().map(|(s, _)| *s));
        }
    }
}

fn layer_tick(ctx: &crate::cfg::Ctx, p: &Arc<Peer>, now: u64) {
    let downs: Vec<Arc<Down>> = lock(&p.downs).values().filter(|d| d.kind == MediaKind::Video && d.class == Class::Full).cloned().collect();
    let sims: Vec<(Arc<Down>, Arc<crate::r#pub::PubStream>)> = downs
        .into_iter()
        .filter_map(|d| {
            let l = d.loaded()?;
            let s = ctx.links.all().iter().find_map(|q| q.pub_by_id(&l.track_id))?;
            s.simulcast.then_some((d, s))
        })
        .collect();
    if sims.is_empty() {
        return;
    }
    let t = p.sub_transport();
    let auto = if ctx.auto() {
        let mut g = lock(&t.sim);
        let s = Signals {
            now,
            remb: g.remb,
            loss: g.loss,
            nack_per_s: g.nack_rate,
            drops: 0,
            est: if g.est_at > 0 { Some((g.gcc.est, g.est_at)) } else { None },
            overuse: g.gcc.usage == crate::sim::core::Usage::Over,
            tracks: sims.len(),
            v2: ctx.v2(),
        };
        let mut pol = g.policy;
        if s.v2 {
            let need = sims.len() as f64 * H_BPS * HEADROOM;
            if let Some(pr) = g.probe.as_ref().filter(|pr| pr.due(now)) {
                let proven = pr.proven();
                let measured = pr.measured();
                tracing::info!(kind = "ev", spec = "PL-KEY-MEDIA-AUTO-LAYER", user = %p.user_id, epoch = %ctx.boot.epoch, probe = "judge", measured = ?measured, target = pr.target, proven = ?proven, "v2 프로브 판정");
                if proven == Some(true) {
                    g.gcc.est = g.gcc.est.max(measured.unwrap_or(0.0));
                    g.est_at = now;
                } else {
                    pol.last_demote = now;
                }
                g.probe = None;
            }
            let clean = s.loss.map(|(l, _)| l <= crate::sim::core::LOSS_CLEAN).unwrap_or(true) && !s.overuse;
            let short = s.est.map(|(e, _)| e < need).unwrap_or(true);
            if g.probe.is_none() && pol.layer == Layer::Low && clean && short && now.saturating_sub(pol.last_demote) >= pol.backoff {
                let pr = Probe::new(now, need);
                tracing::info!(kind = "ev", spec = "PL-KEY-MEDIA-AUTO-LAYER", user = %p.user_id, epoch = %ctx.boot.epoch, probe = "start", rate = pr.rate, target = pr.target, "v2 프로브 시작");
                g.probe = Some(pr);
                let pp = p.clone();
                tokio::spawn(async move { probe_run(pp).await });
            }
        }
        if let Some((l, why)) = policy_tick(&mut pol, &s) {
            tracing::info!(kind = "ev", spec = "PL-KEY-MEDIA-AUTO-LAYER", user = %p.user_id, epoch = %ctx.boot.epoch, layer = l.name(), reason = why, est = ?s.est, remb = ?s.remb, loss = ?s.loss, "자동 레이어");
        }
        g.policy = pol;
        Some(pol.layer)
    } else {
        None
    };
    for (d, s) in sims {
        let (changed, target) = {
            let mut st = lock(&d.st);
            let expired = st.sel.expire(now);
            if expired {
                tracing::info!(kind = "diag", user = %p.user_id, track = %s.track_id, "[SIM:PENDING] expired");
            }
            let c = st.sel.aim(auto, now);
            (c, st.sel.target)
        };
        if changed && let Some(tg) = target {
            for ph in s.phys.load().iter() {
                if ph.layer.load(Ordering::Acquire) == tg {
                    pli(ph, true);
                }
            }
        }
    }
}
