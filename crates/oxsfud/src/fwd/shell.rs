// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-SIM-TICK · 정:IMPL-ROOM-STALL · 정:IMPL-SUB-READY-GATE · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-LINK-PAIR-LIVENESS · 정:CAND-RTCP-REMB-ESTIMATE · 정:RULE-GC-FOUR · 연:ROOM.S.sync_required · 연:ROOM.S.TStall · 정:CAND-RTCP-RR-PER-STREAM · model: claude-opus-5-5
//! 주기 고리 — 상향 NACK · 예열/무음 20ms · TWCC 피드백 100ms · RR · REMB · 레이어 판단 · 쌍별 생존 · 게이트 시한 1,000ms · 정체 감지 5,000ms · 방마다 발언권 타이머(가장 이른 만료 시각).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use rtcp::packet::Packet;

use super::{floor_out, link_suspect, pli, send_prime, RoomHot, SENDER_SSRC};
use crate::link::{lock, Transport};
use crate::ops::Sfu;
use crate::sim::core::Verdict;

pub const STALL_MS: u64 = 5_000;
pub const T_STALL_MS: u64 = 30_000;

type Boxed = Box<dyn Packet + Send + Sync>;

fn send(t: &Transport, pkts: Vec<Boxed>) {
    if pkts.is_empty() {
        return;
    }
    if let Ok(b) = rtcp::packet::marshal(&pkts) {
        t.send_rtcp(&b);
    }
}

pub fn spawn_loops(sfu: &Arc<Sfu>) {
    tokio::spawn(fast(sfu.clone()));
    tokio::spawn(twcc(sfu.clone()));
    tokio::spawn(second(sfu.clone()));
    tokio::spawn(stall(sfu.clone()));
    tokio::spawn(floors(sfu.clone()));
}

async fn fast(sfu: Arc<Sfu>) {
    let mut iv = tokio::time::interval(Duration::from_millis(20));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        iv.tick().await;
        let now = Sfu::now();
        for t in sfu.net.transports().into_iter().filter(|t| t.role.publishes() && t.dtls_up.load(Ordering::Relaxed)) {
            let ing = t.ingress.load();
            let mut out: Vec<Boxed> = Vec::new();
            let mut seen = Vec::new();
            for (ssrc, (_, phys, rtx)) in ing.by_ssrc.iter() {
                if *rtx || seen.contains(ssrc) {
                    continue;
                }
                seen.push(*ssrc);
                let due = lock(&phys.nack).due(now);
                if due.is_empty() {
                    continue;
                }
                let nacks = crate::rtcp::core::nack_pairs(&due).into_iter().map(|(pid, blp)| rtcp::transport_feedbacks::transport_layer_nack::NackPair { packet_id: pid, lost_packets: blp }).collect();
                out.push(Box::new(rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack { sender_ssrc: SENDER_SSRC, media_ssrc: phys.ssrc, nacks }));
                sfu.counters.add("publisher_nack_sent", due.len() as u64);
            }
            drop(ing);
            send(&t, out);
        }
        for t in sfu.net.transports().into_iter().filter(|t| t.role.subscribes() && t.dtls_up.load(Ordering::Relaxed)) {
            probe_tick(&t, now);
        }
        for h in sfu.hot.load().values() {
            let a = h.slot(oxsig::types::Kind::Audio);
            if a.priming.load(Ordering::Acquire) > 0 {
                a.priming.fetch_sub(1, Ordering::AcqRel);
                send_prime(&sfu, a, now);
            } else if a.silence.load(Ordering::Acquire) > 0 {
                a.silence.fetch_sub(1, Ordering::AcqRel);
                send_prime(&sfu, a, now);
            }
        }
    }
}

fn probe_tick(t: &Arc<Transport>, now: u64) {
    let quota = {
        let b = lock(&t.bwe);
        let Some(p) = b.probe.as_ref() else { return };
        if now.saturating_sub(p.started_ms) >= crate::sim::core::PROBE_RUN_MS {
            return;
        }
        let extra = p.rate_bps.saturating_sub(crate::sim::core::LAYER_L_BPS);
        (extra * crate::sim::core::PROBE_CHUNK_MS / 1000 / 8) as usize
    };
    let Some(d) = t.all_downs().into_iter().find(|d| d.rtx_ssrc.is_some() && Option::clone(&d.track.load()).map(|tr| tr.simulcast).unwrap_or(false)) else { return };
    let mut sent = 0usize;
    let mut n = 0u64;
    while sent < quota && n < 64 {
        let Some(len) = super::send_padding(t, &d) else { break };
        sent += len;
        n += 1;
    }
    if let Some(p) = lock(&t.bwe).probe.as_mut() {
        p.sent += n;
    }
}

