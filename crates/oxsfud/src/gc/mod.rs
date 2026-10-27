// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-GC-FOUR · 정:RULE-GC-LEAVE-SEVEN · 정:IMPL-GC-PEER-REAP · 정:RULE-GC-EDGES · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-STALL · 정:IMPL-BUS-HOME-LOST · 정:IMPL-AFF-FORCED · 정:CAND-ROOM-JOIN-AGAIN-EDGE · 정:CAND-BUS-HOME-INST · 연:ROOM.S.join_again · 연:ROOM.S.peer_evicted · 연:ROOM.S.session_gone · 연:LINK.S.peer_reaped · 연:ROOM.S.sync_required · model: claude-opus-5-5
//! 회수 — 퇴장 · 강제 퇴장(강퇴 · 폭파 · 재입장 축출) · 세션 폐기가 한 절차(일곱 단계 · 차례가 계약)로 수렴한다. Peer 를 없애는 사건은 셋(세션 폐기 · 마지막 방 · 홈 잃음). 빈 방 sweep(5s · TTL) · 정체 감지(5s) · 홈 잃음(node 토큰 · grace).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::{ParticipantEventBody, ParticipantEventBodyType, RoomEventBody};
use oxsig::op::Op;
use oxsig::types::Kind;
use zenoh::sample::SampleKind;

use crate::ops::floor::Exec;
use crate::ops::{Sfu, State, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Leave,
    Kick,
    Destroy,
    SessionGone,
    Evict,
    HomeLost,
}

impl Path {
    pub fn as_str(self) -> &'static str {
        match self {
            Path::Leave => "leave",
            Path::Kick => "kick",
            Path::Destroy => "destroy",
            Path::SessionGone => "session_gone",
            Path::Evict => "evict",
            Path::HomeLost => "home_lost",
        }
    }
}

pub fn leave_seven(sfu: &Sfu, st: &mut State, room: &str, user: &str, path: Path, label: &str, exec: &mut Exec) -> bool {
    let now = now_ms();
    if let Some(r) = st.rooms.get_mut(room) {
        let s = r.floor.step(now, |s, e| crate::flr::core::ws_leave(s, e, user));
        exec.floor(room, r.hot.clone(), "ws_leave", Some(user), s);
    }
    let member = st.rooms.get_mut(room).and_then(|r| r.members.remove(user));
    let Some(member) = member else {
        tracing::warn!(room, user, path = path.as_str(), "회수 대상 명단에 없음");
        return false;
    };
    let had_rows = st.peers.get_mut(user).map(|p| p.table.off_room(room)).unwrap_or_default();
    crate::ops::call::apply_changes(st, user, &had_rows);
    let pub_here = st.peers.get(user).is_some_and(|p| p.pub_room.as_deref() == Some(room));
    if pub_here {
        if let Some(p) = st.peers.get_mut(user) {
            p.pub_room = None;
            p.aff.pub_room.store(None);
        }
        common::ev!(spec = "AFF.S.left", area = "AFF", key = %user, from = %room, to = "-", cause = path.as_str(), user = %user);
    }
    let last = match st.peers.get_mut(user) {
        Some(p) => {
            p.rooms.remove(room);
            p.gates.remove(room);
            p.first_ready.remove(room);
            p.rooms.is_empty()
        }
        None => false,
    };
    let state = st.peers.get(user).map(|p| p.publish.state().as_str()).unwrap_or("-");
    if last {
        reap_peer(sfu, st, user, path, exec);
    } else if !had_rows.is_empty()
        && let Some(p) = st.peers.get(user)
    {
        sfu.mline_event(p);
    }
    let mine: Vec<crate::trk::Item> = st.rooms.get(room).map(|r| r.tracks.values().filter(|t| t.user_id == user).cloned().collect()).unwrap_or_default();
    if !mine.is_empty() {
        if let Some(r) = st.rooms.get_mut(room) {
            for i in &mine {
                r.tracks.remove(&i.track_id);
            }
            let v = r.bump();
            let body = oxsig::msg::TrackEventBody { action: oxsig::msg::PublishTracksReqAddAction::Remove, room_id: room.to_string(), tracks: mine.iter().map(|i| i.removed(room)).collect(), version: v };
            sfu.notice_room(room, Op::TrackEvent, serde_json::to_value(&body).unwrap_or_default(), Vec::new());
        }
        for i in &mine {
            let others: Vec<String> = st.rooms.get(room).map(|r| r.members.keys().cloned().collect()).unwrap_or_default();
            for u in others {
                if crate::ops::call::rows_off(st, &u, &i.track_id, room)
                    && let Some(p) = st.peers.get(&u)
                {
                    sfu.mline_event(p);
                }
            }
        }
    }
    if !member.hidden
        && path != Path::Destroy
        && let Some(r) = st.rooms.get_mut(room)
    {
        let v = r.bump();
        let ev = ParticipantEventBody { type_: ParticipantEventBodyType::Left, room_id: room.to_string(), user_id: user.to_string(), role: None, select: None, participant_type: None, metadata: None, permission: None, version: v };
        sfu.notice_room(room, Op::ParticipantEvent, serde_json::to_value(&ev).unwrap_or_default(), vec![user.to_string()]);
    }
    let seq = st.rooms.get(room).map(|r| r.seq).unwrap_or(0);
    if path == Path::Destroy {
        common::ev!(spec = label, area = "ROOM", key = %room, from = "member", to = "-", cause = path.as_str(), room = %room, user = %user, seq = seq, peer_state = state);
    } else {
        common::ev!(spec = label, area = "ROOM", key = %room, from = "member", to = "-", cause = path.as_str(), room = %room, user = %user, seq = seq, in.hidden = member.hidden, peer_state = state);
    }
    true
}

