// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-GC-FOUR 정:RULE-GC-LEAVE-SEVEN 정:IMPL-GC-PEER-REAP 정:RULE-GC-EDGES 정:IMPL-ROOM-LIFE 정:IMPL-ROOM-STALL 정:IMPL-BUS-HOME-LOST 정:IMPL-LINK-PEER-STATE 정:RULE-LINK-PAIR-LIVENESS 정:IMPL-FLR-RTP-OBSERVE 정:IMPL-OPS-DESTROY-REAP 정:IMPL-AFF-FORCED 정:RULE-CONC-SIX 연:ROOM.S.leave 연:ROOM.S.session_gone 연:ROOM.S.peer_evicted 연:ROOM.S.kick 연:ROOM.S.close_notify 연:LINK.S.peer_reaped 연:LINK.S.suspect 연:ROOM.S.sync_required · model: claude-opus-5-5
//! 회수 — 퇴장 일곱(순서가 계약) · 마지막 방이면 Peer 거두기(색인 해제 먼저 · 태스크 직접 종료) · 방 sweep 5s · 정체 감지 5s · 발언권 만료 시각 · 홈 잃음 · 쌍별 생존.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::Op;
use oxsig::msg::{Affiliation, Kind, LinkEventBody, LinkEventBodyType, ParticipantEventBody, ParticipantEventBodyType, RoomEventBody, RoomEventBodyAffiliationCause};

use crate::fwd::shell::now_ms;
use crate::link::transport::PeerState;
use crate::ops::Sfu;
use crate::ops::peer::Peer;
use crate::room::{Room, Sweep};
use crate::sub::table::Class;

pub const ROOM_SWEEP_MS: u64 = 5_000;
pub const STALL_MS: u64 = 5_000;
pub const LINK_WATCH_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Leave,
    Kick,
    Closed,
    SessionGone,
    Evicted,
    HomeLost,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::Leave => "leave",
            Why::Kick => "kick",
            Why::Closed => "room_closed",
            Why::SessionGone => "session_gone",
            Why::Evicted => "evicted",
            Why::HomeLost => "home_lost",
        }
    }

    fn label(self, hidden: bool) -> &'static str {
        match (self, hidden) {
            (Why::Leave, false) => "ROOM.S.leave",
            (Why::Leave, true) => "ROOM.S.leave_hidden",
            (Why::Kick, false) => "ROOM.S.kick",
            (Why::Kick, true) => "ROOM.S.kick_hidden",
            (Why::Closed, _) => "ROOM.S.close_notify",
            (Why::SessionGone | Why::HomeLost, false) => "ROOM.S.session_gone",
            (Why::SessionGone | Why::HomeLost, true) => "ROOM.S.session_gone_hidden",
            (Why::Evicted, false) => "ROOM.S.peer_evicted",
            (Why::Evicted, true) => "ROOM.S.peer_evicted_hidden",
        }
    }
}

pub async fn leave(sfu: &Arc<Sfu>, peer: &Arc<Peer>, room: &Arc<Room>, why: Why, broadcast: bool) -> bool {
    let now = now_ms();
    let step = room.floor.leave(&peer.user, now);
    crate::ops::floor::exec(sfu, room, step).await;
    let gone = room.lock().members.remove(&peer.user);
    let Some(member) = gone else {
        tracing::warn!(user = peer.user.as_str(), room = room.id.as_str(), "leave: not in roster");
        return false;
    };
    let last = peer.aff.count() <= 1;
    {
        let mut st = peer.lock();
        st.table.drop_room(&room.id, now);
        let changed = st.table.finish();
        let wire = (changed.is_some() && !last).then(|| st.table.wire());
        st.gates.remove(&room.id);
        st.prefs.retain(|(r, _), _| *r != room.id);
        drop(st);
        crate::ops::sub::sync(sfu, peer);
        if let Some(t) = wire {
            sfu.emit_user(&peer.user, Op::MlineEvent, &oxsig::msg::MlineEventBody { table: t }, None);
        }
    }
    let was_pub = peer.aff.is_pub(&room.id);
    peer.aff.remove(&room.id);
    let mine: Vec<String> = room.lock().tracks.iter().filter(|l| l.user == peer.user).map(|l| l.track_id.clone()).collect();
    if was_pub {
        let full: Vec<String> = peer.streams().iter().filter(|s| !s.half()).map(|s| s.track_id.clone()).collect();
        for t in &full {
            peer.lock().pubs.remove(t);
            crate::ops::publish::drop_stream(peer, t);
        }
        for s in peer.streams() {
            s.listed.store(false, Ordering::Release);
        }
    }
    if broadcast {
        crate::ops::publish::unlist(sfu, room, &mine);
    } else {
        let mut st = room.lock();
        for t in &mine {
            st.take_track(t);
        }
    }
    if broadcast && !member.hidden {
        let mut st = room.lock();
        let seq = st.bump();
        let ev = ParticipantEventBody {
            type_: ParticipantEventBodyType::Left,
            room_id: room.id.clone(),
            user_id: peer.user.clone(),
            role: None,
            select: None,
            participant_type: None,
            metadata: None,
            permission: None,
            version: room.version(seq),
        };
        sfu.emit_room(&room.id, Op::ParticipantEvent, &ev, vec![peer.user.clone()]);
    }
    let (state, _) = peer.sub_conn().link.state();
    common::log::transition(&common::log::Step {
        area: "ROOM",
        key: &room.id,
        from: "JOINED",
        to: Some("NONE"),
        cause: why.name(),
        label: Some(why.label(member.hidden)),
        ins: &[("peer_state", state.name().to_string())],
        room: Some(&room.id),
        user: Some(&peer.user),
    });
    true
}

