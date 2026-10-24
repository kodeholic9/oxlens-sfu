// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-FWD-SCOPE · 정:IMPL-FWD-ROOM · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:IMPL-FWD-SOURCE-SWITCH · 정:IMPL-PUB-ACCEPT · 정:IMPL-SIM-SWITCH · 정:IMPL-RTCP-TERMINATE · 정:RULE-RTCP-NACK-DOWN · 정:CAND-RTCP-RTX-GATE-WINDOW · 정:CAND-RTCP-SENDER-SSRC · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-SUB-READY-GATE · 연:RULE-LINK-SOURCE-SWITCH · 연:RULE-DOC-AUTHORITY-TWO · 연:RULE-FLR-SLOT-SPEAKER · model: claude-opus-5-5
//! 핫패스 — 발행 RTP 의 방은 매 패킷 `pub_room` · 반이중은 prefan(화자 == 발행자 ∧ 방 = pub_room)이 권위 · 전이중은 video 게이트 · 줄 확인 · 레이어 매칭 → 재기록기 · byte1 PT · 확장 다시 짓기 · SRTP. RTCP 는 종단(SR 번역 · PLI 모음 · NACK→RTX · REMB · TWCC).

pub mod shell;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use bytes::Bytes;
use oxsig::types::Kind;
use oxsig::{MediaCodecs, Op};

use self::core::{ext_block, parse_rtp, read_ext, rebuild, ExtValues, RtpView};
use crate::flr::{Floor, Outcome};
use crate::link::{lock, Down, Transport, Upcall};
use crate::ops::Sfu;
use crate::r#pub::{Phys, PubTrack};
use crate::sim::core::{Layer, Step};
use crate::trk::{Slot, OPUS_SILENCE};

pub mod core;

pub const SENDER_SSRC: u32 = 1;
pub const PLI_BURST_MS: [u64; 3] = [0, 500, 1500];

pub struct RoomHot {
    pub id: String,
    pub media: MediaCodecs,
    pub slots: [Arc<Slot>; 2],
    pub floor: Arc<Floor>,
}

impl RoomHot {
    pub fn new(id: &str, media: &MediaCodecs, policy: &common::policy::Policy) -> RoomHot {
        RoomHot {
            id: id.to_string(),
            media: media.clone(),
            slots: [Slot::new(id, Kind::Audio, media), Slot::new(id, Kind::Video, media)],
            floor: Arc::new(Floor::new(id, policy.floor.queue_max, policy.floor.t2_stop_talking_secs)),
        }
    }

    pub fn slot(&self, k: Kind) -> &Arc<Slot> {
        match k {
            Kind::Audio => &self.slots[0],
            Kind::Video => &self.slots[1],
        }
    }
}

pub struct Plane(pub Weak<Sfu>);

impl Upcall for Plane {
    fn media(&self, t: &Arc<Transport>, plain: &[u8], rtcp: bool) {
        let Some(sfu) = self.0.upgrade() else { return };
        if rtcp {
            on_rtcp(&sfu, t, plain);
        } else {
            on_rtp(&sfu, t, plain);
        }
    }

    fn dc(&self, t: &Arc<Transport>, data: &[u8]) {
        let Some(sfu) = self.0.upgrade() else { return };
        on_dc(&sfu, t, data);
    }

    fn dc_open(&self, t: &Arc<Transport>) {
        tracing::debug!(kind = "ev", spec = "RULE-LINK-TRANSPORT", user = %t.user, "dc open upcall");
    }

    fn suspect(&self, t: &Arc<Transport>) {
        let Some(sfu) = self.0.upgrade() else { return };
        link_suspect(&sfu, t);
    }
}

pub fn link_suspect(sfu: &Sfu, t: &Arc<Transport>) {
    let body = oxsig::LinkEventBody { type_: oxsig::LinkEventBodyType::Suspect, sfu_id: sfu.epoch.clone(), ufrag: t.cred().ufrag };
    sfu.to_user(&t.user, Op::LinkEvent, serde_json::to_value(body).unwrap_or_default(), None);
    tracing::info!(kind = "ev", spec = "LINK.S.suspect", user = %t.user, session = %t.session, role = t.role.as_str(), "path pair expired — suspect");
}

