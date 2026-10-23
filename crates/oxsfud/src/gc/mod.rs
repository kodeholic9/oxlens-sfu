// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-GC-FOUR · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-GC-PEER-REAP · 정:RULE-GC-EDGES · 정:IMPL-BUS-HOME-LOST · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-STALL · 정:IMPL-AFF-FORCED · model: claude-opus-5-5
//! GC — 퇴장 정리 일곱 단계(세 경로가 한 절차로) · 마지막 방이면 Peer 를 거둔다(태스크 종료까지) · 홈을 잃은 Peer · 빈 방 TTL sweep 5s · 정체 감지 5s.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use common::bus::Notice;
use oxsig::aff::Affiliation;
use oxsig::room::{AffiliationCause, ParticipantEvent, ParticipantEventType, RoomEvent};
use oxsig::trk::{MediaKind, TrackType};

use crate::cfg::Ctx;
use crate::flr::core::{Msg, T};
use crate::link::{lock, Peer};
use crate::ops::wire;
use crate::room::Room;

pub const SWEEP_MS: u64 = 5000;
pub const STALL_MS: u64 = 5000;
pub const STALL_COOL_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    Leave,
    Kick,
    Close,
    Evict,
    Session,
    HomeLost,
}

impl Path {
    pub fn name(self) -> &'static str {
        match self {
            Path::Leave => "leave",
            Path::Kick => "kick",
            Path::Close => "room_closed",
            Path::Evict => "evict",
            Path::Session => "session",
            Path::HomeLost => "home_lost",
        }
    }
}

fn now() -> u64 {
    common::clock::wall_ms()
}

pub fn affiliation(p: &Peer) -> Affiliation {
    Affiliation { sub_rooms: p.room_list(), pub_room: p.pub_room() }
}

pub fn release_floor(room: &Room, user: &str) {
    if room.floor.holder().as_deref() == Some(user) {
        let u = user.to_string();
        let m = Msg::new(T::Release, None, 0, 0);
        room.floor.apply("RELEASE", move |s| s.server_rx(&u, &m));
    }
}

pub fn leave_seven(ctx: &Ctx, room: &Arc<Room>, p: &Arc<Peer>, path: Path) -> bool {
    let user = p.user_id.clone();
    let u1 = user.clone();
    room.floor.apply(path.name(), move |s| s.ws_leave(&u1));
    let removed = lock(&room.inner).members.remove(&user);
    let Some(m) = removed else {
        tracing::warn!(kind = "diag", room = %room.id, user = %user, path = path.name(), "회수 대상 미발견");
        return false;
    };
    lock(&p.rooms).remove(&room.id);
    crate::sub::gate_drop(p, &room.id);
    if p.pub_room().as_deref() == Some(room.id.as_str()) {
        p.pub_room.store(None);
    }
    let rows = wire::drop_room_rows(ctx, p, &room.id);
    let last = lock(&p.rooms).is_empty();
    let mine: Vec<Arc<crate::r#pub::PubStream>> = lock(&p.pubs).iter().filter(|s| !s.gone.load(Ordering::Acquire)).cloned().collect();
    for s in &mine {
        let full_here = !s.half.load(Ordering::Acquire) && s.room_id() == room.id;
        let resting_here = s.half.load(Ordering::Acquire) && s.room_id() == room.id && ctx.tracks.has(&room.id, &s.track_id);
        if full_here || resting_here || last {
            s.gone.store(true, Ordering::Release);
            for ph in s.phys.load().iter() {
                p.pub_t.ingress.remove(&ph.ssrc);
            }
            if ctx.tracks.entry(&s.room_id(), &s.track_id).map(|t| t.kind_of == TrackType::Individual).unwrap_or(false)
                && let Some(r) = ctx.rooms.get(&s.room_id()) {
                    ctx.tracks.remove(&r, &s.track_id);
                }
            wire::track_off(ctx, &s.track_id);
        }
    }
    if last {
        ctx.links.reap(p);
        tracing::info!(kind = "ev", spec = "LINK.S.peer_reaped", user = %user, session = %p.session(), epoch = %ctx.boot.epoch, path = path.name(), state = p.pub_t.peer_state().name(), area = "LINK", key = %user, cause = path.name(), "Peer 회수");
    } else {
        lock(&p.pubs).retain(|s| !s.gone.load(Ordering::Acquire));
        let half = lock(&p.pubs).iter().any(|s| s.half.load(Ordering::Acquire));
        for r in p.room_list() {
            if let Some(room) = ctx.rooms.get(&r) {
                let u = user.clone();
                room.floor.apply("reg_half", move |s| s.ws_reg_half(&u, half));
            }
        }
        if rows {
            wire::emit_table(p);
        }
    }
    if !m.hidden && path != Path::Close {
        room.change(|_, v| {
            let body = ParticipantEvent { kind: ParticipantEventType::Left, room_id: room.id.clone(), user_id: user.clone(), role: None, select: None, participant_type: None, metadata: None, permission: None, version: v };
            Some(Notice { op: oxsig::op::PARTICIPANT_EVENT, body: serde_json::to_value(body).ok()?, exclude: vec![user.clone()], room_id: Some(room.id.clone()), sfu_id: None, control: None })
        });
    }
    tracing::info!(kind = "ev", spec = "CP-OP-ROOM-LEAVE", room = %room.id, user = %user, session = %p.session(), epoch = %ctx.boot.epoch, path = path.name(), "퇴장 정리");
    true
}

