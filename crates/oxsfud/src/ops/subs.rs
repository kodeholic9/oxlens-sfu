// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-READY-GATE · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-FLR-FIRST-NOTICE · 정:CAND-SUB-REASSIGN-TICK · 연:RULE-SUB-TABLE-PROC · 연:RULE-SUB-READY-PROC · 연:RULE-SUB-READY-IDEMPOTENT · 연:RULE-SUB-LAYER-PROC · 연:RULE-SUB-LAYER-PAUSED-AXIS · 연:LINK.S.ack · 연:LINK.S.ack_old · 연:LINK.S.reissue · 연:LINK.S.reassign · 연:SUB.S.ready · model: claude-opus-5-5
//! 구독 배관(cold path) — 받는 트랙 산출(슬롯 포함 · 쉬는 트랙 · 내 트랙 제외) → 배정표 → 다운링크 줄 → 출처 목록 결합 → 표가 바뀌면 `MLINE_EVENT`. READY(게이트 해제 · 방 단위 키프레임 1회 · 처음 알리기 · 확인 차례) · SUBSCRIBE_LAYER.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::bus::envelope::Reply;
use oxsig::types::Kind;
use oxsig::{parse, Code, MlineEventBody, Op, ReadyReq, SubscribeLayerReq};

use super::call::Who;
use super::{Sfu, World};
use crate::aff::Peer;
use crate::fwd::RoomHot;
use crate::link::table::{Placed, RowClass, Tuple};
use crate::link::{lock, Down, Transport};
use crate::r#pub::PubTrack;
use crate::room::Room;
use crate::sim::core::Layer;
use crate::trk::Slot;

#[derive(Clone)]
pub enum Carrier {
    Track(Arc<PubTrack>),
    Slot(Arc<Slot>),
}

#[derive(Clone)]
pub struct Recv {
    pub track_id: String,
    pub room: String,
    pub kind: Kind,
    pub class: RowClass,
    pub tuple: Tuple,
    pub room_fmtp: Option<String>,
}

pub fn track_tuple(t: &PubTrack, room: &Room) -> Tuple {
    match t.kind {
        Kind::Audio => Tuple::of(Kind::Audio, t.codec.as_deref().unwrap_or("opus"), t.fmtp.as_deref()),
        Kind::Video => Tuple::of(Kind::Video, &room.video_codec(), t.fmtp.as_deref()),
    }
}

pub fn slot_tuple(s: &Slot) -> Tuple {
    Tuple::of(s.kind, &s.codec, s.fmtp.as_deref())
}

pub fn receivable(room: &Room, hot: &RoomHot, me: &str) -> Vec<Recv> {
    let mut v = Vec::new();
    for s in hot.slots.iter() {
        v.push(Recv { track_id: s.track_id.clone(), room: room.id.clone(), kind: s.kind, class: RowClass::Slot, tuple: slot_tuple(s), room_fmtp: s.fmtp.clone() });
    }
    for t in room.tracks.iter().filter(|t| t.user != me && !t.half.load(Ordering::Relaxed)) {
        v.push(Recv { track_id: t.track_id.clone(), room: room.id.clone(), kind: t.kind, class: RowClass::Full, tuple: track_tuple(t, room), room_fmtp: None });
    }
    v
}

pub fn carrier(w: &World, sfu: &Sfu, track_id: &str, room: &str) -> Option<Carrier> {
    if let Some(h) = sfu.room_hot(room)
        && let Some(s) = h.slots.iter().find(|s| s.track_id == track_id)
    {
        return Some(Carrier::Slot(s.clone()));
    }
    w.rooms.get(room)?.listed(track_id).cloned().map(Carrier::Track)
}

pub fn place(sfu: &Sfu, w: &World, t: &Arc<Transport>, r: &Recv) -> bool {
    let now = Sfu::now();
    let up = t.uplink_ssrcs();
    let placed = {
        let mut table = lock(&t.table);
        let mut taken = table.ssrcs();
        taken.extend(up.iter().copied());
        let p = table.place(&r.track_id, &r.room, r.kind, r.class, &r.tuple, r.room_fmtp.as_deref(), now, || loop {
            let s = crate::link::fresh_u32();
            if taken.insert(s) {
                return s;
            }
        });
        if p == Placed::Full {
            table.wait(&r.track_id, &r.room, r.kind, &r.tuple, now);
        }
        p
    };
    if placed == Placed::Full {
        if try_reassign(sfu, w, t, r) {
            return true;
        }
        tracing::info!(kind = "ev", spec = "LINK.S.track_on_full", user = %t.user, track = %r.track_id, room = %r.room, "row cap — waiting without a row");
        return false;
    }
    tracing::debug!(kind = "ev", spec = "LINK.S.track_on", user = %t.user, track = %r.track_id, room = %r.room, "row placed");
    true
}