pub fn pli(sfu: &Sfu, phys: &Arc<Phys>, now: u64, infra: bool) {
    if !lock(&phys.pli).pass(now, phys.low(), infra) {
        sfu.counters.drop("pli_throttled");
        return;
    }
    let Some(t) = owner_transport(sfu, phys) else { return };
    let p = rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication { sender_ssrc: SENDER_SSRC, media_ssrc: phys.ssrc };
    if let Ok(b) = rtcp::packet::marshal(&[Box::new(p) as Box<dyn rtcp::packet::Packet + Send + Sync>]) {
        t.send_rtcp(&b);
    }
}

fn owner_transport(sfu: &Sfu, phys: &Arc<Phys>) -> Option<Arc<Transport>> {
    for p in sfu.peers.load().values() {
        if p.pubt.ingress.load().by_ssrc.get(&phys.ssrc).map(|(_, x, _)| Arc::ptr_eq(x, phys)).unwrap_or(false) {
            return Some(p.pubt.clone());
        }
    }
    None
}

fn learn(sfu: &Arc<Sfu>, t: &Arc<Transport>, b: &[u8], v: &RtpView) -> Option<(Arc<PubTrack>, Arc<Phys>, bool)> {
    let ing = t.ingress.load();
    if let Some(e) = ing.by_ssrc.get(&v.ssrc) {
        return Some(e.clone());
    }
    let mid = read_ext(b, v, ing.mid_id).map(|x| String::from_utf8_lossy(x).into_owned())?;
    let track = ing.by_mid.get(&mid)?.clone();
    if !track.simulcast {
        sfu.counters.drop("unknown_ssrc");
        return None;
    }
    let rid = read_ext(b, v, ing.rid_id).map(|x| String::from_utf8_lossy(x).into_owned());
    let rrid = read_ext(b, v, ing.rrid_id).map(|x| String::from_utf8_lossy(x).into_owned());
    drop(ing);
    if let Some(r) = rid {
        let spatial = crate::r#pub::core::rid_layer(&r)?;
        let phys = Phys::new(v.ssrc, None, Some(r.clone()), Some(spatial), track.clock());
        track.add_phys(phys.clone());
        let (tr, ph) = (track.clone(), phys.clone());
        t.set_ingress(|i| {
            i.by_ssrc.insert(ph.ssrc, (tr.clone(), ph.clone(), false));
        });
        tracing::info!(kind = "ev", spec = "IMPL-PUB-ACCEPT", user = %track.user, track = %track.track_id, rid = %r, ssrc = v.ssrc, "simulcast uplink stream attached by rid");
        if !track.listed.load(Ordering::Acquire) && !track.half.load(Ordering::Relaxed) {
            track.listed.store(true, Ordering::Release);
            let (sfu2, tr2) = (sfu.clone(), track.clone());
            tokio::spawn(async move {
                let mut w = sfu2.world.lock().await;
                let Some(peer) = w.peers.get(&tr2.user).cloned() else { return };
                if peer.track(&tr2.track_id).is_none() {
                    return;
                }
                if let Some(room) = peer.pub_room() {
                    crate::ops::pubs::list_tracks(&sfu2, &mut w, &room, std::slice::from_ref(&tr2));
                }
            });
        }
        return Some((track, phys, false));
    }
    if let Some(r) = rrid {
        let phys = track.phys.load().iter().find(|p| p.rid.as_deref() == Some(r.as_str())).cloned()?;
        phys.rtx_ssrc.store(v.ssrc, Ordering::Relaxed);
        let (tr, ph) = (track.clone(), phys.clone());
        t.set_ingress(|i| {
            i.by_ssrc.insert(v.ssrc, (tr.clone(), ph.clone(), true));
        });
        return Some((track, phys, true));
    }
    None
}

