// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-RTCP-TERMINATE 정:RULE-RTCP-NACK-UP 정:RULE-RTCP-NACK-DOWN 정:CAND-RTCP-SENDER-SSRC 정:CAND-RTCP-REMB-ESTIMATE 정:CAND-RTCP-NACK-BUDGET-UNIT 정:CAND-RTCP-RTX-GATE-WINDOW 정:CAND-RTCP-RR-PER-STREAM 정:IMPL-FWD-SOURCE-SWITCH 정:RULE-SIM-TICK 정:RULE-OPS-OBSERVE · model: claude-opus-5-5
//! RTCP 종단 — 들어오는 것(SR 번역 · RR/REMB/TWCC 소비 · NACK→RTX · PLI 전달) · 자체 생성(RR 1s · REMB 1s · TWCC 100ms · NACK 상향 200ms) · PLI 스로틀.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bytes::Bytes;
use oxsig::msg::Kind;
use rtcp::packet::Packet;
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::receiver_report::ReceiverReport;
use rtcp::reception_report::ReceptionReport;
use rtcp::sender_report::SenderReport;
use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, RecvDelta, RunLengthChunk, StatusChunkTypeTcc, SymbolTypeTcc, TransportLayerCc};
use rtcp::transport_feedbacks::transport_layer_nack::{TransportLayerNack, nack_pairs_from_sequence_numbers};

use super::core::{Feedback, Sym};
use crate::fwd::rtp::{self, Rewrite};
use crate::fwd::shell::{now_ms, now_us};
use crate::fwd::{Conn, Egress, Ingress};
use crate::ops::Sfu;
use crate::sim::core::{PLI_THROTTLE_H_MS, PLI_THROTTLE_L_MS};
use crate::trk::codec;

pub const SENDER_SSRC: u32 = 1;
pub const RR_MS: u64 = 1_000;
pub const REMB_MS: u64 = 1_000;
pub const TWCC_MS: u64 = 100;
pub const TICK_MS: u64 = 20;
pub const RTX_BUDGET: u32 = 200;
pub const RTX_WINDOW_MS: u64 = 3_000;

fn marshal(p: &(dyn Packet + Send + Sync)) -> Option<Bytes> {
    p.marshal().ok()
}

pub fn send(conn: &Conn, p: &(dyn Packet + Send + Sync)) -> bool {
    match marshal(p) {
        Some(b) => conn.link.send_rtcp(&b),
        None => false,
    }
}

pub fn pli(sfu: &Arc<Sfu>, ing: &Arc<Ingress>, now: u64, infra: bool) {
    if ing.stream.kind != Kind::Video {
        return;
    }
    let throttle = if ing.stream.simulcast && ing.layer == 0 { PLI_THROTTLE_L_MS } else { PLI_THROTTLE_H_MS };
    let last = ing.last_pli_ms.load(Ordering::Relaxed);
    if !infra && last != 0 && now.saturating_sub(last) < throttle {
        sfu.drops.pli_throttled.fetch_add(1, Ordering::Relaxed);
        return;
    }
    ing.last_pli_ms.store(now, Ordering::Relaxed);
    let Some(c) = ing.conn.upgrade() else { return };
    send(&c, &PictureLossIndication { sender_ssrc: SENDER_SSRC, media_ssrc: ing.ssrc });
    ing.pli_tx.fetch_add(1, Ordering::Relaxed);
}

