// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FWD-STEPS · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:RULE-RTCP-NACK-DOWN · 정:IMPL-RTCP-TERMINATE · 정:IMPL-SIM-SWITCH · 정:IMPL-FLR-MEDIA-BIND · 정:CAND-SIM-PROBE-SHAPE · 정:CAND-RTCP-RTX-GATE-WINDOW · 정:RULE-CONC-PERF-AXES · model: claude-opus-5-5
//! egress — 다운링크 스트림마다 재기록(출처 키 · 새 출처는 video 키프레임부터) · 확장 다시 짓기(구독자 표 · 서버 twcc 스탬핑) · PT byte1 · SRTP 송신. RTX(게이트 → 캐시 → 예산) · PLI 전달 · SR 번역 · 서버가 짓는 패킷(예열 · 해제 무음 · 프로브).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::core::covers;
use super::hot::Downlink;
use super::rtp::{self, Rtp};
use crate::link::Transport;
use crate::ops::drops::{DROPS, bump};
use crate::r#pub::Uplink;

fn ext_for(dl: &Downlink, t: &Transport, audio_level: Option<u8>) -> (Vec<u8>, Option<u16>) {
    let table = dl.ext.load();
    let twcc = table.twcc.map(|_| t.next_twcc());
    let block = crate::sub::ext::build(&table, Some(&dl.mid), audio_level, crate::rtcp::wall_us() as u64, u64::from(twcc.unwrap_or(0)));
    (block, twcc.map(|x| x as u16))
}

fn send(dl: &Downlink, t: &Transport, pkt: &[u8], twcc: Option<u16>) -> bool {
    if let Some(s) = twcc {
        crate::rtcp::record_send(t, s, pkt.len());
    }
    let ok = t.send_rtp(pkt);
    if ok {
        dl.egress_pkts.fetch_add(1, Ordering::Relaxed);
        dl.egress_bytes.fetch_add(pkt.len() as u64, Ordering::Relaxed);
    }
    ok
}

pub fn forward(dl: &Arc<Downlink>, r: &Rtp<'_>, up: &Arc<Uplink>, keyframe: bool, audio_level: Option<u8>, now_ms: u64) {
    let Some(t) = dl.transport.upgrade() else { return };
    let key = u64::from(up.ssrc);
    let (seq, ts) = {
        let Ok(mut rw) = dl.rewrite.lock() else { return };
        let switching = rw.rw.source() != Some(key);
        if switching && dl.kind == oxsig::types::Kind::Video && !keyframe {
            bump(&DROPS.await_keyframe);
            drop(rw);
            crate::rtcp::request_keyframe(up, now_ms, false);
            return;
        }
        let cov = covers(dl.clock, r.payload());
        let m = rw.rw.map(key, r.seq, r.ts, now_ms, dl.clock, cov);
        if switching {
            rw.src = Some(Arc::downgrade(up));
            dl.rtx_gate.store(false, Ordering::Release);
            dl.awaiting_key.store(false, Ordering::Release);
        }
        (m.seq, m.ts)
    };
    let (block, twcc) = ext_for(dl, &t, audio_level);
    let pkt = rtp::rebuild(r, dl.ssrc, seq, ts, dl.pt.load(Ordering::Relaxed), &block);
    send(dl, &t, &pkt, twcc);
}

pub fn synthetic(dl: &Arc<Downlink>, key: u64, seq: u16, ts: u32, payload: &[u8], now_ms: u64) {
    let Some(t) = dl.transport.upgrade() else { return };
    let (oseq, ots) = {
        let Ok(mut rw) = dl.rewrite.lock() else { return };
        let cov = covers(dl.clock, payload);
        let m = rw.rw.map(key, seq, ts, now_ms, dl.clock, cov);
        if m.switched || rw.src.is_some() {
            rw.src = None;
        }
        (m.seq, m.ts)
    };
    let (block, twcc) = ext_for(dl, &t, None);
    let pkt = rtp::synth(dl.ssrc, dl.pt.load(Ordering::Relaxed), oseq, ots, false, &block, payload);
    send(dl, &t, &pkt, twcc);
}

