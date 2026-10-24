// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-GC-FOUR · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-GC-PEER-REAP · 정:RULE-GC-EDGES · 정:IMPL-BUS-HOME-LOST · 정:IMPL-ROOM-LIFE · 정:IMPL-AFF-FORCED · 연:RULE-LINK-SERVER-REAP · 연:LINK.S.peer_reaped · 연:LINK.S.room_gone_other · 연:ROOM.S.leave · 연:ROOM.S.leave_hidden · 연:ROOM.S.session_gone · 연:ROOM.S.session_gone_hidden · 연:ROOM.S.peer_evicted · 연:ROOM.S.kick · 연:ROOM.S.close_notify · 연:PUB.S.room_gone · 연:PUB.S.room_gone_other · 연:PUB.S.peer_gone · 연:SUB.S.drop · model: claude-opus-5-5
//! 회수 — 퇴장 · 강제 퇴장 · 세션 폐기가 한 절차(일곱 단계)로 수렴 · Peer 를 없애는 사건은 셋(세션 폐기 · 마지막 방 · 홈 잃음) · 빈 방 sweep 5,000ms · 홈 판정.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use oxsig::{Affiliation, Op, ParticipantEventBody, ParticipantEventBodyType, PublishTracksReqAddAction, RoomEventBody, RoomEventBodyAffiliationCause, TrackEventBody};

use crate::bus::core::Verdict;
use crate::link::lock;
use crate::ops::{subs, Sfu, World};
use crate::room::core::{sweep, Sweep};

pub const SWEEP_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Leave,
    Kick,
    Destroy,
    Session,
    Evict,
    LastRoom,
    HomeLost,
}

impl Path {
    pub fn as_str(self) -> &'static str {
        match self {
            Path::Leave => "leave",
            Path::Kick => "kick",
            Path::Destroy => "destroy",
            Path::Session => "session",
            Path::Evict => "evict",
            Path::LastRoom => "last_room",
            Path::HomeLost => "home_lost",
        }
    }

    fn label(self, hidden: bool) -> &'static str {
        match (self, hidden) {
            (Path::Leave, false) => "ROOM.S.leave",
            (Path::Leave, true) => "ROOM.S.leave_hidden",
            (Path::Kick, false) => "ROOM.S.kick",
            (Path::Kick, true) => "ROOM.S.kick_hidden",
            (Path::Destroy, _) => "ROOM.S.close_notify",
            (Path::Evict, false) => "ROOM.S.peer_evicted",
            (Path::Evict, true) => "ROOM.S.peer_evicted_hidden",
            (_, false) => "ROOM.S.session_gone",
            (_, true) => "ROOM.S.session_gone_hidden",
        }
    }
}

pub fn track_event(sfu: &Sfu, w: &mut World, room: &str, action: PublishTracksReqAddAction, tracks: Vec<oxsig::TrackEntry>) {
    if tracks.is_empty() {
        return;
    }
    let Some(r) = w.rooms.get_mut(room) else { return };
    r.bump();
    let body = TrackEventBody { action, room_id: room.to_string(), tracks, version: r.version(&sfu.epoch) };
    let label = match action {
        PublishTracksReqAddAction::Add => "CP-OP-TRACK-EVENT",
        PublishTracksReqAddAction::Remove => "CP-OP-TRACK-EVENT",
    };
    tracing::info!(kind = "ev", spec = label, room = %room, seq = r.seq, action = ?action, n = body.tracks.len(), "track event");
    sfu.to_room(room, Op::TrackEvent, serde_json::to_value(body).unwrap_or_default(), Vec::new());
}