pub fn reap(sfu: &Arc<Sfu>, peer: &Arc<Peer>, why: &str) {
    if peer.gone.swap(true, Ordering::AcqRel) {
        return;
    }
    let (state, _) = peer.sub_conn().link.state();
    for c in &peer.conns {
        sfu.plane.unbind_conn(c);
    }
    crate::ops::sub::detach_all(sfu, peer);
    for c in &peer.conns {
        for i in c.ingress.load().values() {
            i.subs.store(Arc::new(Vec::new()));
        }
        c.ingress.store(Arc::new(Default::default()));
        c.link.close();
    }
    sfu.peers.remove_if(&peer.user, |_, v| Arc::ptr_eq(v, peer));
    common::log::transition(&common::log::Step {
        area: "LINK",
        key: &peer.user,
        from: state.name(),
        to: None,
        cause: why,
        label: Some("LINK.S.peer_reaped"),
        ins: &[("rooms", peer.aff.count().to_string())],
        room: None,
        user: Some(&peer.user),
    });
}

pub async fn drop_peer(sfu: &Arc<Sfu>, peer: &Arc<Peer>, why: Why) {
    let _op = peer.op.lock().await;
    for r in peer.aff.rooms() {
        if let Some(room) = sfu.room(&r) {
            leave(sfu, peer, &room, why, true).await;
        }
    }
    reap(sfu, peer, why.name());
}

pub async fn evict(sfu: &Arc<Sfu>, old: &Arc<Peer>, keep: Option<&str>) {
    let _op = old.op.lock().await;
    for r in old.aff.rooms() {
        let Some(room) = sfu.room(&r) else { continue };
        if Some(r.as_str()) == keep {
            let step = room.floor.leave(&old.user, now_ms());
            crate::ops::floor::exec(sfu, &room, step).await;
            let mine: Vec<String> = room.lock().tracks.iter().filter(|l| l.user == old.user).map(|l| l.track_id.clone()).collect();
            crate::ops::publish::unlist(sfu, &room, &mine);
            old.aff.remove(&r);
            continue;
        }
        leave(sfu, old, &room, Why::Evicted, true).await;
    }
    reap(sfu, old, Why::Evicted.name());
}

fn aff_without(peer: &Peer, room: &str) -> Affiliation {
    let mut a = peer.aff.snapshot();
    a.sub_rooms.retain(|r| r != room);
    if a.pub_room.as_deref() == Some(room) {
        a.pub_room = None;
    }
    a
}

pub async fn kick(sfu: &Arc<Sfu>, peer: &Arc<Peer>, room: &Arc<Room>) -> bool {
    let _op = peer.op.lock().await;
    let ok = leave(sfu, peer, room, Why::Kick, true).await;
    if ok {
        let ev = RoomEventBody::Affiliation { room_id: room.id.clone(), affiliation: aff_without(peer, &room.id), cause: RoomEventBodyAffiliationCause::Kick };
        sfu.emit_user(&peer.user, Op::RoomEvent, &ev, Some(&room.id));
        if peer.aff.count() == 0 {
            reap(sfu, peer, "last_room");
        }
    }
    ok
}

pub async fn close_room(sfu: &Arc<Sfu>, room: &Arc<Room>) -> u32 {
    let users: Vec<String> = room.lock().members.keys().cloned().collect();
    let mut n = 0;
    for u in users {
        let Some(p) = sfu.peer(&u) else {
            room.lock().members.remove(&u);
            continue;
        };
        let ev = RoomEventBody::Affiliation { room_id: room.id.clone(), affiliation: aff_without(&p, &room.id), cause: RoomEventBodyAffiliationCause::RoomClosed };
        sfu.emit_user(&u, Op::RoomEvent, &ev, Some(&room.id));
        n += 1;
        let _op = p.op.lock().await;
        leave(sfu, &p, room, Why::Closed, false).await;
        if p.aff.count() == 0 {
            reap(sfu, &p, "last_room");
        }
    }
    remove_room(sfu, room).await;
    n
}