pub fn reap_peer(sfu: &Sfu, st: &mut State, user: &str, path: Path, exec: &mut Exec) {
    let Some(p) = st.peers.remove(user) else { return };
    st.arev.insert(user.to_string(), p.table.arev);
    for s in p.streams.values() {
        s.subs.store(Arc::new(Vec::new()));
    }
    for r in p.table.rows() {
        r.dl.carry.store(None);
        for room in st.rooms.values() {
            room.hot.slot(r.kind).detach(&r.dl);
        }
        for q in st.peers.values() {
            for s in q.streams.values() {
                s.detach(&r.dl);
            }
        }
    }
    for t in p.transports() {
        sfu.io.unregister(&t);
        exec.close.push(t);
    }
    common::ev!(spec = "LINK.S.peer_reaped", area = "LINK", key = %user, from = p.publish.state().as_str(), to = "-", cause = path.as_str(), user = %user, in.last = true);
    common::ev!(spec = "PUB.S.peer_gone", area = "PUB", key = %user, from = "registered", to = "-", cause = path.as_str(), user = %user);
}

pub fn evict(sfu: &Sfu, st: &mut State, user: &str, keep: Option<&str>, exec: &mut Exec) {
    let now = now_ms();
    let (rooms, hidden) = match st.peers.get(user) {
        Some(p) => (p.rooms.iter().filter(|r| Some(r.as_str()) != keep).cloned().collect::<Vec<_>>(), p.ctx.hidden),
        None => return,
    };
    for r in rooms {
        let label = if hidden { "ROOM.S.peer_evicted_hidden" } else { "ROOM.S.peer_evicted" };
        leave_seven(sfu, st, &r, user, Path::Evict, label, exec);
    }
    let Some(k) = keep else { return };
    if let Some(r) = st.rooms.get_mut(k) {
        let s = r.floor.step(now, |s, e| crate::flr::core::ws_leave(s, e, user));
        exec.floor(k, r.hot.clone(), "ws_leave", Some(user), s);
    }
    let mine: Vec<crate::trk::Item> = st.rooms.get(k).map(|r| r.tracks.values().filter(|t| t.user_id == user).cloned().collect()).unwrap_or_default();
    if !mine.is_empty() {
        if let Some(r) = st.rooms.get_mut(k) {
            for i in &mine {
                r.tracks.remove(&i.track_id);
            }
            let v = r.bump();
            let body = oxsig::msg::TrackEventBody { action: oxsig::msg::PublishTracksReqAddAction::Remove, room_id: k.to_string(), tracks: mine.iter().map(|i| i.removed(k)).collect(), version: v };
            sfu.notice_room(k, Op::TrackEvent, serde_json::to_value(&body).unwrap_or_default(), Vec::new());
        }
        let others: Vec<String> = st.rooms.get(k).map(|r| r.members.keys().filter(|u| *u != user).cloned().collect()).unwrap_or_default();
        for i in &mine {
            for u in &others {
                if crate::ops::call::rows_off(st, u, &i.track_id, k)
                    && let Some(p) = st.peers.get(u)
                {
                    sfu.mline_event(p);
                }
            }
        }
    }
    common::ev!(spec = "ROOM.S.peer_evicted_self", area = "ROOM", key = %k, from = "member", to = "member", cause = "rejoin", room = %k, user = %user);
    reap_peer(sfu, st, user, Path::Evict, exec);
}