async fn twcc(sfu: Arc<Sfu>) {
    let mut iv = tokio::time::interval(Duration::from_millis(100));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        iv.tick().await;
        for t in sfu.net.transports().into_iter().filter(|t| t.role.publishes() && t.dtls_up.load(Ordering::Relaxed)) {
            let ing = t.ingress.load();
            if ing.twcc_id == 0 {
                continue;
            }
            let media = ing.by_ssrc.keys().next().copied().unwrap_or(0);
            drop(ing);
            let fbs = {
                let mut rx = lock(&t.twcc_rx);
                if !rx.pending() {
                    continue;
                }
                rx.build()
            };
            let pkts: Vec<Boxed> = fbs.into_iter().map(|fb| Box::new(twcc_packet(&fb, media)) as Boxed).collect();
            send(&t, pkts);
        }
    }
}

pub fn twcc_packet(fb: &crate::rtcp::core::Feedback, media: u32) -> rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc {
    use crate::rtcp::core::Sym;
    use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, RecvDelta, RunLengthChunk, StatusChunkTypeTcc, SymbolTypeTcc, TransportLayerCc};
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < fb.symbols.len() {
        let s = fb.symbols[i];
        let mut n = 1;
        while i + n < fb.symbols.len() && fb.symbols[i + n] == s && n < 0x1fff {
            n += 1;
        }
        let sym = match s {
            Sym::NotReceived => SymbolTypeTcc::PacketNotReceived,
            Sym::Small => SymbolTypeTcc::PacketReceivedSmallDelta,
            Sym::Large => SymbolTypeTcc::PacketReceivedLargeDelta,
        };
        chunks.push(PacketStatusChunk::RunLengthChunk(RunLengthChunk { type_tcc: StatusChunkTypeTcc::RunLengthChunk, packet_status_symbol: sym, run_length: n as u16 }));
        i += n;
    }
    let mut deltas = Vec::new();
    let mut di = 0;
    for s in &fb.symbols {
        let sym = match s {
            Sym::NotReceived => continue,
            Sym::Small => SymbolTypeTcc::PacketReceivedSmallDelta,
            Sym::Large => SymbolTypeTcc::PacketReceivedLargeDelta,
        };
        let d = fb.deltas.get(di).copied().unwrap_or(0) as i64;
        di += 1;
        deltas.push(RecvDelta { type_tcc_packet: sym, delta: d * 250 });
    }
    TransportLayerCc { sender_ssrc: SENDER_SSRC, media_ssrc: media, base_sequence_number: fb.base_seq, packet_status_count: fb.status_count, reference_time: fb.reference_time, fb_pkt_count: fb.fb_count, packet_chunks: chunks, recv_deltas: deltas }
}

