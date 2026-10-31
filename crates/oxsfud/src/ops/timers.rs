// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-GC-FOUR · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-STALL · 정:IMPL-LINK-PEER-STATE · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-BUS-HOME-LOST · 정:IMPL-SUB-READY-GATE · 운영:RULE-LOG-KIND · 정:CAND-BUS-HOME-INST · 정:CAND-ROOM-STALL-COOL-UNIT · model: claude-opus-5-5
//! 제어 런타임의 주기 작업 — room sweep 5s(TTL) · 정체 판정 5s(창은 시작점 뒤에서만 · 반이중은 화자 RTP 가 창 안에 있을 때만 · 쿨다운 (방, 사람) 30s) · 쌍별 생존 1s(경로 쌍이 죽으면 `Suspect` 와 `LINK_EVENT` 한 번) · 홈 잃음 · 게이트 시한 latch · stat 줄.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::Op;
use oxsig::msg::*;

use crate::fwd::LAYER_PAUSE;
use crate::gc::Why;
use crate::link::PeerState;
use crate::room::Room;

pub const SWEEP_MS: u64 = 5_000;
pub const STALL_MS: u64 = 5_000;
pub const STALL_COOL_MS: u64 = 30_000;
pub const LIVENESS_MS: u64 = 1_000;

pub fn start() {
    super::spawn(async {
        let mut t = tokio::time::interval(Duration::from_millis(SWEEP_MS));
        loop {
            t.tick().await;
            sweep().await;
        }
    });
    super::spawn(async {
        let mut t = tokio::time::interval(Duration::from_millis(STALL_MS));
        let mut win: HashMap<usize, (u64, u64)> = HashMap::new();
        loop {
            t.tick().await;
            stall(&mut win);
        }
    });
    super::spawn(async {
        let mut t = tokio::time::interval(Duration::from_millis(LIVENESS_MS));
        loop {
            t.tick().await;
            liveness();
            home_lost();
            gates();
        }
    });
    super::spawn(async {
        let ms = super::policy().logging.stat_interval_ms.max(1_000) as u64;
        let mut t = tokio::time::interval(Duration::from_millis(ms));
        loop {
            t.tick().await;
            stat();
        }
    });
}

async fn sweep() {
    let Some(s) = super::sfu() else { return };
    let now = common::now_ms();
    for r in s.rooms.all() {
        if r.registered() == 0 && r.ttl_due(now) {
            super::rooms::destroy_room(&r).await;
        }
    }
}

fn liveness() {
    let Some(s) = super::sfu() else { return };
    let Some(m) = crate::fwd::rt::media() else { return };
    let now = common::now_ms();
    for p in s.peers() {
        for tr in p.transports() {
            if tr.path.load().is_none() || tr.peer_state() == PeerState::Suspect {
                continue;
            }
            if tr.path_alive(now, |id| m.tcp_open(id)) {
                continue;
            }
            if tr.set_state(PeerState::Suspect, now) == Some(PeerState::Alive) {
                super::transition("LINK", "PeerState", "SV-STATE-PEER-STATE", "Alive", "Suspect", "pair_expired", &[("user", &p.user)]);
                let body = LinkEventBody { type_: LinkEventType::Suspect, sfu_id: s.epoch.clone(), ufrag: tr.ufrag() };
                super::user_notice(&p.user, Op::LinkEvent, &body, None);
            }
        }
    }
}

fn home_lost() {
    let Some(s) = super::sfu() else { return };
    let Some(b) = crate::bus::get() else { return };
    if b.isolated() {
        return;
    }
    let now = common::now_ms();
    for p in s.peers() {
        let h = p.home();
        if h.node.is_empty() {
            continue;
        }
        if let Some((false, at)) = b.home_alive(&h.node, &h.inst)
            && now.saturating_sub(at) >= s.node_grace_ms
        {
            tracing::info!(kind = "diag", user = %p.user, "home {}/{} lost — reaping peer", h.node, h.inst);
            crate::gc::evict(&p, None, Why::HomeLost);
        }
    }
}

fn gates() {
    let Some(s) = super::sfu() else { return };
    let now = common::now_ms();
    let tables: Vec<Arc<crate::sub::Table>> = s.tables.map.lock().map(|g| g.values().cloned().collect()).unwrap_or_default();
    for t in tables {
        for room in t.gate_timeouts(now) {
            tracing::warn!(kind = "diag", user = %t.conn.user, room = %room, "READY gate released by timeout — READY 미수신 의심 (no keyframe request on this path)");
        }
    }
}

fn mono_to_wall(mono_ms: u64) -> u64 {
    let off = common::now_ms().saturating_sub(super::floor::now_ms());
    mono_ms + off
}