pub fn session_gone(sfu: &Arc<Sfu>, user: &str, session: &str, code: u16, exec: &mut Exec) {
    let Ok(mut guard) = sfu.state.lock() else { return };
    let st = &mut *guard;
    let Some(p) = st.peers.get(user) else { return };
    if p.fp != session {
        return;
    }
    let hidden = p.ctx.hidden;
    let rooms: Vec<String> = p.rooms.iter().cloned().collect();
    let label = if hidden { "ROOM.S.session_gone_hidden" } else { "ROOM.S.session_gone" };
    tracing::info!(user, code, "세션 폐기");
    for r in rooms {
        leave_seven(sfu, st, &r, user, Path::SessionGone, label, exec);
    }
    if st.peers.contains_key(user) {
        reap_peer(sfu, st, user, Path::SessionGone, exec);
    }
}

pub fn drop_room(sfu: &Sfu, st: &mut State, room: &str) {
    if st.rooms.remove(room).is_some() {
        for p in st.peers.values_mut() {
            p.rooms.remove(room);
        }
        sfu.bus.undeclare_room(room);
        crate::ops::floor::wake(sfu, room);
    }
}

pub async fn sweep_rooms(sfu: &Arc<Sfu>) {
    let now = now_ms();
    let Ok(mut guard) = sfu.state.lock() else { return };
    let st = &mut *guard;
    let mut gone = Vec::new();
    for (id, r) in st.rooms.iter_mut() {
        match r.sweep(now) {
            crate::room::Sweep::Expire => gone.push(id.clone()),
            crate::room::Sweep::MarkEmpty => tracing::info!(room = %id, "빈 방 첫 관측"),
            crate::room::Sweep::Keep => {}
        }
    }
    for id in gone {
        drop_room(sfu, st, &id);
        common::ev!(spec = "ROOM.S.close", area = "ROOM", key = %id, from = "open", to = "closed", cause = "ttl", room = %id);
    }
}