fn unwrap_rtx(b: &[u8], v: &RtpView, phys: &Phys, pt: u8) -> Option<Vec<u8>> {
    let pl = core::payload(b, v);
    if pl.len() < 2 {
        return None;
    }
    let osn = u16::from_be_bytes([pl[0], pl[1]]);
    let mut out = Vec::with_capacity(b.len());
    out.push(b[0] & !0x20);
    out.push((b[1] & 0x80) | (pt & 0x7f));
    out.extend_from_slice(&osn.to_be_bytes());
    out.extend_from_slice(&b[4..8]);
    out.extend_from_slice(&phys.ssrc.to_be_bytes());
    out.extend_from_slice(&b[12..v.payload_off]);
    out.extend_from_slice(&pl[2..]);
    Some(out)
}

pub fn on_rtp(sfu: &Arc<Sfu>, t: &Arc<Transport>, raw: &[u8]) {
    let now = Sfu::now();
    let Some(v0) = parse_rtp(raw) else {
        sfu.counters.drop("rtp_malformed");
        return;
    };
    let Some((track, phys, is_rtx)) = learn(sfu, t, raw, &v0) else {
        sfu.counters.drop("rtp_unknown_ssrc");
        return;
    };
    let owned;
    let (b, v) = if is_rtx {
        match unwrap_rtx(raw, &v0, &phys, track.pt).and_then(|p| parse_rtp(&p).map(|v| (p, v))) {
            Some((p, v)) => {
                owned = p;
                (&owned[..], v)
            }
            None => return,
        }
    } else {
        (raw, v0)
    };
    let ing = t.ingress.load();
    if !is_rtx {
        let valid = lock(&phys.stats).on_packet(v.seq, v.ts, now, phys.clock);
        let _ = valid;
        let prev = phys.last_seq.swap(v.seq as u32 | 0x1_0000, std::sync::atomic::Ordering::AcqRel);
        if prev & 0x1_0000 != 0 && v.seq.wrapping_sub(prev as u16) > 1 && v.seq.wrapping_sub(prev as u16) < 1000 {
            tracing::debug!(kind = "diag", user = %track.user, ssrc = phys.ssrc, from = prev & 0xffff, to = v.seq, "ingress seq gap");
        }
        lock(&phys.nack).on_packet(v.seq, now);
        phys.on_bytes(raw.len(), now);
        if ing.twcc_id != 0
            && let Some(x) = read_ext(b, &v, ing.twcc_id)
            && x.len() >= 2
        {
            lock(&t.twcc_rx).on_packet(u16::from_be_bytes([x[0], x[1]]), common::time::mono_us() as i64);
        }
        if v.pt != track.pt && !track.pt_mismatch.swap(true, Ordering::AcqRel) {
            tracing::warn!(kind = "diag", spec = "IMPL-PUB-ACCEPT", user = %track.user, track = %track.track_id, declared = track.pt, got = v.pt, "[PT:MISMATCH] first RTP PT differs from declared — surfaced, not fixed");
        }
    } else {
        lock(&phys.nack).on_packet(v.seq, now);
    }
    lock(&phys.cache).put(v.seq, b);
    if phys.first_rtp_ms.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        tracing::info!(kind = "ev", spec = "RULE-PUB-STATE", user = %track.user, track = %track.track_id, ssrc = phys.ssrc, "first RTP — Active");
    }
    let _ = track.first_rtp_ms.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Relaxed);
    track.state.store(2, Ordering::Relaxed);
    phys.last_rtp_ms.store(now, Ordering::Relaxed);
    let level = if track.kind == Kind::Audio && ing.level_id != 0 { read_ext(b, &v, ing.level_id).and_then(|x| x.first().copied()) } else { None };
    if let Some(l) = level
        && (l & 0x7f) < 50
    {
        track.last_level_ms.store(now, Ordering::Relaxed);
    }
    drop(ing);
    let kf = track.kind == Kind::Video && crate::r#pub::core::keyframe(track.codec_name(), core::payload(b, &v));
    if kf {
        phys.last_kf_ms.store(now, Ordering::Relaxed);
    }
    forward(sfu, t, &track, &phys, b, &v, kf, level, now);
}

