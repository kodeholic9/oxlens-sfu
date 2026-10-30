// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FWD-SCOPE · 정:RULE-FWD-INGRESS-STATS · 정:IMPL-FWD-ROOM · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-CONC-HOT-PATH · 정:RULE-CONC-SIX · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-SIM-SWITCH · 연:RULE-AFF-SWITCH-TRACKS · model: claude-opus-5-5
//! 전달 — 패킷 길 입구 `on_rtp`(복호한 평문 RTP 하나) · 새 출처를 처음 배우는 `learn`. 방은 매 패킷 발행자의 `pub_room`(RCU) · 분기는 발행 트랙의 `duplex` 하나 · 반이중은 prefan(슬롯 결합 화자 == 발행자)이 강제 권위 · 화자 본인은 뺀다.

pub mod core;
pub mod egress;
pub mod hot;
pub mod rtp;
pub mod rx;
pub mod work;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use oxsig::types::{Duplex, Kind};

use self::hot::{FloorIn, LAYER_NONE};
use crate::link::Transport;
use crate::ops::drops::{DROPS, bump};
use crate::r#pub::{Stream, Uplink};

pub const FLOOR_NOTIFY_MS: u64 = 500;
pub const VOICE_LEVEL: u8 = 50;

pub fn on_rtp(t: &Arc<Transport>, plain: &Bytes, now_ms: u64, at_us: u64) {
    let Some(r) = rtp::parse(plain) else {
        bump(&DROPS.rtp_malformed);
        return;
    };
    let ext = t.ext.load();
    if let Some(v) = r.ext_value(ext.twcc)
        && v.len() >= 2
    {
        crate::rtcp::record_ingress_twcc(t, r.ssrc, u16::from_be_bytes([v[0], v[1]]));
    }
    let found = t.uplinks.load().get(&r.ssrc).cloned();
    let up = match found {
        Some(u) => u,
        None => match learn(t, &r, now_ms) {
            Some(u) => u,
            None => return,
        },
    };
    let Some(stream) = up.stream.upgrade() else {
        bump(&DROPS.rtp_unknown_ssrc);
        return;
    };
    if up.rtx {
        let Some(base) = stream.uplink_of_layer(up.layer) else { return };
        let Some(orig) = rtp::rtx_unwrap(&r, base.ssrc, stream.pt) else { return };
        let orig = Bytes::from(orig);
        let Some(r2) = rtp::parse(&orig) else { return };
        media(t, &base, &stream, &r2, &orig, now_ms, at_us, true);
        return;
    }
    media(t, &up, &stream, &r, plain, now_ms, at_us, false);
}

#[allow(clippy::too_many_arguments)]
fn media(t: &Arc<Transport>, up: &Arc<Uplink>, stream: &Arc<Stream>, r: &rtp::Rtp<'_>, plain: &Bytes, now_ms: u64, at_us: u64, recovered: bool) {
    if let Ok(mut ing) = up.ingress.lock() {
        if !recovered {
            ing.stats.update(r.seq, r.ts, now_ms);
        }
        ing.packets += 1;
        ing.bytes += plain.len() as u64;
    }
    if up.kind == Kind::Video
        && let Ok(mut n) = up.nack.lock()
    {
        n.on_packet(r.seq, now_ms);
    }
    if let Ok(mut c) = up.cache.lock() {
        c.put(r.seq, plain);
    }
    if up.first_rtp_ms.compare_exchange(0, now_ms.max(1), Ordering::AcqRel, Ordering::Acquire).is_ok() {
        crate::ops::first_rtp(stream, up);
    }
    if !up.pt_checked.swap(true, Ordering::AcqRel) && r.pt != stream.pt {
        tracing::warn!(track = &*stream.track_id, ssrc = up.ssrc, declared = stream.pt, got = r.pt, "[PT:MISMATCH]");
    }
    let ext = t.ext.load();
    let level = r.ext_value(ext.audio_level).and_then(|v| v.first().copied());
    if let Some(l) = level
        && (l & 0x7f) < VOICE_LEVEL
    {
        stream.last_voice_ms.store(now_ms, Ordering::Relaxed);
    }
    let Some(room) = stream.aff.pub_room.load_full() else {
        bump(&DROPS.no_pub_room);
        return;
    };
    let keyframe = up.kind == Kind::Video && crate::trk::is_keyframe(&stream.codec, r.payload());
    match stream.duplex() {
        Duplex::Half => {
            let last = stream.aff.rtp_notify_ms.load(Ordering::Acquire);
            if last == 0 || now_ms.saturating_sub(last) >= FLOOR_NOTIFY_MS {
                stream.aff.rtp_notify_ms.store(now_ms.max(1), Ordering::Release);
                let _ = room.floor_tx.send(FloorIn::Rtp(stream.user.clone()));
            }
            let slot = room.slot(up.kind);
            let bound = slot.bound.load();
            let mine = bound.as_ref().is_some_and(|b| b.user == stream.user);
            if !mine {
                bump(&DROPS.prefan_gate);
                return;
            }
            slot.speaker_last_ms.store(now_ms, Ordering::Release);
            let _ = slot.speaker_first_ms.compare_exchange(0, now_ms.max(1), Ordering::AcqRel, Ordering::Acquire);
            slot.priming.store(false, Ordering::Release);
            let mut fan = work::Fanout::new(r, plain, up, keyframe, level, now_ms, at_us);
            for w in slot.downlinks.load().iter() {
                let Some(dl) = w.upgrade() else { continue };
                if dl.user == stream.user {
                    continue;
                }
                if !dl.sendable.load(Ordering::Acquire) {
                    bump(&DROPS.row_gate);
                    continue;
                }
                fan.add(&dl);
            }
            fan.flush();
        }
        Duplex::Full => {
            let mut fan = work::Fanout::new(r, plain, up, keyframe, level, now_ms, at_us);
            for w in stream.subs.load().iter() {
                let Some(dl) = w.upgrade() else { continue };
                let carry = dl.carry.load();
                let Some(c) = carry.as_ref() else { continue };
                if *c.track_id != *stream.track_id {
                    continue;
                }
                if dl.kind == Kind::Video && !c.gate.load(Ordering::Acquire) {
                    bump(&DROPS.ready_gate);
                    continue;
                }
                if !dl.sendable.load(Ordering::Acquire) {
                    bump(&DROPS.row_gate);
                    continue;
                }
                if stream.simulcast && !layer_ok(&dl, up, keyframe, now_ms) {
                    continue;
                }
                fan.add(&dl);
            }
            fan.flush();
        }
    }
}