pub fn leave_seven(sfu: &Sfu, w: &mut World, room_id: &str, user: &str, path: Path, keep_member: bool) -> bool {
    let now = Sfu::now();
    let Some(member) = w.rooms.get(room_id).and_then(|r| r.member(user).cloned()) else {
        tracing::warn!(kind = "diag", room = %room_id, user = %user, path = path.as_str(), "reclaim target not found");
        return false;
    };
    let hidden = member.hidden;
    let peer = w.peers.get(user).cloned();
    let hot = sfu.room_hot(room_id);
    if let Some(h) = &hot {
        let o = h.floor.leave(user, now);
        crate::fwd::floor_out(sfu, h, o);
        for s in h.slots.iter() {
            s.detach_user(user);
        }
    }
    if !keep_member && let Some(r) = w.rooms.get_mut(room_id) {
        r.members.retain(|m| m.user != user);
        r.stall_cool.remove(user);
    }
    if let Some(p) = &peer {
        let t = p.subt.clone();
        let cleared = lock(&t.table).clear_room(room_id);
        for d in t.all_downs().into_iter().filter(|d| d.room().as_deref() == Some(room_id)) {
            if let Some(tr) = Option::clone(&d.track.load()) {
                tr.detach(&d);
            }
        }
        crate::sub::gate_drop(&t, room_id);
        if !keep_member {
            lock(&p.rooms).remove(room_id);
            if p.pub_room().as_deref() == Some(room_id) {
                p.set_pub_room(None);
                tracing::info!(kind = "ev", spec = "AFF.S.left", user = %user, room = %room_id, "publish room left");
            }
        }
        if !cleared.is_empty() && !keep_member {
            subs::sync(sfu, w, p, false);
        }
    }
    let mut removed = Vec::new();
    if let Some(p) = &peer {
        let room_video = w.rooms.get(room_id).map(|r| r.video_codec());
        for tr in p.tracks().into_iter() {
            let listed = w.rooms.get(room_id).map(|r| r.listed(&tr.track_id).is_some()).unwrap_or(false);
            let half = tr.half.load(Ordering::Relaxed);
            let drop_pub = keep_member || (!half && tr.room() == room_id);
            if listed {
                if let Some(r) = w.rooms.get_mut(room_id) {
                    r.unlist(&tr.track_id);
                }
                removed.push(crate::trk::entry_of(&tr, room_id, !half, room_video.as_deref()));
                subs::track_off(sfu, w, room_id, &tr.track_id, true);
            }
            if drop_pub {
                crate::ops::pubs::unpublish(sfu, p, &tr);
                tracing::info!(kind = "ev", spec = if keep_member { "PUB.S.peer_gone" } else { "PUB.S.room_gone" }, user = %user, track = %tr.track_id, room = %room_id, "publication cleared");
            }
        }
    }
    track_event(sfu, w, room_id, PublishTracksReqAddAction::Remove, removed);
    if !hidden && !keep_member && let Some(r) = w.rooms.get_mut(room_id) {
        r.bump();
        let body = ParticipantEventBody { type_: ParticipantEventBodyType::Left, room_id: room_id.to_string(), user_id: user.to_string(), role: None, select: None, participant_type: None, metadata: None, permission: None, version: r.version(&sfu.epoch) };
        sfu.to_room(room_id, Op::ParticipantEvent, serde_json::to_value(body).unwrap_or_default(), vec![user.to_string()]);
    }
    if let Some(p) = &peer
        && !keep_member
        && matches!(path, Path::Kick | Path::Destroy)
    {
        let cause = if path == Path::Kick { RoomEventBodyAffiliationCause::Kick } else { RoomEventBodyAffiliationCause::RoomClosed };
        let body = RoomEventBody::Affiliation { room_id: room_id.to_string(), affiliation: p.affiliation(), cause };
        sfu.to_user(user, Op::RoomEvent, serde_json::to_value(body).unwrap_or_default(), Some(room_id));
    }
    if let Some(r) = w.rooms.get_mut(room_id)
        && r.members.is_empty()
        && r.ever_joined
    {
        r.empty_since = 0;
    }
    let peer_state = peer.as_ref().map(|p| p.pubt.peer_state().as_str()).unwrap_or("-");
    tracing::info!(kind = "ev", spec = path.label(hidden), room = %room_id, user = %user, path = path.as_str(), peer_state, "left room");
    if let Some(p) = &peer
        && !keep_member
        && p.rooms().is_empty()
        && !matches!(path, Path::Session | Path::Evict | Path::HomeLost)
    {
        reap_peer_now(sfu, w, user, Path::LastRoom);
    } else if peer.is_some() && !keep_member {
        tracing::debug!(kind = "ev", spec = "LINK.S.room_gone_other", user = %user, room = %room_id, "peer keeps other rooms");
    }
    true
}

fn reap_peer_now(sfu: &Sfu, w: &mut World, user: &str, path: Path) {
    let Some(p) = w.peers.remove(user) else { return };
    for tr in p.tracks() {
        crate::ops::pubs::unpublish(sfu, &p, &tr);
    }
    let state = p.pubt.peer_state().as_str();
    p.close();
    sfu.sync_peers(w);
    tracing::info!(kind = "ev", spec = "LINK.S.peer_reaped", user = %user, session = %p.session, path = path.as_str(), peer_state = state, "peer reaped — transports and tasks closed");
}

pub fn reap_peer(sfu: &Sfu, w: &mut World, user: &str, path: Path) -> bool {
    let Some(p) = w.peers.get(user).cloned() else { return false };
    for room in p.rooms() {
        leave_seven(sfu, w, &room, user, path, false);
    }
    reap_peer_now(sfu, w, user, path);
    true
}

pub fn affiliation_after(p: Option<&crate::aff::Peer>) -> Affiliation {
    p.map(|p| p.affiliation()).unwrap_or(Affiliation { sub_rooms: Vec::new(), pub_room: None })
}