pub async fn remove_room(sfu: &Arc<Sfu>, room: &Arc<Room>) {
    sfu.rooms.remove_if(&room.id, |_, v| Arc::ptr_eq(v, room));
    sfu.plane.remove_room(&room.id);
    room.floor.wake.notify_one();
    sfu.bus.undeclare_room(&room.id).await;
    common::ev!(spec = "SV-STATE-UNIT-STATE", room = room.id.as_str(), "room removed");
}

pub fn start_floor(sfu: &Arc<Sfu>, room: &Arc<Room>) {
    let s = sfu.clone();
    let r = room.clone();
    tokio::spawn(async move {
        loop {
            if s.room(&r.id).is_none_or(|x| !Arc::ptr_eq(&x, &r)) {
                break;
            }
            let now = now_ms();
            let wait = match r.floor.next_deadline() {
                Some(d) => Duration::from_millis(d.saturating_sub(now)),
                None => Duration::from_secs(60),
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = r.floor.wake.notified() => continue,
            }
            for st in r.floor.due(now_ms()) {
                crate::ops::floor::exec(&s, &r, st).await;
            }
        }
    });
}

async fn room_sweep(sfu: Arc<Sfu>) {
    loop {
        tokio::time::sleep(Duration::from_millis(ROOM_SWEEP_MS)).await;
        let now = now_ms();
        let rooms: Vec<Arc<Room>> = sfu.rooms.iter().map(|r| r.clone()).collect();
        for r in rooms {
            if matches!(r.sweep(now), Sweep::Expired) {
                let _g = sfu.rooms_op.lock().await;
                if r.lock().members.is_empty() {
                    common::ev!(spec = "SV-STATE-UNIT-STATE", room = r.id.as_str(), "room ttl expired");
                    remove_room(&sfu, &r).await;
                }
            }
        }
    }
}

fn stall_check(sfu: &Arc<Sfu>, peer: &Arc<Peer>, now: u64) {
    let conn = peer.sub_conn();
    if conn.link.state().0 != PeerState::Alive {
        return;
    }
    for e in conn.egress.load().iter() {
        let Some(l) = e.load.load_full() else { continue };
        if e.prefs.load().as_ref().is_some_and(|p| p.paused.load(Ordering::Relaxed)) {
            e.stall_from.store(0, Ordering::Release);
            continue;
        }
        let Some(g) = peer.gate(&l.room) else { continue };
        let ready = g.ready_at.load(Ordering::Acquire);
        if ready == 0 || !e.acked() {
            e.stall_from.store(0, Ordering::Release);
            continue;
        }
        let mut start = ready;
        let mut speaker_rtp = None;
        match e.class {
            Class::Full => {
                if let Some(s) = &l.stream
                    && e.kind == Kind::Video
                {
                    let f = s.first_rtp_ms.load(Ordering::Acquire);
                    if f == 0 || !g.open() {
                        e.stall_from.store(0, Ordering::Release);
                        continue;
                    }
                    start = start.max(f);
                }
            }
            Class::Slot => {
                let Some(rm) = sfu.plane.room(&l.room) else { continue };
                let Some(b) = rm.slot(e.kind).bound.load_full() else {
                    e.stall_from.store(0, Ordering::Release);
                    continue;
                };
                if b.user == peer.user || !peer_has_half(sfu, &b.user, e.kind) {
                    e.stall_from.store(0, Ordering::Release);
                    continue;
                }
                start = start.max(b.at_ms);
                speaker_rtp = sfu.room(&l.room).map(|r| r.floor.last_rtp_at.load(Ordering::Acquire));
            }
        }
        let from = e.stall_from.load(Ordering::Acquire);
        let sent = e.sent.load(Ordering::Relaxed);
        if from == 0 || from < start {
            e.stall_from.store(now.max(start), Ordering::Release);
            e.stall_count.store(sent, Ordering::Release);
            continue;
        }
        if now.saturating_sub(from) < STALL_MS {
            continue;
        }
        let moved = sent != e.stall_count.load(Ordering::Acquire);
        let flowing_src = speaker_rtp.is_none_or(|t| t > from);
        e.stall_from.store(now, Ordering::Release);
        e.stall_count.store(sent, Ordering::Release);
        if moved || !flowing_src {
            continue;
        }
        let Some(room) = sfu.room(&l.room) else { continue };
        if !room.lock().stall_allowed(&peer.user, now) {
            continue;
        }
        sfu.drops.stall_sent.fetch_add(1, Ordering::Relaxed);
        let ev = RoomEventBody::SyncRequired { room_id: l.room.clone(), reason: "no_media_flow".into() };
        sfu.emit_user(&peer.user, Op::RoomEvent, &ev, Some(&l.room));
        common::log::transition(&common::log::Step {
            area: "ROOM",
            key: &l.room,
            from: "JOINED",
            to: Some("JOINED"),
            cause: "stall",
            label: Some("ROOM.S.sync_required"),
            ins: &[("mid", e.mid.to_string()), ("track", l.track_id.clone())],
            room: Some(&l.room),
            user: Some(&peer.user),
        });
    }
}