pub fn retransmit(t: &Arc<Transport>, dl: &Arc<Downlink>, out_seq: u16, now_ms: u64) {
    if dl.rtx_gate.load(Ordering::Acquire) {
        bump(&DROPS.rtx_gate);
        return;
    }
    let (in_seq, src, ts_off) = {
        let Ok(rw) = dl.rewrite.lock() else { return };
        let Some(in_seq) = rw.rw.nack_to_source(out_seq) else {
            bump(&DROPS.rtx_gate);
            return;
        };
        (in_seq, rw.src.clone(), rw.rw.ts_off())
    };
    let Some(up) = src.and_then(|w| w.upgrade()) else {
        bump(&DROPS.rtx_gate);
        return;
    };
    let orig = up.cache.lock().ok().and_then(|c| c.get(in_seq).map(<[u8]>::to_vec));
    let Some(orig) = orig else {
        bump(&DROPS.rtx_cache_miss);
        return;
    };
    if !t.nack_budget.take(now_ms) {
        bump(&DROPS.rtx_budget);
        return;
    }
    let Some(r) = rtp::parse(&orig) else { return };
    let rtx_pt = dl.rtx_pt.load(Ordering::Relaxed);
    if rtx_pt == 0 || dl.rtx_ssrc == 0 {
        bump(&DROPS.rtx_cache_miss);
        return;
    }
    let (block, twcc) = ext_for(dl, t, None);
    let rseq = dl.rtx_seq.fetch_add(1, Ordering::Relaxed);
    let pkt = rtp::rtx_wrap(&r, dl.rtx_ssrc, rseq, r.ts.wrapping_add(ts_off), rtx_pt, out_seq, &block);
    if send(dl, t, &pkt, twcc) {
        dl.rtx_sent.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn forward_pli(dl: &Arc<Downlink>, now_ms: u64, infra: bool) {
    let src = dl.rewrite.lock().ok().and_then(|r| r.src.clone()).and_then(|w| w.upgrade());
    match src {
        Some(up) => {
            crate::rtcp::request_keyframe(&up, now_ms, infra);
        }
        None => bump(&DROPS.rtcp_unresolved),
    }
}

pub fn target_layer_pli(dl: &Arc<Downlink>, now_ms: u64) {
    let src = dl.rewrite.lock().ok().and_then(|r| r.src.clone()).and_then(|w| w.upgrade());
    let Some(stream) = src.and_then(|u| u.stream.upgrade()) else { return };
    let target = dl.want_layer.load(Ordering::Acquire).min(dl.auto_layer.load(Ordering::Acquire));
    dl.switch_at_ms.store(now_ms, Ordering::Release);
    if let Some(u) = stream.uplink_of_layer(Some(target)) {
        crate::rtcp::request_keyframe(&u, now_ms, true);
    }
}

pub fn relay_sr(up: &Arc<Uplink>, ntp: u64, rtp_ts: u32) {
    let Some(stream) = up.stream.upgrade() else { return };
    let mut targets: Vec<Arc<Downlink>> = stream.subs.load().iter().filter_map(|w| w.upgrade()).collect();
    if let Some(room) = stream.aff.pub_room.load_full() {
        targets.extend(room.slot(stream.kind).live());
    }
    for dl in targets {
        let same = dl.rewrite.lock().ok().and_then(|r| r.src.as_ref().and_then(|w| w.upgrade()).map(|u| Arc::ptr_eq(&u, up)).map(|x| (x, r.rw.ts_off())));
        let Some((true, ts_off)) = same else { continue };
        let Some(t) = dl.transport.upgrade() else { continue };
        let out = super::core::translate_sr(super::core::SrIn { ssrc: up.ssrc, ntp, rtp_ts }, dl.ssrc, ts_off);
        let pkts = dl.egress_pkts.load(Ordering::Relaxed) as u32;
        let bytes = dl.egress_bytes.load(Ordering::Relaxed) as u32;
        t.send_rtcp(&crate::rtcp::pkt::sr(out.ssrc, out.ntp, out.rtp_ts, pkts, bytes));
    }
}

pub fn send_probe(t: &Arc<Transport>, dl: &Arc<Downlink>) -> Option<u16> {
    let rtx_pt = dl.rtx_pt.load(Ordering::Relaxed);
    if rtx_pt == 0 || dl.rtx_ssrc == 0 {
        return None;
    }
    let (osn, ts) = {
        let rw = dl.rewrite.lock().ok()?;
        (rw.rw.last_seq()?, 0u32)
    };
    let table = dl.ext.load();
    table.twcc?;
    let twcc = t.next_twcc() as u16;
    let block = crate::sub::ext::build(&table, Some(&dl.mid), None, crate::rtcp::wall_us() as u64, u64::from(twcc));
    let rseq = dl.rtx_seq.fetch_add(1, Ordering::Relaxed);
    let mut payload = osn.to_be_bytes().to_vec();
    let mut pkt = rtp::synth(dl.rtx_ssrc, rtx_pt, rseq, ts, false, &block, &[]);
    pkt[0] |= 0x20;
    pkt.append(&mut payload);
    let pad = 224usize;
    pkt.extend(std::iter::repeat_n(0u8, pad - 1));
    pkt.push(pad as u8);
    crate::rtcp::record_send(t, twcc, pkt.len());
    t.send_rtp(&pkt);
    Some(twcc)
}
