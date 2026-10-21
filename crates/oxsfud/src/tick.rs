// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 정:RULE-GC-FOUR 정:IMPL-ROOM-STALL 정:IMPL-RTCP-TERMINATE 정:RULE-LINK-PAIR-LIVENESS 정:IMPL-FLR-RTP-OBSERVE 정:RULE-BUS-FATE · model: claude-opus-5-5
//! 주기 — room sweep 5s · 발언권 2s · 정체 5s · RTCP(RR/REMB 1s · TWCC 100ms · NACK) · 생존 1s · TCP keepalive 20s.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::Op;
use oxsig::wire::{Kind, RoomEventBody};

use crate::room::Room;
use crate::sfu::{Sfu, lock, mono_ms};
use crate::table::RowClass;

const STALL_MS: u64 = 5_000;
const STALL_COOL_MS: u64 = 30_000;

fn every(sfu: &Arc<Sfu>, ms: u64, f: fn(&Arc<Sfu>)) {
    let s = sfu.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(ms));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            iv.tick().await;
            f(&s);
        }
    });
}

fn rooms(sfu: &Sfu) -> Vec<Arc<Room>> {
    lock(&sfu.rooms).values().cloned().collect()
}

pub fn spawn_all(sfu: &Arc<Sfu>) {
    every(sfu, 20, |s| {
        let now = mono_ms();
        for r in rooms(s) {
            crate::media::slot_tick(s, &r, now);
        }
        crate::layer::probe_tick(s);
    });
    every(sfu, 50, |s| crate::rtcp_io::nack_tick(s));
    every(sfu, 100, |s| crate::rtcp_io::twcc_tick(s));
    every(sfu, 1_000, |s| {
        s.beat.store(common::ident::now_ms(), Ordering::Relaxed);
        crate::rtcp_io::report_tick(s);
        crate::layer::tick(s);
        crate::net::liveness(s);
        crate::table::tgate_sweep(s, mono_ms());
        common::log::restore_if_due();
        let peers: Vec<Arc<crate::peer::Peer>> = lock(&s.peers).values().cloned().collect();
        for p in peers {
            if crate::table::waiting_audio(&p.sub_conn) {
                crate::table::fill(s, &p, true);
            }
        }
    });
    every(sfu, 2_000, |s| {
        for r in rooms(s) {
            crate::floor::tick(s, &r);
        }
    });
    every(sfu, 5_000, |s| {
        crate::room::sweep(s);
        stall(s);
    });
    every(sfu, crate::net::TCP_KEEPALIVE_MS, |s| crate::net::tcp_keepalive(s));
}

fn stall(sfu: &Arc<Sfu>) {
    let now = mono_ms();
    let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let conn = p.sub_conn.clone();
        if !conn.is_alive() {
            continue;
        }
        let ready = lock(&p.ready).clone();
        let rows = lock(&conn.table).rows.clone();
        for row in rows {
            let Some(room_id) = row.room_id() else { continue };
            let Some(first_ready) = ready.get(&room_id).map(|r| r.first_at).filter(|t| *t > 0) else { continue };
            let Some(room) = sfu.room(&room_id) else { continue };
            let (eligible, start) = match row.class {
                RowClass::Slot => {
                    let holder = room.holder.load_full();
                    let bound = room.slot(row.kind).bound.load_full();
                    let ok = holder.as_deref().is_some_and(|h| h.as_str() != p.user_id) && bound.is_some();
                    (ok, first_ready)
                }
                RowClass::Full => match row.stream() {
                    Some(s) => {
                        let first = s.first_rtp_at.load(Ordering::Relaxed);
                        let video = s.kind == Kind::Video;
                        let start = if video { first_ready.max(first) } else { first_ready };
                        let paused = crate::table::pref(&conn, &s.track_id).paused || (s.simulcast && conn.auto_layer.load(Ordering::Relaxed) == 2);
                        let ok = !s.is_half() && !s.gone.load(Ordering::Relaxed) && !paused && (!video || first > 0) && (!video || row.open.load(Ordering::Relaxed));
                        (ok, start)
                    }
                    None => (false, 0),
                },
            };
            let eligible = eligible && row.since.load(Ordering::Relaxed) <= conn.ack.load(Ordering::Relaxed) && now >= start;
            let egress = row.egress.load(Ordering::Relaxed);
            let mut st = lock(&row.stall);
            if !eligible || st.0 == 0 {
                st.0 = now;
                st.1 = egress;
                continue;
            }
            if now.saturating_sub(st.0) < STALL_MS {
                continue;
            }
            let stalled = egress == st.1;
            st.0 = now;
            st.1 = egress;
            drop(st);
            if stalled {
                notify_stall(sfu, &room, &p.user_id, now);
            }
        }
    }
}

fn notify_stall(sfu: &Sfu, room: &Room, user: &str, now: u64) {
    {
        let mut c = lock(&room.cool);
        if c.get(user).is_some_and(|t| now.saturating_sub(*t) < STALL_COOL_MS) {
            return;
        }
        c.insert(user.to_string(), now);
    }
    tracing::info!(kind = "ev", spec = "IMPL-ROOM-STALL", room = room.id.as_str(), user = user, "sync_required");
    sfu.emit_user(user, Op::RoomEvent, &RoomEventBody::SyncRequired { room_id: room.id.clone(), reason: "no_media_flow".into() });
}