fn peer_has_half(sfu: &Sfu, user: &str, kind: Kind) -> bool {
    sfu.peer(user).is_some_and(|p| p.streams().iter().any(|s| s.half() && s.kind == kind))
}

async fn stall_loop(sfu: Arc<Sfu>) {
    loop {
        tokio::time::sleep(Duration::from_millis(STALL_MS)).await;
        let now = now_ms();
        let peers: Vec<Arc<Peer>> = sfu.peers.iter().map(|p| p.clone()).collect();
        for p in peers {
            stall_check(&sfu, &p, now);
        }
    }
}

async fn link_watch(sfu: Arc<Sfu>) {
    let mut last_keepalive = 0u64;
    loop {
        tokio::time::sleep(Duration::from_millis(LINK_WATCH_MS)).await;
        let now = now_ms();
        let peers: Vec<Arc<Peer>> = sfu.peers.iter().map(|p| p.clone()).collect();
        let keepalive = now.saturating_sub(last_keepalive) >= crate::link::transport::TCP_KEEPALIVE_MS;
        if keepalive {
            last_keepalive = now;
        }
        for p in peers {
            for c in &p.conns {
                if keepalive {
                    for id in c.link.live_tcp_pairs(now) {
                        if let Some(tx) = sfu.io.tcp.get(&id) {
                            let _ = tx.try_send(vec![0, 0]);
                        }
                    }
                }
                if let Some(ev) = c.link.liveness(now) {
                    let b = LinkEventBody { type_: LinkEventBodyType::Suspect, sfu_id: sfu.epoch.clone(), ufrag: ev.ufrag.clone() };
                    sfu.emit_user(&p.user, Op::LinkEvent, &b, None);
                    common::log::transition(&common::log::Step {
                        area: "LINK",
                        key: &p.user,
                        from: "Alive",
                        to: Some("Suspect"),
                        cause: "pair_expired",
                        label: Some("LINK.S.suspect"),
                        ins: &[("ufrag", ev.ufrag)],
                        room: None,
                        user: Some(&p.user),
                    });
                }
            }
            let gates: Vec<(String, Arc<crate::sub::Gate>)> = p.lock().gates.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (room, g) in gates {
                if g.timeout(now) {
                    sfu.drops.gate_timeout.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(user = p.user.as_str(), room = room.as_str(), "T-gate expired without READY — READY 미수신 의심 (no keyframe request on this path)");
                }
            }
        }
    }
}

async fn home_watch(sfu: Arc<Sfu>) {
    let grace = Duration::from_secs(sfu.cfg.node_grace_secs);
    loop {
        tokio::time::sleep(Duration::from_millis(LINK_WATCH_MS)).await;
        let peers: Vec<Arc<Peer>> = sfu.peers.iter().map(|p| p.clone()).collect();
        for p in peers {
            let h = p.home.load_full();
            if matches!(sfu.bus.home(&h.node, &h.inst, grace), crate::bus::HomeState::Lost) {
                tracing::warn!(user = p.user.as_str(), "home lost node={} inst={} — reaping", h.node, h.inst);
                let _g = sfu.rooms_op.lock().await;
                drop_peer(&sfu, &p, Why::HomeLost).await;
            }
        }
    }
}

async fn stats(sfu: Arc<Sfu>) {
    let ms = u64::from(sfu.cfg.policy.logging.stat_interval_ms).max(1000);
    loop {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        let conns = sfu.plane.conns().len();
        let send_fail = sfu.io.send_fail.load(Ordering::Relaxed);
        let put_drop = sfu.bus.put_drop.load(Ordering::Relaxed);
        common::stat!(rooms = sfu.rooms.len(), peers = sfu.peers.len(), conns = conns, send_fail = send_fail, bus_put_drop = put_drop, "sfud stat");
    }
}

pub fn start(sfu: &Arc<Sfu>) {
    tokio::spawn(room_sweep(sfu.clone()));
    tokio::spawn(stall_loop(sfu.clone()));
    tokio::spawn(link_watch(sfu.clone()));
    tokio::spawn(home_watch(sfu.clone()));
    tokio::spawn(stats(sfu.clone()));
}
