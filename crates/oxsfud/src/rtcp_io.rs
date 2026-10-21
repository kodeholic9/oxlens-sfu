// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-RTCP-TERMINATE 정:RULE-RTCP-NACK-UP 정:RULE-RTCP-NACK-DOWN 정:IMPL-FWD-SOURCE-SWITCH 연:RULE-LINK-SOURCE-SWITCH RFC 3550 RFC 4585 RFC 4588 · model: claude-opus-5-5
//! RTCP — 서버는 두 세션의 종단이다. 수신 RR · REMB · TWCC 는 자체 생성, SR 은 번역, 구독자 RR 은 소비한다.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use oxsig::wire::Kind;
use rtcp::packet::Packet;
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::receiver_report::ReceiverReport;
use rtcp::reception_report::ReceptionReport;
use rtcp::sender_report::SenderReport;
use rtcp::transport_feedbacks::transport_layer_cc::{
    PacketStatusChunk, RecvDelta, RunLengthChunk, StatusChunkTypeTcc, SymbolTypeTcc, TransportLayerCc,
};
use rtcp::transport_feedbacks::transport_layer_nack::{NackPair, TransportLayerNack};

use crate::media::{PubStream, PubTrack};
use crate::peer::Conn;
use crate::rtp::{self, Rewrite};
use crate::sfu::{Sfu, lock, mono_ms, now_ms};
use crate::table::Row;

pub const SERVER_SSRC: u32 = 1;

fn marshal(p: &dyn Packet) -> Vec<u8> {
    p.marshal().map(|b| b.to_vec()).unwrap_or_default()
}

pub fn pli(media_ssrc: u32) -> Vec<u8> {
    marshal(&PictureLossIndication { sender_ssrc: SERVER_SSRC, media_ssrc })
}

fn row_by_ssrc(conn: &Conn, ssrc: u32) -> Option<Arc<Row>> {
    lock(&conn.table).rows.iter().find(|r| r.ssrc == ssrc || r.rtx_ssrc == Some(ssrc)).cloned()
}

pub fn on_rtcp(sfu: &Arc<Sfu>, conn: &Arc<Conn>, plain: &[u8]) {
    let mut b = Bytes::copy_from_slice(plain);
    let pkts = match rtcp::packet::unmarshal(&mut b) {
        Ok(p) => p,
        Err(_) => {
            sfu.counters.drop("rtcp_parse");
            return;
        }
    };
    for p in pkts {
        let any = p.as_any();
        if let Some(sr) = any.downcast_ref::<SenderReport>() {
            on_sr(sfu, conn, sr);
        } else if let Some(rr) = any.downcast_ref::<ReceiverReport>() {
            for r in &rr.reports {
                if row_by_ssrc(conn, r.ssrc).is_some() {
                    lock(&conn.bwe).on_loss(r.fraction_lost, mono_ms());
                }
            }
        } else if let Some(n) = any.downcast_ref::<TransportLayerNack>() {
            on_nack(sfu, conn, n);
        } else if let Some(p) = any.downcast_ref::<PictureLossIndication>() {
            on_pli(sfu, conn, p.media_ssrc);
        } else if let Some(f) = any.downcast_ref::<FullIntraRequest>() {
            for e in &f.fir {
                on_pli(sfu, conn, e.ssrc);
            }
            on_pli(sfu, conn, f.media_ssrc);
        } else if let Some(r) = any.downcast_ref::<ReceiverEstimatedMaximumBitrate>() {
            lock(&conn.bwe).on_remb(r.bitrate as u64, mono_ms());
        } else if let Some(t) = any.downcast_ref::<TransportLayerCc>() {
            lock(&conn.bwe).on_twcc(t, now_ms());
        }
    }
}

fn on_sr(sfu: &Arc<Sfu>, conn: &Arc<Conn>, sr: &SenderReport) {
    let Some(track) = conn.pubs.load().get(&sr.ssrc).cloned() else { return };
    lock(&track.sr).replace((sr.ntp_time, sr.rtp_time));
    track.sr_at.store(mono_ms(), Ordering::Relaxed);
    let Some(stream) = track.stream.upgrade() else { return };
    let rows = source_rows(sfu, conn, &stream);
    for row in rows {
        let (started, src, ts_off, sent, octets) = {
            let rw = lock(&row.rw);
            (rw.started, rw.src, rw.ts_off, rw.sent, rw.octets)
        };
        if !started || src != sr.ssrc {
            continue;
        }
        let Some(sc) = row.conn.upgrade() else { continue };
        let out = SenderReport {
            ssrc: row.ssrc,
            ntp_time: sr.ntp_time,
            rtp_time: sr.rtp_time.wrapping_add(ts_off),
            packet_count: sent as u32,
            octet_count: octets as u32,
            reports: vec![],
            profile_extensions: Bytes::new(),
        };
        sc.send_rtcp(&marshal(&out));
    }
}