pub fn destroy_room(sfu: &Sfu, w: &mut World, room_id: &str) -> usize {
    let users: Vec<String> = w.rooms.get(room_id).map(|r| r.members.iter().map(|m| m.user.clone()).collect()).unwrap_or_default();
    let n = users.len();
    for u in &users {
        leave_seven_destroy(sfu, w, room_id, u);
    }
    w.rooms.remove(room_id);
    sfu.drop_hot(room_id);
    n
}

fn leave_seven_destroy(sfu: &Sfu, w: &mut World, room_id: &str, user: &str) {
    let now = Sfu::now();
    let peer = w.peers.get(user).cloned();
    if let Some(h) = sfu.room_hot(room_id) {
        let o = h.floor.leave(user, now);
        crate::fwd::floor_out(sfu, &h, o);
        for s in h.slots.iter() {
            s.detach_user(user);
        }
    }
    if let Some(r) = w.rooms.get_mut(room_id) {
        r.members.retain(|m| m.user != user);
    }
    if let Some(p) = &peer {
        let t = p.subt.clone();
        lock(&t.table).clear_room(room_id);
        for d in t.all_downs().into_iter().filter(|d| d.room().as_deref() == Some(room_id)) {
            if let Some(tr) = Option::clone(&d.track.load()) {
                tr.detach(&d);
            }
        }
        crate::sub::gate_drop(&t, room_id);
        lock(&p.rooms).remove(room_id);
        if p.pub_room().as_deref() == Some(room_id) {
            p.set_pub_room(None);
        }
        for tr in p.tracks() {
            if !tr.half.load(Ordering::Relaxed) && tr.room() == room_id {
                if let Some(r) = w.rooms.get_mut(room_id) {
                    r.unlist(&tr.track_id);
                }
                crate::ops::pubs::unpublish(sfu, p, &tr);
            } else if let Some(r) = w.rooms.get_mut(room_id) {
                r.unlist(&tr.track_id);
            }
        }
        subs::sync(sfu, w, p, true);
        let body = RoomEventBody::Affiliation { room_id: room_id.to_string(), affiliation: p.affiliation(), cause: RoomEventBodyAffiliationCause::RoomClosed };
        sfu.to_user(user, Op::RoomEvent, serde_json::to_value(body).unwrap_or_default(), Some(room_id));
        tracing::info!(kind = "ev", spec = "ROOM.S.close_notify", room = %room_id, user = %user, "room closed — member notified");
        if p.rooms().is_empty() {
            reap_peer_now(sfu, w, user, Path::LastRoom);
        }
    }
}

pub async fn sweep_loop(sfu: Arc<Sfu>) {
    let mut iv = tokio::time::interval(Duration::from_millis(SWEEP_MS));
    loop {
        iv.tick().await;
        let now = Sfu::now();
        let mut expired = Vec::new();
        {
            let mut w = sfu.world.lock().await;
            for r in w.rooms.values_mut() {
                match sweep(r.members.len(), r.ever_joined, r.empty_since, r.unused_ttl_secs, r.departure_ttl_secs, now) {
                    Sweep::Keep => {}
                    Sweep::MarkEmpty => r.empty_since = now.max(1),
                    Sweep::Expire => expired.push(r.id.clone()),
                }
            }
            for id in &expired {
                w.rooms.remove(id);
                sfu.drop_hot(id);
                tracing::info!(kind = "ev", spec = "IMPL-ROOM-LIFE", room = %id, "empty room expired — removed");
            }
        }
        for id in expired {
            sfu.bus.undeclare_room(&id).await;
        }
    }
}

pub async fn home_loop(sfu: Arc<Sfu>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Verdict>) {
    while let Some(v) = rx.recv().await {
        let mut w = sfu.world.lock().await;
        match v {
            Verdict::Rehome { node, from, to } => {
                for p in w.peers.values() {
                    let h = p.home();
                    if h.node == node && h.inst == from {
                        p.set_home(&common::bus::envelope::Home { node: node.clone(), inst: to.clone() });
                    }
                }
                tracing::info!(kind = "ev", spec = "CAND-BUS-HOME-INST", node = %node, from = %from, to = %to, "home moved to the new node instance");
            }
            Verdict::Reap { node, inst } => {
                let users: Vec<String> = w.peers.values().filter(|p| {
                    let h = p.home();
                    h.node == node && h.inst == inst
                }).map(|p| p.user.clone()).collect();
                for u in users {
                    reap_peer(&sfu, &mut w, &u, Path::HomeLost);
                }
                tracing::info!(kind = "ev", spec = "IMPL-BUS-HOME-LOST", node = %node, inst = %inst, "home lost — peers reclaimed");
            }
            Verdict::Keep { node, inst, why } => {
                tracing::info!(kind = "ev", spec = "IMPL-BUS-HOME-LOST", node = %node, inst = %inst, why, "home lost — not reclaimed");
            }
        }
    }
}