async fn second(sfu: Arc<Sfu>) {
    let mut iv = tokio::time::interval(Duration::from_millis(1000));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        iv.tick().await;
        let now = Sfu::now();
        let max_bps = sfu.boot.policy.media.max_bitrate_bps as f32;
        for t in sfu.net.transports() {
            if t.closed.load(Ordering::Relaxed) {
                continue;
            }
            if t.tick(now) {
                link_suspect(&sfu, &t);
            }
            for room in crate::sub::gate_expired(&t, now) {
                if crate::sub::gate_open(&t, &room) {
                    tracing::warn!(kind = "diag", spec = "IMPL-SUB-READY-GATE", user = %t.user, room = %room, "T-gate expired — gate opened without READY (no keyframe request) · READY 미수신 의심");
                }
            }
            if !t.dtls_up.load(Ordering::Relaxed) {
                continue;
            }
            if t.role.publishes() {
                let ing = t.ingress.load();
                let mut reports = Vec::new();
                let mut ssrcs = Vec::new();
                for (ssrc, (_, phys, rtx)) in ing.by_ssrc.iter() {
                    if *rtx || ssrcs.contains(ssrc) {
                        continue;
                    }
                    ssrcs.push(*ssrc);
                    let mut st = lock(&phys.stats);
                    if !st.started() {
                        continue;
                    }
                    let r = st.report();
                    let (lsr, dlsr) = if st.last_sr_at_ms > 0 { (st.last_sr_ntp_mid, ((now.saturating_sub(st.last_sr_at_ms)) * 65_536 / 1000) as u32) } else { (0, 0) };
                    reports.push(rtcp::reception_report::ReceptionReport { ssrc: *ssrc, fraction_lost: r.fraction, total_lost: (r.lost as u32) & 0x00ff_ffff, last_sequence_number: r.extended_max, jitter: r.jitter, last_sender_report: lsr, delay: dlsr });
                }
                drop(ing);
                for r in reports {
                    send(&t, vec![Box::new(rtcp::receiver_report::ReceiverReport { ssrc: SENDER_SSRC, reports: vec![r], profile_extensions: bytes::Bytes::new() })]);
                }
                if !ssrcs.is_empty() {
                    send(&t, vec![Box::new(rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate { sender_ssrc: SENDER_SSRC, bitrate: max_bps, ssrcs })]);
                }
            }
            if t.role.subscribes() && sfu.auto_on() {
                layer_tick(&sfu, &t, now);
            }
        }
    }
}

fn layer_tick(sfu: &Sfu, t: &Arc<Transport>, now: u64) {
    let downs: Vec<_> = t.all_downs().into_iter().filter(|d| Option::clone(&d.track.load()).map(|tr| tr.simulcast).unwrap_or(false)).collect();
    if downs.is_empty() {
        return;
    }
    let verdict = {
        let mut b = lock(&t.bwe);
        if let Some(p) = b.probe.clone()
            && now.saturating_sub(p.started_ms) >= crate::sim::core::PROBE_JUDGE_MS
        {
            let measured = p.measured_bps();
            match measured {
                Some(m) if m >= p.need_bps => {
                    b.aimd.reset_rate(m);
                    tracing::info!(kind = "ev", spec = "CAND-SIM-PROBE-SHAPE", user = %t.user, measured_bps = m, need_bps = p.need_bps, sent = p.sent, "probe proved capacity");
                }
                _ => {
                    b.policy.last_demote_ms = Some(now);
                    tracing::info!(kind = "ev", spec = "CAND-SIM-PROBE-SHAPE", user = %t.user, measured_bps = ?measured, need_bps = p.need_bps, sent = p.sent, "probe did not prove capacity — clean window restarts");
                }
            }
            b.probe = None;
        }
        let s = b.signals(now, sfu.v2(), downs.len() as u32);
        if b.probe.is_none()
            && let Some(rate) = b.policy.probe_due(now, &s)
        {
            let need = (crate::sim::core::Policy::demand_bps(s.tracks, crate::sim::core::Layer::High) as f64 * crate::sim::core::HEADROOM) as u64;
            b.probe = Some(crate::sim::Probe { started_ms: now, rate_bps: rate, need_bps: need, ..Default::default() });
            tracing::info!(kind = "ev", spec = "CAND-SIM-PROBE-SHAPE", user = %t.user, rate_bps = rate, need_bps = need, "probe started");
        }
        let v = b.policy.tick(now, &s);
        b.nacks = 0;
        b.drops = 0;
        v
    };
    if verdict == Verdict::Stay {
        return;
    }
    tracing::info!(kind = "ev", spec = "RULE-SIM-TICK", user = %t.user, verdict = ?verdict, "auto layer decision");
    for d in downs {
        let want = crate::ops::subs::target_layer(&d, sfu, t);
        if lock(&d.sel).retarget(want, now)
            && let Some(tr) = Option::clone(&d.track.load())
            && let Some(p) = want.spatial().and_then(|s| tr.phys_by_spatial(s))
        {
            pli(sfu, &p, now, false);
        }
    }
}