pub fn stall(sfu: &Arc<Sfu>, now: u64) {
    let Ok(mut guard) = sfu.state.lock() else { return };
    let st = &mut *guard;
    let mut alerts: Vec<(String, String)> = Vec::new();
    for p in st.peers.values() {
        if p.sub_transport().state() != crate::link::PeerState::Alive {
            continue;
        }
        for r in p.table.rows() {
            let dl = &r.dl;
            let Some((track, room)) = r.track.clone() else { continue };
            let Some(first) = p.first_ready.get(&room).copied() else { continue };
            let first_mono = now.saturating_sub(common::time::wall_ms().saturating_sub(first));
            let eligible = dl.sendable.load(Ordering::Acquire) && !dl.paused.load(Ordering::Acquire);
            let mut start = first_mono;
            let mut active_src = true;
            if dl.slot {
                let Some(rm) = st.rooms.get(&room) else { continue };
                let slot = rm.hot.slot(dl.kind);
                match slot.bound.load().as_ref() {
                    Some(b) if *b.user != *p.user => {
                        start = start.max(b.at_ms);
                        active_src = slot.speaker_last_ms.load(Ordering::Acquire) > dl.stall_from.load(Ordering::Acquire);
                    }
                    _ => active_src = false,
                }
            } else if dl.kind == Kind::Video {
                let s = crate::ops::call::stream_of(st, &track);
                let first_rtp = s.and_then(|s| s.uplinks.load().iter().map(|u| u.first_rtp_ms.load(Ordering::Acquire)).filter(|x| *x != 0).min());
                match first_rtp {
                    Some(f) => start = start.max(f),
                    None => active_src = false,
                }
            }
            let pkts = dl.egress_pkts.load(Ordering::Relaxed);
            let from = dl.stall_from.load(Ordering::Acquire);
            if !eligible || !active_src || from == 0 || from < start {
                dl.stall_from.store(now.max(start), Ordering::Release);
                dl.stall_pkts.store(pkts, Ordering::Release);
                continue;
            }
            if now.saturating_sub(from) >= crate::room::STALL_MS {
                if pkts == dl.stall_pkts.load(Ordering::Acquire) {
                    alerts.push((room.clone(), p.user.clone()));
                }
                dl.stall_from.store(now, Ordering::Release);
                dl.stall_pkts.store(pkts, Ordering::Release);
            }
        }
    }
    for (room, user) in alerts {
        let ok = st.rooms.get_mut(&room).is_some_and(|r| r.stall_allowed(&user, now));
        if ok {
            common::ev!(spec = "ROOM.S.sync_required", area = "ROOM", key = %room, from = "-", to = "cool", cause = "stall", room = %room, user = %user);
            let body = RoomEventBody::SyncRequired { room_id: room.clone(), reason: "stalled".into() };
            sfu.notice_user(&user, Op::RoomEvent, serde_json::to_value(&body).unwrap_or_default());
        }
    }
}

pub async fn watch_home(sfu: Arc<Sfu>) {
    let Ok(sub) = sfu.bus.watch_nodes().await else {
        tracing::warn!("node 토큰 구독 실패");
        return;
    };
    let mut alive: HashSet<(String, String)> = HashSet::new();
    let mut ever: HashSet<String> = HashSet::new();
    let mut gone: HashMap<(String, String), u64> = HashMap::new();
    let grace = sfu.cfg.node_grace_secs * 1_000;
    let mut iv = tokio::time::interval(std::time::Duration::from_millis(1_000));
    loop {
        tokio::select! {
            s = sub.recv_async() => {
                let Ok(s) = s else { break };
                let Some(t) = sfu.bus.ks.parse_node(s.key_expr().as_str()) else { continue };
                let k = (t.node.clone(), t.inst.clone());
                ever.insert(t.node.clone());
                if s.kind() == SampleKind::Put {
                    gone.remove(&k);
                    alive.insert(k);
                } else {
                    alive.remove(&k);
                    gone.insert(k, now_ms());
                }
            }
            _ = iv.tick() => {
                let now = now_ms();
                let others_alive = alive.iter().any(|(n, _)| *n != sfu.cfg.node_id);
                if !others_alive || ever.len() <= 2 {
                    continue;
                }
                let due: Vec<(String, String)> = gone.iter().filter(|(_, at)| now.saturating_sub(**at) >= grace).map(|(k, _)| k.clone()).collect();
                for k in due {
                    gone.remove(&k);
                    reap_home(&sfu, &k).await;
                }
            }
        }
    }
}

async fn reap_home(sfu: &Arc<Sfu>, home: &(String, String)) {
    let mut exec = Exec::default();
    {
        let Ok(mut guard) = sfu.state.lock() else { return };
        let st = &mut *guard;
        let users: Vec<String> = st.peers.values().filter(|p| p.home == *home).map(|p| p.user.clone()).collect();
        for u in users {
            let rooms: Vec<String> = st.peers.get(&u).map(|p| p.rooms.iter().cloned().collect()).unwrap_or_default();
            let hidden = st.peers.get(&u).is_some_and(|p| p.ctx.hidden);
            let label = if hidden { "ROOM.S.session_gone_hidden" } else { "ROOM.S.session_gone" };
            for r in rooms {
                leave_seven(sfu, st, &r, &u, Path::HomeLost, label, &mut exec);
            }
            if st.peers.contains_key(&u) {
                reap_peer(sfu, st, &u, Path::HomeLost, &mut exec);
            }
        }
    }
    exec.run(sfu).await;
}