pub fn forced(ctx: &Ctx, room: &Arc<Room>, p: &Arc<Peer>, cause: AffiliationCause) {
    let path = if cause == AffiliationCause::Kick { Path::Kick } else { Path::Close };
    let mut aff = affiliation(p);
    aff.sub_rooms.retain(|r| r != &room.id);
    if aff.pub_room.as_deref() == Some(room.id.as_str()) {
        aff.pub_room = None;
    }
    let ev = RoomEvent::Affiliation { room_id: room.id.clone(), affiliation: aff, cause };
    room.emit_user(&p.user_id, Notice { op: oxsig::op::ROOM_EVENT, body: serde_json::to_value(ev).unwrap_or_default(), exclude: Vec::new(), room_id: Some(room.id.clone()), sfu_id: Some(ctx.boot.epoch.clone()), control: None });
    leave_seven(ctx, room, p, path);
}

pub fn drop_peer(ctx: &Ctx, p: &Arc<Peer>, path: Path) {
    let rooms = p.room_list();
    for r in rooms {
        if let Some(room) = ctx.rooms.get(&r) {
            leave_seven(ctx, &room, p, path);
        }
    }
    if !p.reaped.load(Ordering::Acquire) {
        for s in lock(&p.pubs).iter() {
            if !s.gone.swap(true, Ordering::AcqRel) {
                wire::track_off(ctx, &s.track_id);
            }
        }
        ctx.links.reap(p);
    }
}

pub fn session_gone(ctx: &Ctx, session: &str, user: &str, path: &str) -> bool {
    let Some(p) = ctx.links.peer(user) else { return false };
    if p.session() != session {
        return false;
    }
    tracing::info!(kind = "ev", spec = "CP-OP-LEAVE", user = %user, session = %session, epoch = %ctx.boot.epoch, path = %path, "세션 폐기 — Peer 회수");
    drop_peer(ctx, &p, Path::Session);
    true
}

pub fn home_lost(ctx: &Ctx, node: &str, inst: &str, new: Option<&str>) {
    for p in ctx.links.all() {
        let mine = {
            let h = lock(&p.home);
            h.0 == node && h.1 == inst
        };
        if !mine {
            continue;
        }
        match new {
            Some(n) => {
                lock(&p.home).1 = n.to_string();
                tracing::info!(kind = "diag", user = %p.user_id, node = %node, from = %inst, to = %n, "홈 옮겨 달기");
            }
            None => {
                tracing::info!(kind = "ev", spec = "LINK.S.peer_reaped", user = %p.user_id, node = %node, inst = %inst, path = "home_lost", "홈을 잃은 Peer");
                drop_peer(ctx, &p, Path::HomeLost);
            }
        }
    }
}

pub fn destroy(ctx: &Ctx, room: &Arc<Room>) -> usize {
    let users = room.member_ids();
    let mut n = 0;
    for u in &users {
        if let Some(p) = ctx.links.peer(u) {
            forced(ctx, room, &p, AffiliationCause::RoomClosed);
            n += 1;
        }
    }
    remove_room(ctx, room);
    n
}