fn rank(w: &World, sfu: &Sfu, t: &Arc<Transport>, track: &str, room: &str, kind: Kind) -> (u64, u64) {
    let c = carrier(w, sfu, track, room);
    match (c, kind) {
        (Some(Carrier::Track(p)), Kind::Audio) => (0, p.last_level_ms.load(Ordering::Relaxed)),
        (Some(Carrier::Track(p)), Kind::Video) => {
            let prio = t.all_downs().iter().find(|d| d.track_id().as_deref() == Some(track)).map(|d| lock(&d.prefs).priority as u64).unwrap_or(0);
            (prio, p.state.load(Ordering::Relaxed) as u64)
        }
        _ => (0, 0),
    }
}

fn try_reassign(sfu: &Sfu, w: &World, t: &Arc<Transport>, r: &Recv) -> bool {
    let rows: Vec<(String, String, u64)> = lock(&t.table).rows.iter().filter(|x| x.class == RowClass::Full && x.kind == r.kind && x.tuple == r.tuple).filter_map(|x| x.track.clone().map(|(a, b)| (a, b, x.loaded_at))).collect();
    let mine = rank(w, sfu, t, &r.track_id, &r.room, r.kind);
    let victim = rows.iter().map(|(tr, rm, at)| (rank(w, sfu, t, tr, rm, r.kind), *at, tr.clone(), rm.clone())).min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let Some((vrank, _, vt, vr)) = victim else { return false };
    if mine <= vrank {
        return false;
    }
    let now = Sfu::now();
    let mut table = lock(&t.table);
    if table.reassign(&vt, &vr, &r.track_id, &r.room, now).is_some() {
        table.unwait(&r.track_id, &r.room);
        table.wait(&vt, &vr, r.kind, &r.tuple, now);
        tracing::info!(kind = "ev", spec = "LINK.S.reassign", user = %t.user, from = %vt, to = %r.track_id, "row reassigned at cap");
        return true;
    }
    false
}

pub fn reissue(sfu: &Sfu, w: &World, t: &Arc<Transport>) {
    let waiting: Vec<(String, String, Kind, Tuple, u64)> = {
        let mut v = lock(&t.table).waiting.clone();
        v.sort_by_key(|x| x.4);
        v
    };
    for (track, room, kind, tuple, _) in waiting {
        if carrier(w, sfu, &track, &room).is_none() {
            lock(&t.table).unwait(&track, &room);
            continue;
        }
        let free = lock(&t.table).rows.iter().any(|x| x.track.is_none() && x.class == RowClass::Full && x.kind == kind && x.tuple == tuple);
        if !free {
            continue;
        }
        lock(&t.table).unwait(&track, &room);
        let r = Recv { track_id: track.clone(), room: room.clone(), kind, class: RowClass::Full, tuple, room_fmtp: None };
        if place(sfu, w, t, &r) {
            tracing::info!(kind = "ev", spec = "LINK.S.reissue", user = %t.user, track = %track, room = %room, "waiting track got a freed row");
        }
    }
}

pub fn clear(t: &Arc<Transport>, track: &str, room: &str) -> bool {
    let mut table = lock(&t.table);
    let had = table.clear(track, room).is_some();
    let waited = table.unwait(track, room);
    if had {
        tracing::debug!(kind = "ev", spec = "LINK.S.track_off", user = %t.user, track = %track, room = %room, "row emptied");
    } else if waited {
        tracing::debug!(kind = "ev", spec = "LINK.S.track_off_norow", user = %t.user, track = %track, room = %room, "waiting track dropped");
    }
    had
}