fn stall(win: &mut HashMap<usize, (u64, u64)>) {
    let Some(s) = super::sfu() else { return };
    let now = common::now_ms();
    let mut seen = std::collections::HashSet::new();
    for room in s.rooms.all() {
        for u in room.users() {
            let Some(p) = s.peer(&u) else { continue };
            if p.subscribe.peer_state() != PeerState::Alive {
                continue;
            }
            let Some(t) = s.table(&p) else { continue };
            let Some(ready) = t.ready_at(&room.id) else { continue };
            for (tid, rid, row) in t.loaded() {
                if rid != room.id {
                    continue;
                }
                let key = Arc::as_ptr(&row) as usize;
                seen.insert(key);
                let Some(start) = eligible_from(&room, &tid, &row, &u, ready) else {
                    win.remove(&key);
                    continue;
                };
                let sent = row.sent.load(Ordering::Relaxed);
                let (s0, ws) = *win.entry(key).or_insert((sent, now));
                if ws < start {
                    win.insert(key, (sent, now));
                    continue;
                }
                if now.saturating_sub(ws) < STALL_MS {
                    continue;
                }
                let slot = row.slot;
                let speaker_spoke = !slot || {
                    let lr = room.slot(row.kind).last_rtp_us.load(Ordering::Acquire) / 1000;
                    lr > 0 && mono_to_wall(lr) > ws
                };
                if sent == s0 && speaker_spoke {
                    sync_required(&room, &u, now);
                }
                win.insert(key, (sent, now));
            }
        }
    }
    win.retain(|k, _| seen.contains(k));
}

fn eligible_from(room: &Room, tid: &str, row: &crate::fwd::DownRow, user: &str, ready: u64) -> Option<u64> {
    let s = super::sfu()?;
    if !row.open() || row.paused.load(Ordering::Acquire) {
        return None;
    }
    if row.slot {
        let fan = room.slot(row.kind);
        let sp = fan.speaker.load_full()?;
        if sp.as_str() == user {
            return None;
        }
        let ctx = s.pubs.get(&sp)?;
        let bound = ctx.streams().iter().any(|st| st.kind == row.kind && st.is_half() && st.room.load().as_str() == room.id);
        if !bound {
            return None;
        }
        let granted = super::floor::of(&room.id).and_then(|rf| rf.clock.lock().ok().map(|c| c.granted_at)).map(mono_to_wall).unwrap_or(0);
        return Some(ready.max(granted));
    }
    let st = s.stream(tid)?;
    if st.is_half() {
        return None;
    }
    let pub_room = s.peer(&st.user).and_then(|p| crate::aff::pub_room(&p))?;
    if !crate::fwd::on_pub_room(&st, &pub_room) || crate::fwd::row_gate(&st, row).is_some() {
        return None;
    }
    if row.kind == Kind::Video {
        if st.simulcast && row.layer.load(Ordering::Acquire) == LAYER_PAUSE && row.target.load(Ordering::Acquire) == LAYER_PAUSE {
            return None;
        }
        let first = st.first_rtp_ms.load(Ordering::Acquire);
        if first == 0 {
            return None;
        }
        return Some(ready.max(first));
    }
    Some(ready)
}

fn sync_required(room: &Room, user: &str, now: u64) {
    let fire = match room.stall_cool.lock() {
        Ok(mut g) => {
            let last = g.get(user).copied().unwrap_or(0);
            if last != 0 && now.saturating_sub(last) < STALL_COOL_MS {
                false
            } else {
                g.insert(user.to_string(), now);
                true
            }
        }
        Err(_) => false,
    };
    if !fire {
        return;
    }
    tracing::info!(kind = "diag", room = %room.id, user = %user, "stall detected — sync_required");
    let body = RoomEventBody::SyncRequired { room_id: room.id.clone(), reason: "stall".into() };
    super::user_notice(user, Op::RoomEvent, &body, Some(&room.id));
}

fn stat() {
    let Some(s) = super::sfu() else { return };
    let drops: u64 = s.drops.iter().map(|d| d.load(Ordering::Relaxed)).sum();
    let (mut count, mut sum) = (0u64, 0u64);
    if let Some(m) = crate::fwd::rt::media() {
        for w in &m.workers {
            count += w.transit.count.load(Ordering::Relaxed);
            sum += w.transit.sum_us.load(Ordering::Relaxed);
        }
    }
    let l = common::log::Line::new(common::log::Kind::Stat)
        .rec("peers", s.peers().len())
        .rec("rooms", s.rooms.all().len())
        .rec("drops", drops)
        .rec("transit_count", count)
        .rec("transit_sum_us", sum)
        .rec("dc_dropped", crate::aux::DC_DROPPED.load(Ordering::Relaxed))
        .msg("sfud");
    common::log::emit(l);
}