fn source_rows(sfu: &Sfu, conn: &Conn, stream: &PubStream) -> Vec<Arc<Row>> {
    if stream.is_half() {
        let Some(peer) = conn.peer() else { return vec![] };
        let Some(room) = peer.pub_room.load_full() else { return vec![] };
        let _ = sfu;
        return room.slot(stream.kind).rows.load().iter().filter_map(|w| w.upgrade()).collect();
    }
    stream.subs.load().iter().filter_map(|w| w.upgrade()).collect()
}

fn source_of(sfu: &Sfu, row: &Row) -> Option<(Arc<PubStream>, Arc<PubTrack>)> {
    let src = lock(&row.rw).src;
    let stream = match row.stream() {
        Some(s) => s,
        None => {
            let room = sfu.room(&row.room_id()?)?;
            room.slot(row.kind).bound.load_full()?
        }
    };
    let track = stream.tracks.load().iter().find(|t| t.ssrc == src).cloned()?;
    Some((stream, track))
}

fn on_pli(sfu: &Arc<Sfu>, conn: &Arc<Conn>, media_ssrc: u32) {
    let Some(row) = row_by_ssrc(conn, media_ssrc) else { return };
    if let Some((stream, track)) = source_of(sfu, &row) {
        crate::media::request_keyframe_track(sfu, &stream, &track, false);
    } else if let Some(s) = row.stream() {
        crate::media::request_keyframe(sfu, &s, None, false);
    }
}

fn on_nack(sfu: &Arc<Sfu>, conn: &Arc<Conn>, n: &TransportLayerNack) {
    let Some(row) = row_by_ssrc(conn, n.media_ssrc) else {
        sfu.counters.drop("nack_unknown_ssrc");
        return;
    };
    lock(&conn.bwe).on_nack(mono_ms());
    if row.kind == Kind::Audio || row.rtx_ssrc.is_none() {
        sfu.counters.drop("nack_audio");
        return;
    }
    let Some((_stream, track)) = source_of(sfu, &row) else {
        sfu.counters.drop("nack_no_source");
        return;
    };
    let rw = *lock(&row.rw);
    let now = mono_ms();
    for pair in &n.nacks {
        let mut seqs = vec![pair.packet_id];
        for i in 0..16u16 {
            if pair.lost_packets & (1 << i) != 0 {
                seqs.push(pair.packet_id.wrapping_add(i + 1));
            }
        }
        for out_seq in seqs {
            let Some(in_seq) = rw.reverse(out_seq) else {
                sfu.counters.drop("rtx_gate");
                continue;
            };
            let pkt = lock(&track.cache).get(in_seq).map(|p| p.to_vec());
            let Some(pkt) = pkt else {
                sfu.counters.drop("rtx_cache_miss");
                continue;
            };
            if !conn.rtx_allow(now) {
                sfu.counters.drop("rtx_budget");
                continue;
            }
            send_rtx(&row, conn, &pkt, out_seq, rw.ts_off);
        }
    }
}

fn send_rtx(row: &Row, conn: &Conn, orig: &[u8], out_seq: u16, ts_off: u32) {
    let Some(p) = rtp::Rtp::parse(orig) else { return };
    let rtx_pt = row.rtx_pt.load(Ordering::Relaxed);
    let Some(rtx_ssrc) = row.rtx_ssrc else { return };
    let seq = row.rtx_seq.fetch_add(1, Ordering::Relaxed) as u16;
    let mut body = out_seq.to_be_bytes().to_vec();
    body.extend_from_slice(p.payload());
    let mut exts = Vec::new();
    let ids = conn.sub_ext.load();
    if let Some(id) = ids.abs_send_time {
        exts.push((id, rtp::abs_send_time(now_ms() * 1000).to_vec()));
    }
    if let Some(id) = ids.twcc {
        let s = conn.twcc_out.fetch_add(1, Ordering::Relaxed) as u16;
        exts.push((id, s.to_be_bytes().to_vec()));
    }
    let w = Rewrite { pt: rtx_pt, seq, ts: p.ts.wrapping_add(ts_off), ssrc: rtx_ssrc, marker: p.marker, exts: &exts };
    let out = rtp::build_raw(&[], 0, &w, &body);
    conn.send_rtp(&out);
}

#[derive(Default)]
pub struct TwccIn {
    recs: BTreeMap<u64, u64>,
    last: Option<u64>,
    next_base: Option<u64>,
    fb_count: u8,
}

impl TwccIn {
    pub fn record(&mut self, seq: u16, arrival_us: u64) {
        let ext = match self.last {
            None => seq as u64 + 0x1_0000,
            Some(l) => {
                let diff = seq.wrapping_sub(l as u16) as i16 as i64;
                (l as i64 + diff).max(0) as u64
            }
        };
        if self.last.is_none_or(|l| ext > l) {
            self.last = Some(ext);
        }
        if self.next_base.is_some_and(|b| ext < b) {
            return;
        }
        self.recs.insert(ext, arrival_us);
        if self.recs.len() > 4096
            && let Some((&k, _)) = self.recs.iter().next()
        {
            self.recs.remove(&k);
        }
    }