pub fn sync(sfu: &Sfu, w: &World, peer: &Peer, notify: bool) {
    let t = &peer.subt;
    let tags = lock(&peer.ext_tags).clone();
    let rows = lock(&t.table).rows.clone();
    for row in rows.iter() {
        let d = match t.down(row.ssrc) {
            Some(d) => d,
            None => {
                let (pt, rtx) = lock(&t.table).pt_of(&row.tuple).unwrap_or((0, None));
                let ext = crate::sub::ext_table(t.pc_mode, tags.as_deref(), row.kind == Kind::Audio);
                let d = Down::new(t, row, pt, rtx, ext);
                d.carry.store(Arc::new(None));
                t.put_down(d.clone());
                d
            }
        };
        let cur = Option::clone(&d.carry.load());
        if cur != row.track {
            detach(sfu, w, &d, cur.as_ref());
            d.carry.store(Arc::new(row.track.clone()));
            d.since.store(row.since, Ordering::Release);
            d.need_kf.store(d.kind == Kind::Video, Ordering::Release);
            if let Some((tr, rm)) = &row.track {
                match carrier(w, sfu, tr, rm) {
                    Some(Carrier::Track(p)) => {
                        d.track.store(Arc::new(Some(p.clone())));
                        p.attach(&d);
                        if p.simulcast {
                            let mut sel = lock(&d.sel);
                            let want = target_layer(&d, sfu, t);
                            let _ = sel.retarget(want, Sfu::now());
                        }
                    }
                    Some(Carrier::Slot(s)) => {
                        d.track.store(Arc::new(None));
                        s.attach(&d);
                    }
                    None => d.track.store(Arc::new(None)),
                }
            } else {
                d.track.store(Arc::new(None));
            }
        } else {
            d.since.store(row.since.max(d.since.load(Ordering::Acquire)), Ordering::Release);
        }
    }
    let committed = lock(&t.table).commit();
    if committed && notify {
        let table = lock(&t.table).wire();
        let arev = table.arev;
        sfu.to_user(&t.user, Op::MlineEvent, serde_json::to_value(MlineEventBody { table }).unwrap_or_default(), None);
        tracing::info!(kind = "ev", spec = "CP-OP-MLINE-EVENT", user = %t.user, session = %t.session, arev, "mline table changed");
    }
}

fn detach(sfu: &Sfu, w: &World, d: &Arc<Down>, cur: Option<&(String, String)>) {
    if let Some(p) = Option::clone(&d.track.load()) {
        p.detach(d);
    }
    if let Some((tr, rm)) = cur
        && let Some(Carrier::Slot(s)) = carrier(w, sfu, tr, rm)
    {
        s.detach(d);
    }
}

pub fn target_layer(d: &Down, sfu: &Sfu, t: &Transport) -> Layer {
    let prefs = *lock(&d.prefs);
    if prefs.paused {
        return Layer::Pause;
    }
    let manual = prefs.spatial.map(|s| Layer::from_spatial(s.min(1))).unwrap_or(Layer::High);
    let auto = if sfu.auto_on() { lock(&t.bwe).policy.layer } else { Layer::High };
    match (manual, auto) {
        (Layer::Low, _) | (_, Layer::Low) => Layer::Low,
        _ => Layer::High,
    }
}

pub fn load_room(sfu: &Sfu, w: &World, peer: &Peer, room: &str) {
    let (Some(r), Some(h)) = (w.rooms.get(room), sfu.room_hot(room)) else { return };
    for recv in receivable(r, &h, &peer.user) {
        place(sfu, w, &peer.subt, &recv);
    }
}

pub fn track_on(sfu: &Sfu, w: &World, room: &str, track: &Arc<PubTrack>, notify: bool) {
    let Some(r) = w.rooms.get(room) else { return };
    let recv = Recv { track_id: track.track_id.clone(), room: room.to_string(), kind: track.kind, class: RowClass::Full, tuple: track_tuple(track, r), room_fmtp: None };
    for m in r.members.iter().filter(|m| m.user != track.user) {
        if let Some(p) = w.peers.get(&m.user) {
            place(sfu, w, &p.subt, &recv);
            sync(sfu, w, p, notify);
        }
    }
}

pub fn track_off(sfu: &Sfu, w: &World, room: &str, track_id: &str, notify: bool) {
    let Some(r) = w.rooms.get(room) else { return };
    for m in r.members.iter() {
        if let Some(p) = w.peers.get(&m.user) {
            if clear(&p.subt, track_id, room) {
                reissue(sfu, w, &p.subt);
            }
            sync(sfu, w, p, notify);
        }
    }
}

