// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-GC-FOUR · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-GC-PEER-REAP · 정:RULE-GC-EDGES · model: claude-opus-5-5
//! 회수 — 퇴장 · 강제 퇴장 · 세션 폐기가 한 절차(일곱 단계 · 순서가 계약)로 수렴 · 마지막 방이면 Peer 를 거둔다(등록 해제 · 태스크 종료까지) · 축출의 단위는 Peer.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::{Duplex, TrackAction};

use crate::link::Peer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Leave,
    Kick,
    Destroy,
    Evict,
    SessionGone,
    HomeLost,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::Leave => "leave",
            Why::Kick => "kick",
            Why::Destroy => "destroy",
            Why::Evict => "evict",
            Why::SessionGone => "session_gone",
            Why::HomeLost => "home_lost",
        }
    }
}

pub fn leave(peer: &Arc<Peer>, room_id: &str, why: Why, keep_roster: bool) -> bool {
    let Some(s) = crate::ops::sfu() else { return false };
    let Some(room) = s.rooms.get(room_id) else {
        tracing::warn!(kind = "diag", room = %room_id, user = %peer.user, "gc: room not found ({})", why.as_str());
        return false;
    };
    let user = peer.user.clone();
    crate::ops::floor::run(&room, |f, _| f.ws_leave(&user));
    if crate::aff::pub_room(peer).as_deref() == Some(room_id) {
        crate::aff::set_pub_room(peer, None);
        crate::ops::media::aff_line(&user, "AFF.S.left", Some(room_id), None, why.as_str());
    }
    let hidden = if keep_roster {
        room.member(&user).map(|m| m.hidden).unwrap_or(false)
    } else {
        match room.remove(&user) {
            Some(m) => m.hidden,
            None => {
                tracing::warn!(kind = "diag", room = %room_id, user = %user, "gc: not in roster ({})", why.as_str());
                return false;
            }
        }
    };
    if let Ok(mut g) = peer.rooms.lock() {
        g.remove(room_id);
    }
    if let Some(t) = s.table(peer) {
        t.forget_room(room_id);
    }
    let last = peer.rooms().is_empty();
    crate::ops::media::resync_peer(peer, !last);
    let mut removed = Vec::new();
    if let Some(ctx) = s.pubs.get(&user) {
        for st in ctx.streams() {
            let here = st.room.load().as_str() == room_id;
            let full_here = here && st.duplex() == Duplex::Full;
            let resting_here = here && st.listed.load(Ordering::Acquire);
            if full_here || last {
                if st.listed.swap(false, Ordering::AcqRel) && here {
                    removed.push(crate::trk::entry(&st));
                }
                crate::ops::media::drop_stream(peer, &st);
            } else if resting_here {
                removed.push(crate::trk::entry(&st));
                st.room.store(std::sync::Arc::new(String::new()));
            }
        }
    }
    if last {
        reap(peer, why);
    }
    if !removed.is_empty() && why != Why::Destroy {
        crate::ops::media::track_event(&room, TrackAction::Remove, removed);
        crate::ops::media::resync_room(room_id);
    }
    if !keep_roster && !hidden && why != Why::Destroy {
        room.announce_left(&user);
    }
    tracing::info!(kind = "diag", room = %room_id, user = %user, "gc: left via {} (state {})", why.as_str(), peer.publish.peer_state().as_str());
    true
}

pub fn reap(peer: &Arc<Peer>, why: Why) {
    let Some(s) = crate::ops::sfu() else { return };
    let mine = match s.peers.lock() {
        Ok(mut g) => match g.get(&peer.user) {
            Some(p) if Arc::ptr_eq(p, peer) => {
                g.remove(&peer.user);
                true
            }
            _ => false,
        },
        Err(_) => false,
    };
    if let Some(ctx) = s.pubs.get(&peer.user) {
        for st in ctx.streams() {
            crate::ops::media::drop_stream(peer, &st);
        }
    }
    if mine {
        s.pubs.remove(&peer.user);
        crate::aux::forget(&peer.user);
        crate::rtcp::forget_user(&peer.user);
    }
    s.tables.remove(peer.subscribe.id);
    let m = crate::fwd::rt::media();
    for (i, tr) in peer.transports().into_iter().enumerate() {
        tr.closed.store(true, Ordering::Release);
        let _ = tr.stop.send(true);
        if let Some(m) = m {
            m.unregister(&tr);
            m.release_worker(tr.worker, i == 0);
        }
    }
    tracing::info!(kind = "diag", user = %peer.user, "gc: peer reaped via {}", why.as_str());
}

pub fn evict(old: &Arc<Peer>, keep: Option<&str>, why: Why) {
    for rid in old.rooms() {
        let k = keep == Some(rid.as_str());
        leave(old, &rid, why, k);
    }
    reap(old, why);
}