    pub fn build(&mut self, media_ssrc: u32) -> Option<TransportLayerCc> {
        let (&first, &first_at) = self.recs.iter().next()?;
        let base = self.next_base.map(|b| b.max(first)).unwrap_or(first);
        let end = *self.recs.keys().next_back()?;
        let end = end.min(base + 2000);
        let reference = (first_at / 64_000) & 0x00ff_ffff;
        let mut prev = reference * 64_000;
        let mut symbols: Vec<SymbolTypeTcc> = Vec::new();
        let mut deltas: Vec<RecvDelta> = Vec::new();
        for s in base..=end {
            match self.recs.get(&s) {
                Some(&at) => {
                    let d = (at as i64 - prev as i64) / 250;
                    let small = (0..=255).contains(&d);
                    let d = d.clamp(i16::MIN as i64, i16::MAX as i64);
                    let ty = if small { SymbolTypeTcc::PacketReceivedSmallDelta } else { SymbolTypeTcc::PacketReceivedLargeDelta };
                    symbols.push(ty);
                    deltas.push(RecvDelta { type_tcc_packet: ty, delta: d * 250 });
                    prev = (prev as i64 + d * 250) as u64;
                }
                None => symbols.push(SymbolTypeTcc::PacketNotReceived),
            }
        }
        let mut chunks: Vec<PacketStatusChunk> = Vec::new();
        let mut i = 0;
        while i < symbols.len() {
            let sym = symbols[i];
            let mut j = i;
            while j < symbols.len() && symbols[j] == sym && j - i < 8191 {
                j += 1;
            }
            chunks.push(PacketStatusChunk::RunLengthChunk(RunLengthChunk {
                type_tcc: StatusChunkTypeTcc::RunLengthChunk,
                packet_status_symbol: sym,
                run_length: (j - i) as u16,
            }));
            i = j;
        }
        let fb = TransportLayerCc {
            sender_ssrc: SERVER_SSRC,
            media_ssrc,
            base_sequence_number: base as u16,
            packet_status_count: symbols.len() as u16,
            reference_time: reference as u32,
            fb_pkt_count: self.fb_count,
            packet_chunks: chunks,
            recv_deltas: deltas,
        };
        self.fb_count = self.fb_count.wrapping_add(1);
        self.recs.retain(|k, _| *k > end);
        self.next_base = Some(end + 1);
        Some(fb)
    }
}

pub fn twcc_tick(sfu: &Sfu) {
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let c = &p.pub_conn;
        let media = match lock(&p.streams).iter().find(|s| s.ext.twcc.is_some()) {
            Some(s) => s.tracks.load().first().map(|t| t.ssrc).unwrap_or(s.ssrc),
            None => continue,
        };
        let fb = lock(&c.twcc_in).build(media);
        if let Some(fb) = fb {
            c.send_rtcp(&marshal(&fb));
        }
    }
}

pub fn nack_tick(sfu: &Sfu) {
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let tracks: Vec<Arc<PubTrack>> = p.pub_conn.pubs.load().values().cloned().collect();
        for t in tracks {
            let due = lock(&t.nack).due(now);
            if due.is_empty() {
                continue;
            }
            let mut pairs: Vec<NackPair> = Vec::new();
            for s in due {
                if let Some(last) = pairs.last_mut() {
                    let d = s.wrapping_sub(last.packet_id);
                    if (1..=16).contains(&d) {
                        last.lost_packets |= 1 << (d - 1);
                        continue;
                    }
                }
                pairs.push(NackPair { packet_id: s, lost_packets: 0 });
            }
            let n = TransportLayerNack { sender_ssrc: SERVER_SSRC, media_ssrc: t.ssrc, nacks: pairs };
            p.pub_conn.send_rtcp(&marshal(&n));
        }
    }
}

pub fn report_tick(sfu: &Sfu) {
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let tracks: Vec<Arc<PubTrack>> = p.pub_conn.pubs.load().values().cloned().collect();
        if tracks.is_empty() {
            continue;
        }
        let mut reports = Vec::new();
        let mut ssrcs = Vec::new();
        for t in &tracks {
            let (frac, lost, max, jitter) = {
                let mut r = lock(&t.recv);
                if !r.started {
                    continue;
                }
                r.tick_rate(now);
                r.report()
            };
            let (lsr, dlsr) = match *lock(&t.sr) {
                Some((ntp, _)) => {
                    let at = t.sr_at.load(Ordering::Relaxed);
                    (((ntp >> 16) & 0xffff_ffff) as u32, ((now.saturating_sub(at)) * 65_536 / 1000) as u32)
                }
                None => (0, 0),
            };
            reports.push(ReceptionReport { ssrc: t.ssrc, fraction_lost: frac, total_lost: lost, last_sequence_number: max, jitter, last_sender_report: lsr, delay: dlsr });
            ssrcs.push(t.ssrc);
        }
        if reports.is_empty() {
            continue;
        }
        let mut out = marshal(&ReceiverReport { ssrc: SERVER_SSRC, reports, profile_extensions: Bytes::new() });
        let remb = ReceiverEstimatedMaximumBitrate { sender_ssrc: SERVER_SSRC, bitrate: sfu.policy.media.max_bitrate_bps as f32, ssrcs };
        out.extend(marshal(&remb));
        p.pub_conn.send_rtcp(&out);
    }
}