#[allow(clippy::too_many_arguments)]
fn forward(sfu: &Arc<Sfu>, t: &Arc<Transport>, track: &Arc<PubTrack>, phys: &Arc<Phys>, b: &[u8], v: &RtpView, kf: bool, level: Option<u8>, now: u64) {
    let Some(room) = Option::clone(&t.pub_room.load()) else {
        sfu.counters.drop("no_pub_room");
        return;
    };
    if track.half.load(Ordering::Acquire) {
        let Some(hot) = sfu.room_hot(&room) else { return };
        let slot = hot.slot(track.kind);
        let bound = slot.speaker.load().as_ref().as_ref().map(|(_, tr)| Arc::ptr_eq(tr, track)).unwrap_or(false);
        if !bound {
            sfu.counters.drop("prefan_gate");
            return;
        }
        if let Some(o) = hot.floor.on_rtp(&track.user, now) {
            floor_out(sfu, &hot, o);
        }
        slot.last_rtp_ms.store(now, Ordering::Relaxed);
        slot.priming.store(0, Ordering::Release);
        slot.first_rtp.store(true, Ordering::Release);
        for d in slot.downs.load().iter() {
            if d.user == track.user {
                continue;
            }
            send_down(sfu, d, phys, phys.ssrc as u64, b, v, kf, level, now, true);
        }
        return;
    }
    if track.room() != room {
        sfu.counters.drop("not_pub_room_track");
        return;
    }
    for d in track.downs.load().iter() {
        let Some(st) = d.transport.upgrade() else { continue };
        if track.kind == Kind::Video && crate::sub::gate_closed(&st, &room) {
            d.gate_drops.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if track.simulcast {
            let layer = phys.spatial.map(Layer::from_spatial).unwrap_or(Layer::High);
            let step = lock(&d.sel).step(layer, kf, now);
            match step {
                Step::Drop => continue,
                Step::Switched => {
                    d.need_kf.store(false, Ordering::Release);
                    tracing::debug!(kind = "ev", spec = "IMPL-SIM-SWITCH", user = %d.user, track = %track.track_id, layer = layer.rid(), "layer switch confirmed on keyframe");
                }
                Step::Forward => {}
            }
        }
        send_down(sfu, d, phys, phys.ssrc as u64, b, v, kf, level, now, false);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn send_down(sfu: &Sfu, d: &Arc<Down>, phys: &Arc<Phys>, src: u64, b: &[u8], v: &RtpView, kf: bool, level: Option<u8>, now: u64, slot: bool) -> bool {
    let Some(st) = d.transport.upgrade() else { return false };
    if !st.dtls_up.load(Ordering::Acquire) {
        return false;
    }
    if !d.acked() {
        sfu.counters.drop("row_not_acked");
        return false;
    }
    if lock(&d.prefs).paused {
        return false;
    }
    if d.kind == Kind::Video && d.need_kf.load(Ordering::Acquire) {
        if !kf {
            pli(sfu, phys, now, false);
            sfu.counters.drop("await_keyframe");
            return false;
        }
        d.need_kf.store(false, Ordering::Release);
    }
    let (seq, ts, switched) = {
        let mut rw = lock(&d.rw);
        let sw = rw.will_switch(src);
        let (s, t) = rw.rewrite(src, v.seq, v.ts, now);
        (s, t, sw)
    };
    if switched || Option::clone(&d.cur.load()).map(|c| !Arc::ptr_eq(&c, phys)).unwrap_or(true) {
        d.cur.store(Arc::new(Some(phys.clone())));
        if slot {
            d.rtx_gate.store(false, Ordering::Release);
        }
    }
    let ext = d.ext.load();
    let twcc = ext.twcc.map(|_| (st.twcc_seq.fetch_add(1, Ordering::Relaxed) & 0xffff) as u16);
    let vals = ExtValues { mid: Some(d.mid.to_string()), audio_level: level, t_us: common::time::wall_us(), twcc };
    let block = ext_block(&ext, &vals);
    let out = rebuild(b, v, d.pt.load(Ordering::Relaxed), d.ssrc, seq, ts, &block);
    if let Some(n) = twcc {
        lock(&st.sent).record(n, common::time::mono_us(), out.len(), false);
    }
    if st.send_rtp(&out) {
        d.egress.fetch_add(1, Ordering::Relaxed);
        d.egress_bytes.fetch_add(out.len() as u64, Ordering::Relaxed);
        d.last_egress_ms.store(now, Ordering::Relaxed);
        return true;
    }
    false
}

pub const PADDING_BYTES: usize = 224;

pub fn send_padding(t: &Arc<Transport>, d: &Arc<Down>) -> Option<usize> {
    let rtx_ssrc = d.rtx_ssrc?;
    let pt = d.rtx_pt.load(Ordering::Relaxed);
    let ts = lock(&d.rw).last_out().map(|(_, ts)| ts).unwrap_or(0);
    let ext = d.ext.load();
    let twcc = ext.twcc.map(|_| (t.twcc_seq.fetch_add(1, Ordering::Relaxed) & 0xffff) as u16)?;
    let block = ext_block(&ext, &ExtValues { mid: Some(d.mid.to_string()), audio_level: None, t_us: common::time::wall_us(), twcc: Some(twcc) });
    let seq = d.rtx_seq.fetch_add(1, Ordering::Relaxed);
    let mut pkt = Vec::with_capacity(12 + block.len() + PADDING_BYTES);
    pkt.push(0x80 | 0x20 | if block.is_empty() { 0 } else { 0x10 });
    pkt.push(pt & 0x7f);
    pkt.extend_from_slice(&seq.to_be_bytes());
    pkt.extend_from_slice(&ts.to_be_bytes());
    pkt.extend_from_slice(&rtx_ssrc.to_be_bytes());
    pkt.extend_from_slice(&block);
    pkt.extend(std::iter::repeat_n(0u8, PADDING_BYTES - 1));
    pkt.push(PADDING_BYTES as u8);
    lock(&t.sent).record(twcc, common::time::mono_us(), pkt.len(), true);
    t.send_rtp(&pkt).then_some(pkt.len())
}

pub fn send_prime(sfu: &Sfu, slot: &Slot, now: u64) {
    let holder = Option::clone(&slot.holder.load()).or_else(|| Option::clone(&slot.last_holder.load()));
    let (seq, ts) = {
        let mut p = lock(&slot.prime);
        p.0 = p.0.wrapping_add(1);
        p.1 = p.1.wrapping_add(960);
        (p.0, p.1)
    };
    let mut pkt = vec![0x80, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    pkt[2..4].copy_from_slice(&seq.to_be_bytes());
    pkt[4..8].copy_from_slice(&ts.to_be_bytes());
    pkt.extend_from_slice(&OPUS_SILENCE);
    let Some(v) = parse_rtp(&pkt) else { return };
    let dummy = Phys::new(0, None, None, None, 48_000);
    let src = slot.prime_src();
    for d in slot.downs.load().iter() {
        if holder.as_deref() == Some(d.user.as_str()) {
            continue;
        }
        send_down(sfu, d, &dummy, src, &pkt, &v, false, None, now, true);
    }
}

pub fn floor_out(sfu: &Sfu, hot: &RoomHot, o: Outcome) {
    if o.labels.is_empty() && o.outs.is_empty() && o.fx.is_empty() {
        return;
    }
    let now = Sfu::now();
    let peers = sfu.peers.load();
    for out in &o.outs {
        let Some(frame) = crate::flr::encode(&hot.id, &out.m) else { continue };
        match peers.get(&out.to) {
            Some(p) => {
                p.pubt.dc.send(frame);
            }
            None => sfu.counters.drop("floor_no_peer"),
        }
        tracing::info!(kind = "ev", spec = crate::flr::name(out.m.t), room = %hot.id, user = %out.to, n = out.m.n, floor_sequence = out.m.q, "floor message");
    }
    if let Some(first) = o.labels.first() {
        tracing::debug!(kind = "ev", spec = *first, room = %hot.id, labels = ?o.labels, "floor step");
    }
    for fx in o.fx {
        match fx {
            crate::flr::core::Fx::MediaBind(u) => bind(sfu, hot, &u, now),
            crate::flr::core::Fx::MediaRelease(prev) => {
                for s in hot.slots.iter() {
                    s.release();
                    for d in s.downs.load().iter() {
                        d.rtx_gate.store(true, Ordering::Release);
                    }
                }
                tracing::info!(kind = "ev", spec = "IMPL-FLR-MEDIA-BIND", room = %hot.id, prev = ?prev, "slots released — silence frames");
            }
        }
    }
}

fn bind(sfu: &Sfu, hot: &RoomHot, user: &str, now: u64) {
    let Some(p) = sfu.peers.load().get(user).cloned() else { return };
    let in_pub = p.pub_room().as_deref() == Some(hot.id.as_str());
    let was_idle = hot.slots[0].holder.load().is_none() && hot.slots[1].holder.load().is_none();
    for s in hot.slots.iter() {
        let tr = p.tracks().into_iter().find(|t| t.kind == s.kind && t.half.load(Ordering::Relaxed)).filter(|_| in_pub);
        let tr = tr.filter(|t| s.kind == Kind::Audio || t.codec_name().eq_ignore_ascii_case(&s.codec));
        if s.kind == Kind::Video && tr.is_none() && p.tracks().iter().any(|t| t.kind == Kind::Video && t.half.load(Ordering::Relaxed)) {
            tracing::warn!(kind = "diag", spec = "IMPL-FLR-MEDIA-BIND", room = %hot.id, user = %user, "speaker video codec differs from room — not bound");
        }
        s.bind(user, tr.clone(), was_idle, now);
        for d in s.downs.load().iter() {
            d.rtx_gate.store(true, Ordering::Release);
            if d.kind == Kind::Video {
                d.need_kf.store(true, Ordering::Release);
            }
        }
        if s.kind == Kind::Video
            && let Some(t) = tr
            && let Some(ph) = t.main_phys()
        {
            for ms in PLI_BURST_MS {
                let (ph2, sw) = (ph.clone(), Arc::downgrade(&p));
                let sfu_w = sfu.self_weak();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    if sw.upgrade().is_none() {
                        return;
                    }
                    if let Some(s) = sfu_w.upgrade() {
                        pli(&s, &ph2, Sfu::now(), true);
                    }
                });
            }
        }
    }
    tracing::info!(kind = "ev", spec = "IMPL-FLR-MEDIA-BIND", room = %hot.id, user = %user, prime = was_idle, "speaker bound to slots");
}

pub fn on_dc(sfu: &Arc<Sfu>, t: &Arc<Transport>, data: &[u8]) {
    let Some(f) = oxsig::dc::DcFrame::decode(data) else {
        sfu.counters.drop("dc_truncated");
        return;
    };
    if f.svc != oxsig::dc::SVC_FLOOR {
        sfu.counters.drop("dc_unknown_svc");
        return;
    }
    let Some((room, msg, ackreq)) = crate::flr::decode(&f.payload) else {
        sfu.counters.drop("mbcp_malformed");
        return;
    };
    let now = Sfu::now();
    let user = t.user.clone();
    let kind = msg.t;
    let q = msg.q;
    match sfu.room_hot(&room) {
        Some(h) => {
            tracing::info!(kind = "ev", spec = crate::flr::name(kind), room = %room, user = %user, floor_sequence = q, "floor message in");
            let o = h.floor.rx(&user, msg, now);
            floor_out(sfu, &h, o);
        }
        None => {
            if kind == crate::flr::core::T::Request
                && let Some(frame) = crate::flr::encode(&room, &crate::flr::core::Msg::new(crate::flr::core::T::Deny, Some(crate::flr::core::NOT_IN_ROOM.to_string()), crate::flr::core::deny::OTHER_REASON as u32, 0))
            {
                t.dc.send(frame);
            }
        }
    }
    if ackreq {
        let of = match kind {
            crate::flr::core::T::Release => oxsig::mbcp::RELEASE,
            _ => oxsig::mbcp::REQUEST,
        };
        if let Some(frame) = crate::flr::ack_for(&room, of, None) {
            t.dc.send(frame);
        }
    }
}

pub fn on_rtcp(sfu: &Arc<Sfu>, t: &Arc<Transport>, plain: &[u8]) {
    let mut buf = Bytes::copy_from_slice(plain);
    let Ok(pkts) = rtcp::packet::unmarshal(&mut buf) else {
        sfu.counters.drop("rtcp_malformed");
        return;
    };
    let now = Sfu::now();
    for p in pkts {
        let any = p.as_any();
        if let Some(sr) = any.downcast_ref::<rtcp::sender_report::SenderReport>() {
            on_sr(sfu, t, sr, now);
        } else if let Some(rr) = any.downcast_ref::<rtcp::receiver_report::ReceiverReport>() {
            for r in &rr.reports {
                if t.down(r.ssrc).is_some() {
                    lock(&t.bwe).loss = Some((r.fraction_lost as f64 / 256.0, now));
                }
            }
        } else if let Some(pl) = any.downcast_ref::<rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication>() {
            keyframe_for(sfu, t, pl.media_ssrc, now);
        } else if let Some(fir) = any.downcast_ref::<rtcp::payload_feedbacks::full_intra_request::FullIntraRequest>() {
            for e in &fir.fir {
                keyframe_for(sfu, t, e.ssrc, now);
            }
            keyframe_for(sfu, t, fir.media_ssrc, now);
        } else if let Some(n) = any.downcast_ref::<rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack>() {
            on_nack(sfu, t, n, now);
        } else if let Some(r) = any.downcast_ref::<rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate>() {
            lock(&t.bwe).remb = Some((r.bitrate as u64, now));
        } else if let Some(cc) = any.downcast_ref::<rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc>() {
            on_twcc_feedback(t, cc, now);
        }
    }
}

fn keyframe_for(sfu: &Sfu, t: &Arc<Transport>, ssrc: u32, now: u64) {
    let Some(d) = t.down(ssrc) else { return };
    let src = Option::clone(&d.cur.load()).or_else(|| Option::clone(&d.track.load()).and_then(|tr| tr.main_phys()));
    if let Some(p) = src {
        pli(sfu, &p, now, false);
    }
}

fn on_sr(sfu: &Sfu, t: &Arc<Transport>, sr: &rtcp::sender_report::SenderReport, now: u64) {
    let ing = t.ingress.load();
    let Some((track, phys, _)) = ing.by_ssrc.get(&sr.ssrc).cloned() else { return };
    drop(ing);
    let input = core::Sr { ssrc: sr.ssrc, ntp: sr.ntp_time, rtp_ts: sr.rtp_time };
    let _ = lock(&phys.sr).replace((input, now));
    lock(&phys.stats).last_sr_ntp_mid = ((sr.ntp_time >> 16) & 0xffff_ffff) as u32;
    lock(&phys.stats).last_sr_at_ms = now;
    let mut downs: Vec<Arc<Down>> = track.downs.load().iter().cloned().collect();
    for h in sfu.hot.load().values() {
        let s = h.slot(track.kind);
        if s.speaker.load().as_ref().as_ref().map(|(_, tr)| Arc::ptr_eq(tr, &track)).unwrap_or(false) {
            downs.extend(s.downs.load().iter().filter(|d| d.user != track.user).cloned());
        }
    }
    for d in downs {
        if !Option::clone(&d.cur.load()).map(|c| Arc::ptr_eq(&c, &phys)).unwrap_or(false) {
            continue;
        }
        let Some(st) = d.transport.upgrade() else { continue };
        let ts_off = lock(&d.rw).ts_off();
        let out = core::translate_sr(input, d.ssrc, ts_off);
        let pkt = rtcp::sender_report::SenderReport {
            ssrc: out.ssrc,
            ntp_time: out.ntp,
            rtp_time: out.rtp_ts,
            packet_count: d.egress.load(Ordering::Relaxed) as u32,
            octet_count: d.egress_bytes.load(Ordering::Relaxed) as u32,
            reports: vec![],
            profile_extensions: Bytes::new(),
        };
        if let Ok(b) = rtcp::packet::marshal(&[Box::new(pkt) as Box<dyn rtcp::packet::Packet + Send + Sync>]) {
            st.send_rtcp(&b);
        }
    }
}

fn on_nack(sfu: &Sfu, t: &Arc<Transport>, n: &rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack, now: u64) {
    let Some(d) = t.down(n.media_ssrc) else { return };
    let (Some(rtx_ssrc), rtx_pt) = (d.rtx_ssrc, d.rtx_pt.load(Ordering::Relaxed)) else {
        sfu.counters.drop("nack_audio_unadopted");
        return;
    };
    let count: usize = n.nacks.iter().map(|p| crate::rtcp::core::nack_expand(p.packet_id, p.lost_packets).len()).sum();
    lock(&t.bwe).nacks += count as u64;
    if d.rtx_gate.load(Ordering::Acquire) {
        sfu.counters.add("rtx_gate", count as u64);
        return;
    }
    let Some(phys) = Option::clone(&d.cur.load()) else { return };
    for pair in &n.nacks {
        for out_seq in crate::rtcp::core::nack_expand(pair.packet_id, pair.lost_packets) {
            let Some(in_seq) = lock(&d.rw).nack_in(out_seq) else {
                sfu.counters.drop("rtx_gate");
                continue;
            };
            if !lock(&t.nack_budget).take(now) {
                sfu.counters.drop("rtx_budget");
                continue;
            }
            let Some(orig) = lock(&phys.cache).get(in_seq).map(<[u8]>::to_vec) else {
                sfu.counters.drop("rtx_cache_miss");
                continue;
            };
            let Some(v) = parse_rtp(&orig) else { continue };
            let ts = lock(&d.rw).sr_ts(v.ts);
            let ext = d.ext.load();
            let twcc = ext.twcc.map(|_| (t.twcc_seq.fetch_add(1, Ordering::Relaxed) & 0xffff) as u16);
            let block = ext_block(&ext, &ExtValues { mid: Some(d.mid.to_string()), audio_level: None, t_us: common::time::wall_us(), twcc });
            let base = rebuild(&orig, &v, d.pt.load(Ordering::Relaxed), d.ssrc, out_seq, ts, &block);
            let Some(bv) = parse_rtp(&base) else { continue };
            let rseq = d.rtx_seq.fetch_add(1, Ordering::Relaxed);
            let mut rtx = Vec::with_capacity(base.len() + 2);
            rtx.extend_from_slice(&base[..bv.payload_off]);
            rtx[0] &= !0x20;
            rtx[1] = (rtx[1] & 0x80) | (rtx_pt & 0x7f);
            rtx[2..4].copy_from_slice(&rseq.to_be_bytes());
            rtx[8..12].copy_from_slice(&rtx_ssrc.to_be_bytes());
            rtx.extend_from_slice(&out_seq.to_be_bytes());
            rtx.extend_from_slice(core::payload(&base, &bv));
            if let Some(nn) = twcc {
                lock(&t.sent).record(nn, common::time::mono_us(), rtx.len(), false);
            }
            if t.send_rtp(&rtx) {
                d.rtx_sent.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn on_twcc_feedback(t: &Arc<Transport>, cc: &rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc, now: u64) {
    use rtcp::transport_feedbacks::transport_layer_cc::{PacketStatusChunk, StatusChunkTypeTcc, SymbolTypeTcc};
    let mut statuses: Vec<bool> = Vec::new();
    for c in &cc.packet_chunks {
        match c {
            PacketStatusChunk::RunLengthChunk(r) => {
                let got = r.packet_status_symbol != SymbolTypeTcc::PacketNotReceived;
                statuses.extend(std::iter::repeat_n(got, r.run_length as usize));
            }
            PacketStatusChunk::StatusVectorChunk(v) => {
                let _ = StatusChunkTypeTcc::StatusVectorChunk;
                for s in &v.symbol_list {
                    statuses.push(*s != SymbolTypeTcc::PacketNotReceived);
                }
            }
        }
    }
    statuses.truncate(cc.packet_status_count as usize);
    let mut arrival = (cc.reference_time as i64) * 64_000;
    let mut deltas = cc.recv_deltas.iter();
    let mut pkts = Vec::new();
    let mut sent = lock(&t.sent);
    for (i, got) in statuses.iter().enumerate() {
        let seq = cc.base_sequence_number.wrapping_add(i as u16);
        let rec = sent.take(seq);
        if !got {
            continue;
        }
        let Some(dl) = deltas.next() else { break };
        arrival += dl.delta;
        if let Some((send_us, bytes, probe)) = rec {
            pkts.push((send_us, arrival, bytes, probe));
        }
    }
    drop(sent);
    lock(&t.bwe).on_feedback(&pkts, now);
}

pub fn rtp_view(b: &[u8]) -> Option<RtpView> {
    parse_rtp(b)
}