fn layer_ok(dl: &hot::Downlink, up: &Uplink, keyframe: bool, now_ms: u64) -> bool {
    if dl.paused.load(Ordering::Acquire) {
        return false;
    }
    let l = up.layer.unwrap_or(0);
    let target = dl.want_layer.load(Ordering::Acquire).min(dl.auto_layer.load(Ordering::Acquire));
    let cur = dl.cur_layer.load(Ordering::Acquire);
    if l == cur && (target == cur || !keyframe_of_target_seen(dl, now_ms)) {
        return true;
    }
    if l == target && keyframe {
        dl.cur_layer.store(l, Ordering::Release);
        return true;
    }
    if cur == LAYER_NONE && keyframe && l < target {
        dl.cur_layer.store(l, Ordering::Release);
        return true;
    }
    false
}

fn keyframe_of_target_seen(dl: &hot::Downlink, now_ms: u64) -> bool {
    let at = dl.switch_at_ms.load(Ordering::Acquire);
    at != 0 && now_ms.saturating_sub(at) > crate::sim::core::PENDING_EXPIRE_MS && {
        let cur = dl.cur_layer.load(Ordering::Acquire);
        dl.auto_layer.store(cur, Ordering::Release);
        dl.switch_at_ms.store(0, Ordering::Release);
        false
    }
}

pub fn learn(t: &Arc<Transport>, r: &rtp::Rtp<'_>, now_ms: u64) -> Option<Arc<Uplink>> {
    let ext = t.ext.load();
    let pending = t.pending_sim.load();
    if pending.is_empty() {
        bump(&DROPS.rtp_unknown_ssrc);
        return None;
    }
    let mid = r.ext_value(ext.mid);
    let stream = match mid {
        Some(m) => pending.iter().find(|s| s.mid.as_bytes() == m).cloned(),
        None => None,
    };
    let Some(stream) = stream else {
        bump(&DROPS.rtp_unknown_ssrc);
        return None;
    };
    let (rtx, rid) = match (r.ext_value(ext.rid), r.ext_value(ext.repaired_rid)) {
        (Some(v), _) => (false, v),
        (None, Some(v)) => (true, v),
        _ => {
            bump(&DROPS.rid_unknown);
            return None;
        }
    };
    let Some(layer) = crate::r#pub::rid_layer(rid) else {
        bump(&DROPS.rid_unknown);
        return None;
    };
    if rtx && stream.uplink_of_layer(Some(layer)).is_none() {
        return None;
    }
    let up = Arc::new(Uplink::new(r.ssrc, rtx, Some(layer), stream.kind, &stream));
    stream.add_uplink(up.clone());
    t.uplinks.rcu(|m| {
        let mut n = (**m).clone();
        n.insert(r.ssrc, up.clone());
        n
    });
    tracing::info!(track = &*stream.track_id, ssrc = r.ssrc, layer, rtx, "업링크 스트림 학습(rid)");
    let _ = now_ms;
    Some(up)
}