pub fn remove_room(ctx: &Ctx, room: &Arc<Room>) {
    room.floor.dead.store(true, Ordering::Release);
    ctx.tracks.drop_room(&room.id);
    ctx.rooms.remove(&room.id);
    ctx.bus.undeclare_room(&room.id);
    tracing::info!(kind = "ev", spec = "SV-STATE-ROOM-LIFE", room = %room.id, epoch = %ctx.boot.epoch, "방 소멸 · room 토큰 철회");
}

fn stall_one(ctx: &Ctx, room: &Arc<Room>, p: &Arc<Peer>, now: u64) {
    if p.sub_transport().peer_state() != crate::link::core::PeerState::Alive || !p.sub_transport().has_dtls() {
        return;
    }
    let ready = crate::sub::ready_at(p, &room.id);
    if ready == 0 {
        return;
    }
    let downs: Vec<Arc<crate::link::Down>> = lock(&p.downs).values().cloned().collect();
    let mut stalled = false;
    for d in downs {
        let Some(l) = d.loaded() else { continue };
        if l.room_id != room.id || !crate::fwd::shell::sendable(&d, p) {
            continue;
        }
        let last = d.last_at.load(Ordering::Acquire);
        let start = if d.class == crate::link::table::Class::Slot {
            let slot = room.slot(d.kind);
            let Some(sp) = slot.speaker() else { continue };
            if sp == p.user_id {
                continue;
            }
            let lr = room.floor.last_rtp_at.load(Ordering::Acquire);
            let bound = slot.bound_at.load(Ordering::Acquire);
            if lr == 0 || lr < bound.max(ready) || now.saturating_sub(lr) > STALL_MS {
                continue;
            }
            ready.max(bound)
        } else {
            let Some(s) = wire::stream_of(ctx, &l.track_id) else { continue };
            if s.half.load(Ordering::Acquire) || s.muted.load(Ordering::Acquire) {
                continue;
            }
            if d.kind == MediaKind::Video {
                if crate::sub::pref(p, &l.track_id).paused {
                    continue;
                }
                let f = s.first_rtp_at.load(Ordering::Acquire);
                if f == 0 {
                    continue;
                }
                ready.max(f)
            } else {
                ready
            }
        };
        if now.saturating_sub(start) < STALL_MS {
            continue;
        }
        if now.saturating_sub(last.max(start)) > STALL_MS {
            stalled = true;
        }
    }
    if !stalled {
        return;
    }
    {
        let mut g = lock(&room.inner);
        let cool = g.stall_cool.get(&p.user_id).copied().unwrap_or(0);
        if cool > 0 && now.saturating_sub(cool) < STALL_COOL_MS {
            return;
        }
        g.stall_cool.insert(p.user_id.clone(), now);
    }
    tracing::info!(kind = "ev", spec = "ROOM.S.sync_required", room = %room.id, user = %p.user_id, epoch = %ctx.boot.epoch, area = "ROOM", key = %room.id, cause = "stall", "정체 감지");
    let ev = RoomEvent::SyncRequired { room_id: room.id.clone(), reason: "no_media_flow".to_string() };
    room.emit_user(&p.user_id, Notice { op: oxsig::op::ROOM_EVENT, body: serde_json::to_value(ev).unwrap_or_default(), exclude: Vec::new(), room_id: Some(room.id.clone()), sfu_id: None, control: None });
}

pub fn spawn_sweeps() {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(SWEEP_MS));
        loop {
            iv.tick().await;
            let Some(ctx) = crate::cfg::get() else { continue };
            let now = now();
            for room in ctx.rooms.all() {
                if crate::room::Rooms::tick_empty(&room, now) {
                    tracing::info!(kind = "ev", spec = "SV-STATE-ROOM-LIFE", room = %room.id, epoch = %ctx.boot.epoch, "빈 방 TTL 만료");
                    remove_room(ctx, &room);
                    continue;
                }
                for u in room.member_ids() {
                    if let Some(p) = ctx.links.peer(&u) {
                        stall_one(ctx, &room, &p, now);
                    }
                }
            }
        }
    });
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(1000));
        loop {
            iv.tick().await;
            if let Some(ctx) = crate::cfg::get() {
                wire::reassign_tick(ctx);
            }
        }
    });
}
