// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-UP · 정:RULE-RTCP-NACK-DOWN · 정:CAND-RTCP-SENDER-SSRC · 정:CAND-RTCP-REMB-ESTIMATE · 정:CAND-RTCP-NACK-BUDGET-UNIT · 정:CAND-RTCP-RTX-GATE-WINDOW · 정:CAND-RTCP-RR-PER-STREAM · 정:IMPL-FWD-SOURCE-SWITCH · 연:RULE-LINK-SOURCE-SWITCH · model: claude-opus-5-5
//! RTCP — 두 세션의 종단. 발행자 쪽은 서버가 짓는다(RR 1s · TWCC 100ms · REMB 1s · NACK 상향) · 구독자 쪽은 서버가 소비한다(RR · REMB · TWCC → 자동 판단 · NACK → RTX · PLI → 지금 출처 발행자) · SR 은 번역만 한다 · `1pc` 는 평문에서 분해한다.

pub mod core;
pub mod pkt;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use oxsig::types::Kind;

use self::core::TwccRecv;
use self::pkt::Packet;
use crate::fwd::hot::Downlink;
use crate::link::Transport;
use crate::ops::drops::{DROPS, bump};

pub const RR_MS: u64 = 1_000;
pub const TWCC_MS: u64 = 100;
pub const REMB_MS: u64 = 1_000;
pub const NACK_POLL_MS: u64 = 20;
pub const RTX_BUDGET: u32 = 200;
pub const RTX_WINDOW_MS: u64 = 3_000;
pub const SEND_HISTORY: usize = 4_096;

#[derive(Debug, Default)]
pub struct Ingress {
    pub twcc: TwccRecv,
    pub media_ssrc: u32,
    pub remb_bps: Option<(u64, u64)>,
    pub loss: Option<(f64, u64)>,
    pub nacks: VecDeque<u64>,
    pub sent: VecDeque<(u16, i64, usize)>,
}

#[derive(Debug, Default)]
pub struct Budget {
    start_ms: AtomicU64,
    count: AtomicU32,
}

impl Budget {
    pub fn take(&self, now_ms: u64) -> bool {
        let start = self.start_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(start) >= RTX_WINDOW_MS {
            self.start_ms.store(now_ms, Ordering::Release);
            self.count.store(0, Ordering::Release);
        }
        self.count.fetch_add(1, Ordering::AcqRel) < RTX_BUDGET
    }
}

fn find_downlink(t: &Transport, ssrc: u32) -> Option<Arc<Downlink>> {
    t.downlinks.load().get(&ssrc).cloned()
}

pub fn on_rtcp(t: &Arc<Transport>, plain: &[u8], now_ms: u64) {
    for p in pkt::parse(plain) {
        match p {
            Packet::Sr { ssrc, ntp, rtp_ts, .. } => {
                let Some(u) = t.uplinks.load().get(&ssrc).cloned() else {
                    bump(&DROPS.rtcp_unresolved);
                    continue;
                };
                if let Ok(mut ing) = u.ingress.lock() {
                    ing.last_sr = Some((pkt::ntp_mid(ntp), common::time::mono_us()));
                    ing.sr = Some(crate::fwd::core::SrIn { ssrc, ntp, rtp_ts });
                }
                crate::fwd::egress::relay_sr(&u, ntp, rtp_ts);
            }
            Packet::Rr { blocks, .. } => {
                let mut resolved = false;
                for b in blocks {
                    if find_downlink(t, b.ssrc).is_some() {
                        resolved = true;
                        if let Ok(mut ing) = t.ingress.lock() {
                            ing.loss = Some((f64::from(b.fraction) * 100.0 / 256.0, now_ms));
                        }
                    }
                }
                if !resolved && t.role == crate::link::Role::Both && t.downlinks.load().is_empty() {
                    bump(&DROPS.rtcp_unresolved);
                }
            }
            Packet::Nack { media, seqs } => {
                let Some(d) = find_downlink(t, media) else {
                    bump(&DROPS.rtcp_unresolved);
                    continue;
                };
                if let Ok(mut ing) = t.ingress.lock() {
                    for _ in &seqs {
                        ing.nacks.push_back(now_ms);
                    }
                    while ing.nacks.front().is_some_and(|x| now_ms.saturating_sub(*x) > 1_000) {
                        ing.nacks.pop_front();
                    }
                }
                d.nacks_recv.fetch_add(seqs.len() as u64, Ordering::Relaxed);
                if d.kind == Kind::Audio {
                    for _ in &seqs {
                        bump(&DROPS.nack_audio);
                    }
                    continue;
                }
                for s in seqs {
                    crate::fwd::egress::retransmit(t, &d, s, now_ms);
                }
            }
            Packet::Pli { media } => match find_downlink(t, media) {
                Some(d) => {
                    d.plis_recv.fetch_add(1, Ordering::Relaxed);
                    crate::fwd::egress::forward_pli(&d, now_ms, false);
                }
                None => bump(&DROPS.rtcp_unresolved),
            },
            Packet::Fir { media } => {
                for m in media {
                    if let Some(d) = find_downlink(t, m) {
                        crate::fwd::egress::forward_pli(&d, now_ms, false);
                    }
                }
            }
            Packet::Remb { bitrate, .. } => {
                if let Ok(mut ing) = t.ingress.lock() {
                    ing.remb_bps = Some((bitrate, now_ms));
                }
            }
            Packet::Twcc { raw, .. } => crate::sim::on_feedback(t, &raw, now_ms),
            Packet::Ignored(_) => {}
        }
    }
}

