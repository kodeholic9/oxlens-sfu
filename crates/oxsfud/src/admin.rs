// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-OPS-SURFACE 정:IMPL-OPS-DESTROY-REAP 정:RULE-FLR-SNAPSHOT 정:RULE-OPS-OBSERVE 운영 규격서 §3-3 §3-5 §3-6 §4-3 §4-4 · model: claude-opus-5-5
//! C 평면 — 방 · 사용자 · 버림 계수 · 방 상세(정본) · 강제 퇴장 · 폭파 · 로그 수준.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::bplane::{AdminLink, AdminReq, AdminRoom, AdminUser, UnitRes};
use oxsig::wire::Kind;
use serde_json::json;

use crate::sfu::{Sfu, lock};

pub fn handle(sfu: &Arc<Sfu>, a: AdminReq) -> UnitRes {
    match a {
        AdminReq::Rooms => {
            let rooms: Vec<Arc<crate::room::Room>> = lock(&sfu.rooms).values().cloned().collect();
            UnitRes::AdminRooms(
                rooms
                    .iter()
                    .map(|r| {
                        let g = r.lock();
                        AdminRoom { room_id: r.id.clone(), members: g.members.len() as u32, member_ids: g.members.keys().cloned().collect() }
                    })
                    .collect(),
            )
        }
        AdminReq::RoomSnapshot { room_id } => match sfu.room(&room_id) {
            Some(r) => UnitRes::AdminJson(room_snapshot(sfu, &r)),
            None => UnitRes::Fail { code: oxsig::code::Code::ROOM_NOT_FOUND.0, message: "room not found".into(), details: None },
        },
        AdminReq::Users => {
            let peers: Vec<Arc<crate::peer::Peer>> = lock(&sfu.peers).values().cloned().collect();
            UnitRes::AdminUsers(
                peers
                    .iter()
                    .map(|p| AdminUser {
                        session_id: p.session_id.clone(),
                        user_id: p.user_id.clone(),
                        links: p
                            .conns()
                            .iter()
                            .map(|c| AdminLink {
                                sfu_id: sfu.epoch.clone(),
                                ufrag: c.ufrag(),
                                state: if c.is_alive() { "Alive".into() } else { "Suspect".into() },
                                since: c.state_since.load(Ordering::Relaxed),
                            })
                            .collect(),
                    })
                    .collect(),
            )
        }
        AdminReq::Drops => {
            let mut v = serde_json::Map::new();
            for (k, n) in lock(&sfu.counters.drops).iter() {
                v.insert((*k).to_string(), json!(n));
            }
            let dc: u64 = lock(&sfu.peers).values().map(|p| p.pub_conn.dc.dropped.load(Ordering::Relaxed)).sum();
            v.insert("dc_dropped".into(), json!(dc));
            if let Some(c) = common::log::counters() {
                v.insert("log_dropped".into(), json!(c.dropped.load(Ordering::Relaxed)));
                v.insert("log_spooled".into(), json!(c.spooled.load(Ordering::Relaxed)));
            }
            v.insert("codes".into(), sfu.counters.snapshot()["codes"].clone());
            UnitRes::AdminJson(serde_json::Value::Object(v))
        }
        AdminReq::Reap { room_id, user_id } => match crate::session::kick(sfu, &room_id, &user_id) {
            Ok(r) => UnitRes::Reaped(r),
            Err((c, m)) => UnitRes::Fail { code: c.0, message: m, details: None },
        },
        AdminReq::Destroy { room_id, if_version } => match crate::session::destroy(sfu, &room_id, if_version.as_deref()) {
            Ok(r) => UnitRes::Destroyed(r),
            Err((c, m, d)) => UnitRes::Fail { code: c.0, message: m, details: d },
        },
        AdminReq::LogLevel { level, ttl_secs } => match common::log::set_level(&level, ttl_secs) {
            Ok((from, to)) => UnitRes::AdminJson(json!({ "from": from, "to": to })),
            Err(e) => UnitRes::Fail { code: oxsig::code::Code::INVALID_PAYLOAD.0, message: e, details: None },
        },
    }
}

fn room_snapshot(sfu: &Sfu, r: &crate::room::Room) -> serde_json::Value {
    let (participants, hidden, tracks, user_count, rec) = {
        let g = r.lock();
        (r.visible(&g), g.members.values().filter(|m| m.hidden).count(), r.tracks(&g), r.user_count(&g), r.rec(&g))
    };
    let floor = crate::floor::snapshot(r);
    let state = {
        let fr = lock(&r.floor);
        format!("{:?}", fr.srv.g)
    };
    let slot = |k: Kind| r.slot(k).bound.load().as_ref().map(|s| s.track_id.clone());
    json!({
        "room": {
            "room_id": r.id, "name": r.name, "capacity": r.capacity,
            "user_count": user_count, "rec": rec,
            "version": r.version(sfu),
            "participants": participants,
            "hidden_count": hidden,
            "tracks": tracks,
            "floor": { "state": state, "speaker": floor["speaker"], "queue": floor["queue"], "started_at": floor["started_at"] },
            "slots": { "audio": slot(Kind::Audio), "video": slot(Kind::Video) },
        }
    })
}