pub fn on_rtcp(sfu: &Arc<Sfu>, conn: &Arc<Conn>, plain: &[u8]) {
    let mut b = Bytes::copy_from_slice(plain);
    let pkts = match rtcp::packet::unmarshal(&mut b) {
        Ok(p) => p,
        Err(_) => {
            sfu.drops.rtcp_unresolved.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let now = now_ms();
    for p in pkts {
        let a = p.as_any();
        if let Some(sr) = a.downcast_ref::<SenderReport>() {
            on_sr(sfu, conn, sr);
        } else if let Some(rr) = a.downcast_ref::<ReceiverReport>() {
            for r in &rr.reports {
                if let Some(e) = conn.egress_by_ssrc(r.ssrc)
                    && let Ok(mut bwe) = conn.bwe.lock()
                {
                    bwe.on_loss(e.mid, f64::from(r.fraction_lost) * 100.0 / 256.0, now);
                }
            }
        } else if let Some(n) = a.downcast_ref::<TransportLayerNack>() {
            conn.nack_rx.fetch_add(1, Ordering::Relaxed);
            let Some(e) = conn.egress_by_ssrc(n.media_ssrc) else {
                sfu.drops.rtcp_unresolved.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            for pair in &n.nacks {
                for seq in pair.packet_list() {
                    nack_down(sfu, conn, &e, seq, now);
                }
            }
        } else if let Some(x) = a.downcast_ref::<PictureLossIndication>() {
            conn.pli_rx.fetch_add(1, Ordering::Relaxed);
            key_request(sfu, conn, x.media_ssrc, now);
        } else if let Some(x) = a.downcast_ref::<FullIntraRequest>() {
            key_request(sfu, conn, x.media_ssrc, now);
        } else if let Some(r) = a.downcast_ref::<ReceiverEstimatedMaximumBitrate>() {
            if let Ok(mut bwe) = conn.bwe.lock() {
                bwe.on_remb(f64::from(r.bitrate), now);
            }
        } else if let Some(t) = a.downcast_ref::<TransportLayerCc>()
            && let Ok(mut bwe) = conn.bwe.lock()
        {
            bwe.on_twcc(t, now);
        }
    }
}

fn key_request(sfu: &Arc<Sfu>, conn: &Arc<Conn>, media: u32, now: u64) {
    let Some(e) = conn.egress_by_ssrc(media) else {
        sfu.drops.rtcp_unresolved.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let src = e.rw.lock().ok().map(|rw| rw.src);
    let Some(src) = src else { return };
    if let Some(i) = ingress_of_src(sfu, &e, src) {
        pli(sfu, &i, now, false);
    }
}

pub fn ingress_of_src(sfu: &Arc<Sfu>, e: &Egress, src: u64) -> Option<Arc<Ingress>> {
    let ssrc = (src & 0xFFFF_FFFF) as u32;
    let l = e.load.load_full()?;
    let user = match &l.user {
        Some(u) => u.clone(),
        None => sfu.plane.room(&l.room).and_then(|rm| rm.slot(e.kind).bound.load_full().map(|b| b.user.clone()))?,
    };
    let peer = sfu.peers.get(&user).map(|p| p.clone())?;
    peer.ingress_by_ssrc(ssrc)
}

fn on_sr(sfu: &Arc<Sfu>, conn: &Arc<Conn>, sr: &SenderReport) {
    let Some(ing) = conn.ingress_of(sr.ssrc) else { return };
    let s = super::super::fwd::core::Sr { ssrc: sr.ssrc, ntp: sr.ntp_time, rtp_ts: sr.rtp_time };
    if let Ok(mut st) = ing.st.lock() {
        st.sr = Some(s);
    }
    let mut targets: Vec<Arc<Egress>> = ing.subs.load().iter().filter_map(|w| w.upgrade()).collect();
    if ing.stream.half()
        && let Some(peer) = conn.peer.upgrade()
        && let Some(room) = peer.aff.pub_room()
        && let Some(rm) = sfu.plane.room(&room)
    {
        targets.extend(rm.slot(ing.stream.kind).subs.load().iter().filter_map(|w| w.upgrade()));
    }
    for e in targets {
        let (src, off) = match e.rw.lock() {
            Ok(rw) => (rw.src, rw.rewriter.ts_off),
            Err(_) => continue,
        };
        if (src & 0xFFFF_FFFF) as u32 != ing.ssrc {
            continue;
        }
        let t = super::super::fwd::core::translate_sr(s, e.ssrc, off);
        let Some(sub) = e.conn.upgrade() else { continue };
        let pkt = SenderReport {
            ssrc: t.ssrc,
            ntp_time: t.ntp,
            rtp_time: t.rtp_ts,
            packet_count: e.sent.load(Ordering::Relaxed) as u32,
            octet_count: 0,
            reports: Vec::new(),
            profile_extensions: Bytes::new(),
        };
        send(&sub, &pkt);
    }
}

fn nack_down(sfu: &Arc<Sfu>, conn: &Arc<Conn>, e: &Arc<Egress>, seq: u16, now: u64) {
    if e.kind != Kind::Video || e.rtx_ssrc.is_none() {
        sfu.drops.rtx_no_rtx.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if e.rtx_block.load(Ordering::Acquire) {
        sfu.drops.rtx_gate.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let (src, src_seq, ts_off) = match e.rw.lock() {
        Ok(rw) => (rw.src, rw.rewriter.nack_to_source(seq), rw.rewriter.ts_off),
        Err(_) => return,
    };
    let Some(src_seq) = src_seq else {
        sfu.drops.rtx_gate.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(ing) = ingress_of_src(sfu, e, src) else {
        sfu.drops.rtx_cache_miss.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let pkt = match ing.st.lock() {
        Ok(st) => st.cache.get(src_seq).map(<[u8]>::to_vec),
        Err(_) => None,
    };
    let Some(pkt) = pkt else {
        sfu.drops.rtx_cache_miss.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(peer) = conn.peer.upgrade() else { return };
    if !peer.rtx_budget(now) {
        sfu.drops.rtx_budget.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let Some(h) = rtp::parse(&pkt) else { return };
    let rtx_pt = e.rtx_pt.load(Ordering::Relaxed);
    let rseq = match e.rw.lock() {
        Ok(mut rw) => {
            rw.rtx_seq = rw.rtx_seq.wrapping_add(1);
            rw.rtx_seq
        }
        Err(_) => return,
    };
    let ext = crate::sub::table::SubExt::of(e.kind, &conn.ext.load());
    let tw = conn.twcc_out.fetch_add(1, Ordering::Relaxed);
    let mut ext_buf = Vec::new();
    if !ext.is_empty() {
        crate::sub::ext::build(&ext, Some(&e.mid_str), None, now_us(), tw, &mut ext_buf);
    }
    let mut out = Vec::with_capacity(pkt.len() + 32);
    rtp::rtx(&pkt, &h, seq, &Rewrite { pt: rtx_pt, seq: rseq, ts: h.ts.wrapping_add(ts_off), ssrc: e.rtx_ssrc.unwrap_or(0), ext: &ext_buf }, &mut out);
    if conn.link.send_rtp(&out) {
        sfu.drops.rtx_sent.fetch_add(1, Ordering::Relaxed);
        conn.rtx_tx.fetch_add(1, Ordering::Relaxed);
    }
}

fn twcc_packet(fb: &Feedback, media: u32) -> TransportLayerCc {
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < fb.symbols.len() {
        let s = fb.symbols[i];
        let mut j = i;
        while j < fb.symbols.len() && fb.symbols[j] == s && j - i < 8191 {
            j += 1;
        }
        let sym = match s {
            Sym::NotReceived => SymbolTypeTcc::PacketNotReceived,
            Sym::Small => SymbolTypeTcc::PacketReceivedSmallDelta,
            Sym::Large => SymbolTypeTcc::PacketReceivedLargeDelta,
        };
        chunks.push(PacketStatusChunk::RunLengthChunk(RunLengthChunk {
            type_tcc: StatusChunkTypeTcc::RunLengthChunk,
            packet_status_symbol: sym,
            run_length: (j - i) as u16,
        }));
        i = j;
    }
    let mut deltas = Vec::new();
    let mut d = fb.deltas_250us.iter();
    for s in &fb.symbols {
        if *s == Sym::NotReceived {
            continue;
        }
        let Some(v) = d.next() else { break };
        let t = if *s == Sym::Small { SymbolTypeTcc::PacketReceivedSmallDelta } else { SymbolTypeTcc::PacketReceivedLargeDelta };
        deltas.push(RecvDelta { type_tcc_packet: t, delta: i64::from(*v) * 250 });
    }
    TransportLayerCc {
        sender_ssrc: SENDER_SSRC,
        media_ssrc: media,
        base_sequence_number: fb.base_seq,
        packet_status_count: fb.status_count,
        reference_time: fb.reference_time,
        fb_pkt_count: fb.fb_count,
        packet_chunks: chunks,
        recv_deltas: deltas,
    }
}

fn rr_for(ing: &Ingress, now: u64) -> Option<ReceiverReport> {
    let mut st = ing.st.lock().ok()?;
    if !st.stats.started() {
        return None;
    }
    let r = st.stats.report();
    let (lsr, dlsr) = match st.sr {
        Some(sr) => (((sr.ntp >> 16) & 0xFFFF_FFFF) as u32, 0u32),
        None => (0, 0),
    };
    let _ = now;
    Some(ReceiverReport {
        ssrc: SENDER_SSRC,
        reports: vec![ReceptionReport {
            ssrc: ing.ssrc,
            fraction_lost: r.fraction,
            total_lost: (r.lost as u32) & 0x00FF_FFFF,
            last_sequence_number: r.extended_max,
            jitter: r.jitter,
            last_sender_report: lsr,
            delay: dlsr,
        }],
        profile_extensions: Bytes::new(),
    })
}

pub async fn conn_loop(sfu: Arc<Sfu>, conn: Arc<Conn>) {
    let mut tick = tokio::time::interval(Duration::from_millis(TICK_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_twcc = 0u64;
    let mut last_rr = 0u64;
    let mut last_sim = 0u64;
    loop {
        tokio::select! {
            _ = conn.link.cancel.cancelled() => break,
            _ = tick.tick() => {}
        }
        if !conn.link.srtp_ready.load(Ordering::Acquire) {
            continue;
        }
        let now = now_ms();
        let ings: Vec<Arc<Ingress>> = conn.ingress.load().values().cloned().collect();
        for i in &ings {
            let due = i.st.lock().map(|mut st| st.nack.due(now)).unwrap_or_default();
            if !due.is_empty() {
                send(&conn, &TransportLayerNack { sender_ssrc: SENDER_SSRC, media_ssrc: i.ssrc, nacks: nack_pairs_from_sequence_numbers(&due) });
                sfu.drops.nack_up_sent.fetch_add(due.len() as u64, Ordering::Relaxed);
                i.nack_tx.fetch_add(1, Ordering::Relaxed);
            }
        }
        if now.saturating_sub(last_twcc) >= TWCC_MS && conn.twcc_id.load(Ordering::Relaxed) != 0 {
            last_twcc = now;
            let fbs = conn.twcc_in.lock().map(|mut t| if t.pending() { t.build() } else { Vec::new() }).unwrap_or_default();
            let media = conn.media_ssrc.load(Ordering::Relaxed);
            for fb in &fbs {
                send(&conn, &twcc_packet(fb, media));
            }
        }
        if now.saturating_sub(last_rr) >= RR_MS {
            last_rr = now;
            for i in &ings {
                if let Some(rr) = rr_for(i, now) {
                    send(&conn, &rr);
                }
            }
            if !ings.is_empty() {
                let ssrcs: Vec<u32> = ings.iter().map(|i| i.ssrc).collect();
                let bps = sfu.cfg.policy.media.max_bitrate_bps as f32;
                send(&conn, &ReceiverEstimatedMaximumBitrate { sender_ssrc: SENDER_SSRC, bitrate: bps, ssrcs });
            }
        }
        if now.saturating_sub(last_sim) >= crate::sim::core::TICK_MS {
            last_sim = now;
            crate::sim::tick(&sfu, &conn, now);
        }
        crate::sim::probe(&sfu, &conn, now);
    }
}

pub fn keyframe_now(sfu: &Arc<Sfu>, ing: &Arc<Ingress>) {
    pli(sfu, ing, now_ms(), true);
}

pub fn is_key(codec_name: &str, pkt: &[u8]) -> bool {
    match rtp::parse(pkt) {
        Some(h) => codec::is_keyframe(codec_name, rtp::payload(pkt, &h)),
        None => false,
    }
}