pub fn record_send(t: &Transport, twcc_seq: u16, bytes: usize) {
    if let Ok(mut ing) = t.ingress.lock() {
        ing.sent.push_back((twcc_seq, wall_us(), bytes));
        while ing.sent.len() > SEND_HISTORY {
            ing.sent.pop_front();
        }
    }
}

pub fn wall_us() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

pub fn record_ingress_twcc(t: &Transport, ssrc: u32, seq: u16) {
    let at = (common::time::mono_us()) as i64;
    if let Ok(mut ing) = t.ingress.lock() {
        if ing.media_ssrc == 0 {
            ing.media_ssrc = ssrc;
        }
        ing.twcc.record(seq, at);
    }
}

pub fn send_rr_remb(t: &Transport, now_us: u64, max_bitrate_bps: u64) {
    let ups = t.uplinks.load();
    let mut ssrcs = Vec::new();
    for u in ups.values() {
        if u.rtx {
            continue;
        }
        ssrcs.push(u.ssrc);
        let block = {
            let Ok(mut ing) = u.ingress.lock() else { continue };
            if !ing.stats.started() {
                continue;
            }
            let r = ing.stats.report();
            let (lsr, dlsr) = match ing.last_sr {
                Some((mid, at_us)) => (mid, pkt::dlsr_us(now_us.saturating_sub(at_us))),
                None => (0, 0),
            };
            pkt::ReportBlock { ssrc: u.ssrc, fraction: r.fraction, lost: r.lost, ext_max: r.extended_max, jitter: r.jitter, lsr, dlsr }
        };
        t.send_rtcp(&pkt::rr(&block));
    }
    if !ssrcs.is_empty() {
        ssrcs.sort_unstable();
        t.send_rtcp(&pkt::remb(max_bitrate_bps, &ssrcs));
    }
}

pub fn send_twcc(t: &Transport) {
    if t.ext.load().twcc.is_none() {
        return;
    }
    let (fbs, media) = {
        let Ok(mut ing) = t.ingress.lock() else { return };
        if !ing.twcc.pending() {
            return;
        }
        (ing.twcc.build(), ing.media_ssrc)
    };
    for fb in fbs {
        t.send_rtcp(&core::encode_twcc(pkt::SENDER_SSRC, media, &fb));
    }
}

pub fn send_nacks(t: &Transport, now_ms: u64) {
    for u in t.uplinks.load().values() {
        if u.rtx || u.kind != Kind::Video {
            continue;
        }
        let due = u.nack.lock().map(|mut n| n.due(now_ms)).unwrap_or_default();
        if !due.is_empty() {
            u.nacks_sent.fetch_add(due.len() as u64, Ordering::Relaxed);
            t.send_rtcp(&pkt::nack(u.ssrc, &due));
        }
    }
}

pub fn request_keyframe(u: &crate::r#pub::Uplink, now_ms: u64, infra: bool) -> bool {
    let throttle = if u.layer == Some(0) { crate::sim::core::PLI_THROTTLE_L_MS } else { crate::sim::core::PLI_THROTTLE_H_MS };
    let last = u.last_pli_ms.load(Ordering::Acquire);
    if !infra && last != 0 && now_ms.saturating_sub(last) < throttle {
        bump(&DROPS.pli_throttled);
        return false;
    }
    u.last_pli_ms.store(now_ms.max(1), Ordering::Release);
    u.plis_sent.fetch_add(1, Ordering::Relaxed);
    let Some(s) = u.stream.upgrade() else { return false };
    let Some(t) = s.transport.upgrade() else { return false };
    t.send_rtcp(&pkt::pli(u.ssrc))
}

pub fn keyframe_burst(s: &Arc<crate::r#pub::Stream>, delays_ms: &[u64]) {
    for d in delays_ms {
        let s = s.clone();
        let d = *d;
        tokio::spawn(async move {
            if d > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(d)).await;
            }
            for u in s.uplinks.load().iter().filter(|u| !u.rtx) {
                request_keyframe(u, common::time::mono_ms(), true);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_per_window() {
        let b = Budget::default();
        for _ in 0..RTX_BUDGET {
            assert!(b.take(10));
        }
        assert!(!b.take(10));
        assert!(b.take(3_100));
    }
}