async fn stall(sfu: Arc<Sfu>) {
    let mut iv = tokio::time::interval(Duration::from_millis(STALL_MS));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        iv.tick().await;
        let now = Sfu::now();
        let mut w = sfu.world.lock().await;
        let mut hits: Vec<(String, String)> = Vec::new();
        for r in w.rooms.values() {
            let Some(hot) = sfu.room_hot(&r.id) else { continue };
            for m in r.members.iter().filter(|m| m.ready_at > 0) {
                let Some(p) = w.peers.get(&m.user) else { continue };
                let t = &p.subt;
                if t.peer_state() != crate::link::core::PeerState::Alive || !t.dtls_up.load(Ordering::Relaxed) {
                    continue;
                }
                let mut stalled = false;
                for d in t.all_downs().into_iter().filter(|d| d.room().as_deref() == Some(r.id.as_str())) {
                    let egress = d.egress.load(Ordering::Relaxed) + 1;
                    let mark = d.stall_mark.swap(egress, Ordering::AcqRel);
                    let moved = egress != mark;
                    if lock(&d.prefs).paused || !d.acked() || moved {
                        continue;
                    }
                    let window_start = now.saturating_sub(STALL_MS);
                    let eligible = if d.slot {
                        let s = hot.slot(d.kind);
                        let holder = Option::clone(&s.holder.load());
                        let bound = s.speaker.load().is_some();
                        let since = s.bound_at.load(Ordering::Relaxed).max(m.ready_at);
                        holder.as_deref() != Some(m.user.as_str()) && bound && since <= window_start && s.last_rtp_ms.load(Ordering::Relaxed) > window_start
                    } else {
                        let Some(tr) = Option::clone(&d.track.load()) else { continue };
                        let start = if tr.kind == oxsig::types::Kind::Video { m.ready_at.max(tr.first_rtp_ms.load(Ordering::Relaxed)) } else { m.ready_at };
                        let recent = tr.phys.load().iter().any(|ph| ph.last_rtp_ms.load(Ordering::Relaxed) > window_start);
                        let audio = tr.kind == oxsig::types::Kind::Audio;
                        (audio || tr.first_rtp_ms.load(Ordering::Relaxed) > 0) && start > 0 && start <= window_start && (recent || audio) && !crate::sub::gate_closed(t, &r.id) && !tr.muted.load(Ordering::Relaxed)
                    };
                    if eligible && mark != 0 {
                        stalled = true;
                    }
                }
                if stalled {
                    hits.push((r.id.clone(), m.user.clone()));
                }
            }
        }
        for (room, user) in hits {
            let Some(r) = w.rooms.get_mut(&room) else { continue };
            let last = r.stall_cool.get(&user).copied().unwrap_or(0);
            if last != 0 && now.saturating_sub(last) < T_STALL_MS {
                continue;
            }
            r.stall_cool.insert(user.clone(), now);
            let body = oxsig::RoomEventBody::SyncRequired { room_id: room.clone(), reason: "stall".into() };
            sfu.to_user(&user, oxsig::Op::RoomEvent, serde_json::to_value(body).unwrap_or_default(), Some(&room));
            tracing::info!(kind = "ev", spec = "ROOM.S.sync_required", room = %room, user = %user, "delivery stall — sync_required");
        }
    }
}

async fn floors(sfu: Arc<Sfu>) {
    let mut running: std::collections::HashMap<String, tokio::task::JoinHandle<()>> = std::collections::HashMap::new();
    let mut iv = tokio::time::interval(Duration::from_millis(250));
    loop {
        iv.tick().await;
        let hot = sfu.hot.load();
        running.retain(|id, h| {
            let keep = hot.contains_key(id);
            if !keep {
                h.abort();
            }
            keep
        });
        for (id, h) in hot.iter() {
            if !running.contains_key(id) {
                running.insert(id.clone(), tokio::spawn(floor_task(Arc::downgrade(&sfu), h.clone())));
            }
        }
    }
}

async fn floor_task(sfu: std::sync::Weak<Sfu>, hot: Arc<RoomHot>) {
    loop {
        let due = hot.floor.next_due();
        let wait = match due {
            Some(d) => Duration::from_millis(d.saturating_sub(Sfu::now())),
            None => Duration::from_secs(3600),
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = hot.floor.wake.notified() => continue,
        }
        let Some(s) = sfu.upgrade() else { return };
        let now = Sfu::now();
        for o in hot.floor.fire(now) {
            floor_out(&s, &hot, o);
        }
    }
}