pub fn request_keyframes(sfu: &Sfu, t: &Arc<Transport>, room: Option<&str>) {
    let now = Sfu::now();
    let mut seen: Vec<u32> = Vec::new();
    for d in t.all_downs().into_iter().filter(|d| d.kind == Kind::Video) {
        if let Some(r) = room
            && d.room().as_deref() != Some(r)
        {
            continue;
        }
        let src = Option::clone(&d.cur.load()).map(|p| (p.ssrc, p)).or_else(|| Option::clone(&d.track.load()).and_then(|t| t.main_phys()).map(|p| (p.ssrc, p)));
        if let Some((ssrc, phys)) = src
            && !seen.contains(&ssrc)
        {
            seen.push(ssrc);
            crate::fwd::pli(sfu, &phys, now, true);
        }
    }
}

pub async fn ready(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: ReadyReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return sfu.fail(e.code().failure_with(e.message(), None)),
    };
    let mut w = sfu.world.lock().await;
    let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::NotInRoom.failure()) };
    let now = Sfu::now();
    let first = match w.rooms.get_mut(&req.room_id).and_then(|r| r.member_mut(&who.user)) {
        Some(m) => {
            let f = m.ready_at == 0;
            if f {
                m.ready_at = now;
            }
            f
        }
        None => return sfu.fail(Code::NotInRoom.failure()),
    };
    let t = peer.subt.clone();
    let opened = crate::sub::gate_open(&t, &req.room_id);
    let prev = t.ack.load(Ordering::Acquire);
    if req.arev > prev {
        t.ack.store(req.arev, Ordering::Release);
        tracing::info!(kind = "ev", spec = "LINK.S.ack", user = %who.user, session = %who.session, arev = req.arev, "table order acknowledged");
    } else {
        tracing::debug!(kind = "ev", spec = "LINK.S.ack_old", user = %who.user, arev = req.arev, acked = prev, "old arev");
    }
    request_keyframes(sfu, &t, Some(&req.room_id));
    let hot = sfu.room_hot(&req.room_id);
    drop(w);
    if let Some(h) = hot {
        let o = h.floor.ready(&who.user, now);
        crate::fwd::floor_out(sfu, &h, o);
    }
    tracing::info!(kind = "ev", spec = "SUB.S.ready", user = %who.user, session = %who.session, room = %req.room_id, arev = req.arev, first, gate_opened = opened, "READY");
    Reply::Ok(serde_json::json!({}))
}

pub async fn subscribe_layer(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: SubscribeLayerReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return sfu.fail(e.code().failure_with(e.message(), None)),
    };
    let w = sfu.world.lock().await;
    let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::NotInRoom.failure()) };
    if !peer.in_room(&req.room_id) {
        return sfu.fail(Code::NotInRoom.failure());
    }
    let t = peer.subt.clone();
    let now = Sfu::now();
    let mut reorder = false;
    for tg in &req.targets {
        let Some(d) = t.all_downs().into_iter().find(|d| d.track_id().as_deref() == Some(tg.track_id.as_str()) && d.room().as_deref() == Some(req.room_id.as_str())) else { continue };
        let was_paused;
        {
            let mut p = lock(&d.prefs);
            was_paused = p.paused;
            if let Some(s) = tg.spatial {
                p.spatial = Some(s.min(1));
            }
            if let Some(pa) = tg.paused {
                p.paused = pa;
            }
            if let Some(pr) = tg.priority {
                reorder |= pr != p.priority;
                p.priority = pr;
            }
        }
        let want = target_layer(&d, sfu, &t);
        let changed = lock(&d.sel).retarget(want, now);
        let unpaused = was_paused && !lock(&d.prefs).paused;
        if changed || unpaused {
            d.need_kf.store(true, Ordering::Release);
            if let Some(tr) = Option::clone(&d.track.load()) {
                let phys = want.spatial().and_then(|s| tr.phys_by_spatial(s)).or_else(|| tr.main_phys());
                if let Some(p) = phys {
                    crate::fwd::pli(sfu, &p, now, unpaused);
                }
            }
        }
    }
    if reorder {
        reissue(sfu, &w, &t);
        sync(sfu, &w, &peer, true);
    }
    tracing::info!(kind = "ev", spec = "CP-OP-SUBSCRIBE-LAYER", user = %who.user, room = %req.room_id, targets = req.targets.len(), "subscribe layer");
    Reply::Ok(serde_json::json!({}))
}
