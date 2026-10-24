// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-SURFACE · 정:RULE-OPS-OBSERVE · 정:IMPL-OPS-DESTROY-REAP · 정:IMPL-ROOM-PERMISSION · 정:RULE-FLR-SNAPSHOT · 운영 규격서 §3-2 · 운영 규격서 §3-3 · 운영 규격서 §3-5 · 운영 규격서 §3-6 · 운영 규격서 §3-9 · 운영 규격서 §4-3 · 연:ROOM.S.perm_floor_down · 연:ROOM.S.perm_floor_up · 연:ROOM.S.perm_pub_down · 연:ROOM.S.perm_pub_up · 연:ROOM.S.perm_absent · 연:PUB.S.perm_down_revoke · 연:PUB.S.perm_down · 연:PUB.S.perm_up · 연:DEV-ROOM-HIDDEN-SELF-PSTATE · 연:RULE-ROOM-PSTATE-ORDER · model: claude-opus-5-5
//! 운영 물음 — 방 · 방 상세(정본 · hidden 은 수만) · 사용자 연결 · 사용자 미디어(배정표 줄 + 재기록기) · 버림 계수 · 폭파(확인값 `version`) · 한 사람 퇴장 · 권한 비트(적용 → 회수 → 통지).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::bus::envelope::{Admin, Reply};
use oxsig::{Code, Op, ParticipantStateBody, ParticipantStateBodyType, Permission};
use serde_json::{json, Value};

use super::{rooms, Sfu, World};
use crate::gc::{destroy_room, leave_seven, Path};
use crate::link::lock;
use crate::r#pub::core::{bit_for, Bit};
use crate::room::core::set_override;

pub async fn admin(sfu: &Arc<Sfu>, what: Admin) -> Reply {
    match what {
        Admin::Rooms => {
            let w = sfu.world.lock().await;
            let rows: Vec<Value> = w.rooms.values().map(|r| json!({"room_id": r.id, "members": r.members.len(), "member_ids": r.members.iter().map(|m| m.user.clone()).collect::<Vec<_>>(), "user_count": r.user_count()})).collect();
            Reply::Ok(json!({"rooms": rows, "total": w.rooms.len()}))
        }
        Admin::RoomSnapshot { room_id } => room_snapshot(sfu, &room_id).await,
        Admin::Users => {
            let w = sfu.world.lock().await;
            let rows: Vec<Value> = w.peers.values().map(|p| json!({"user_id": p.user, "session": p.session, "links": links(sfu, p)})).collect();
            Reply::Ok(json!({"users": rows}))
        }
        Admin::UserMedia { user_id } => user_media(sfu, &user_id).await,
        Admin::Drops => {
            let mut m = sfu.counters.snapshot();
            let c = &sfu.net.counters;
            for (k, v) in [
                ("stun_integrity_fail", &c.stun_integrity_fail),
                ("stun_unknown_ufrag", &c.stun_unknown_ufrag),
                ("stun_malformed", &c.stun_malformed),
                ("demux_unknown", &c.demux_unknown),
                ("unknown_path", &c.unknown_path),
                ("srtp_fail", &c.srtp_fail),
                ("tcp_unidentified", &c.tcp_unidentified),
                ("tcp_bad_frame", &c.tcp_bad_frame),
                ("send_fail", &c.send_fail),
                ("srtp_encrypt_fail", &c.srtp_encrypt_fail),
                ("no_path", &c.no_path),
            ] {
                m.insert(k.to_string(), Value::from(v.load(Ordering::Relaxed)));
            }
            m.insert("bus_put_dropped".into(), Value::from(sfu.bus.dropped.load(Ordering::Relaxed)));
            m.insert("unit".into(), Value::from(sfu.node.clone()));
            m.insert("sfu_id".into(), Value::from(sfu.epoch.clone()));
            Reply::Ok(Value::Object(m))
        }
        Admin::Destroy { room_id, if_version } => {
            let Some(want) = if_version else { return sfu.fail(Code::MissingField.failure_with("if_version", None)) };
            let n = {
                let mut w = sfu.world.lock().await;
                let Some(r) = w.rooms.get(&room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
                let cur = format!("{}:{}", sfu.epoch, r.seq);
                if want != cur {
                    return sfu.fail(Code::PreconditionFailed.failure_with("if_version 불일치", Some(json!({"current": cur}))));
                }
                destroy_room(sfu, &mut w, &room_id)
            };
            sfu.bus.undeclare_room(&room_id).await;
            tracing::info!(kind = "ev", spec = "OP-PATH-ADMIN-ROOMS-ROOM-ID-DESTROY", room = %room_id, notified = n, "room destroyed");
            Reply::Ok(json!({"room_id": room_id, "destroyed": true, "notified": n}))
        }
        Admin::Reap { room_id, user_id } => {
            let mut w = sfu.world.lock().await;
            if !w.rooms.contains_key(&room_id) {
                return sfu.fail(Code::RoomNotFound.failure());
            }
            if !w.rooms.get(&room_id).map(|r| r.has(&user_id)).unwrap_or(false) {
                return sfu.fail(Code::NotInRoom.failure());
            }
            leave_seven(sfu, &mut w, &room_id, &user_id, Path::Kick, false);
            let version = w.rooms.get(&room_id).map(|r| r.version(&sfu.epoch));
            tracing::info!(kind = "ev", spec = "OP-PATH-ADMIN-ROOMS-ROOM-ID-REAP-USER-ID", room = %room_id, user = %user_id, "user reaped from room");
            Reply::Ok(json!({"room_id": room_id, "user_id": user_id, "left": true, "version": version}))
        }
        Admin::SetPermission { room_id, user_id, permission } => set_permission(sfu, &room_id, &user_id, permission).await,
    }
}

fn links(sfu: &Sfu, p: &crate::aff::Peer) -> Vec<Value> {
    p.transports()
        .iter()
        .map(|t| {
            let since = t.suspect_since.load(Ordering::Relaxed);
            json!({"sfu_id": sfu.epoch, "role": t.role.as_str(), "ufrag": t.cred().ufrag, "state": if t.peer_state() == crate::link::core::PeerState::Alive { "Alive" } else { "Suspect" }, "since": if since > 0 { since } else { t.created_ms }, "dtls": t.dtls_up.load(Ordering::Relaxed), "dc": t.dc.open.load(Ordering::Relaxed)})
        })
        .collect()
}

async fn room_snapshot(sfu: &Arc<Sfu>, room_id: &str) -> Reply {
    let w = sfu.world.lock().await;
    let Some(r) = w.rooms.get(room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
    let hot = sfu.room_hot(room_id);
    let floor = hot.as_ref().map(|h| {
        let mut f = h.floor.snapshot();
        let last = h.floor.last_rtp_ms.load(Ordering::Relaxed);
        f["last_rtp_at"] = if last > 0 { json!(last) } else { Value::Null };
        f
    });
    let slots = hot.as_ref().map(|h| json!({"audio": h.slots[0].speaker_user(), "video": h.slots[1].speaker_user()}));
    Reply::Ok(json!({"room": {
        "room_id": r.id, "name": r.name, "capacity": r.capacity, "user_count": r.user_count(), "rec": r.rec(),
        "version": r.version(&sfu.epoch), "participants": r.participants(), "hidden_count": r.hidden_count(),
        "tracks": rooms::tracks_of(r, hot.as_deref()), "floor": floor, "slots": slots,
    }}))
}

async fn user_media(sfu: &Arc<Sfu>, user: &str) -> Reply {
    let w = sfu.world.lock().await;
    let Some(p) = w.peers.get(user).cloned() else { return Reply::Ok(json!({"links": []})) };
    let t = p.subt.clone();
    let table = lock(&t.table).clone();
    let mlines: Vec<Value> = table
        .rows
        .iter()
        .map(|row| {
            let d = t.down(row.ssrc);
            let rewrite = d.as_ref().and_then(|d| {
                let rw = lock(&d.rw);
                rw.last_out().map(|(ls, lt)| {
                    let src = Option::clone(&d.track.load()).map(|tr| json!({"user_id": tr.user, "track_id": tr.track_id}));
                    json!({"src": src, "seq_off": rw.seq_off(), "ts_off": rw.ts_off(), "last_seq": ls, "last_ts": lt, "last_at": d.last_egress_ms.load(Ordering::Relaxed), "egress": d.egress.load(Ordering::Relaxed)})
                })
            });
            json!({"entry": table.entry(row), "since": row.loaded_at, "rewrite": rewrite})
        })
        .collect();
    Reply::Ok(json!({"links": [{"sfu_id": sfu.epoch, "ufrag": t.cred().ufrag, "arev": table.arev, "acked": t.ack.load(Ordering::Relaxed), "mlines": mlines}]}))
}

async fn set_permission(sfu: &Arc<Sfu>, room_id: &str, user: &str, next: Permission) -> Reply {
    let mut w = sfu.world.lock().await;
    let Some(r) = w.rooms.get_mut(room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
    let Some(m) = r.member(user).cloned() else {
        tracing::info!(kind = "ev", spec = "ROOM.S.perm_absent", room = %room_id, user = %user, "permission for a non-member refused");
        return sfu.fail(Code::NotInRoom.failure());
    };
    let Some(change) = set_override(&mut r.overrides, user, m.permission, next) else {
        return Reply::Ok(json!({"room_id": room_id, "user_id": user, "changed": false, "permission": next}));
    };
    if let Some(mm) = r.member_mut(user) {
        mm.permission = next;
    }
    revoke(sfu, &mut w, room_id, user, &change);
    let hidden = m.hidden;
    let now = Sfu::now();
    if let Some(h) = sfu.room_hot(room_id) {
        let lowered_floor = change.lowered().contains(&Bit::Floor);
        let raised_floor = change.raised().contains(&Bit::Floor);
        if lowered_floor || raised_floor {
            let o = h.floor.perm(user, raised_floor, now);
            crate::fwd::floor_out(sfu, &h, o);
        }
    }
    let label = match (change.lowered().is_empty(), change.lowered().contains(&Bit::Floor) || change.raised().contains(&Bit::Floor)) {
        (false, true) => "ROOM.S.perm_floor_down",
        (true, true) => "ROOM.S.perm_floor_up",
        (false, false) => "ROOM.S.perm_pub_down",
        (true, false) => "ROOM.S.perm_pub_up",
    };
    let Some(r) = w.rooms.get_mut(room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
    if hidden {
        let body = ParticipantStateBody { type_: ParticipantStateBodyType::Permission, user_id: user.to_string(), room_id: room_id.to_string(), version: None, permission: next };
        sfu.to_user(user, Op::ParticipantState, serde_json::to_value(body).unwrap_or_default(), Some(room_id));
    } else {
        r.bump();
        let body = ParticipantStateBody { type_: ParticipantStateBodyType::Permission, user_id: user.to_string(), room_id: room_id.to_string(), version: Some(r.version(&sfu.epoch)), permission: next };
        sfu.to_room(room_id, Op::ParticipantState, serde_json::to_value(body).unwrap_or_default(), Vec::new());
    }
    tracing::info!(kind = "ev", spec = label, room = %room_id, user = %user, hidden, seq = r.seq, "permission changed — enforced before notify");
    Reply::Ok(json!({"room_id": room_id, "user_id": user, "changed": true, "permission": next, "version": r.version(&sfu.epoch)}))
}

fn revoke(sfu: &Sfu, w: &mut World, room_id: &str, user: &str, change: &crate::room::core::PermChange) {
    let lowered = change.lowered();
    if lowered.is_empty() {
        return;
    }
    let Some(peer) = w.peers.get(user).cloned() else { return };
    let victims: Vec<_> = peer.tracks().into_iter().filter(|t| !t.half.load(Ordering::Relaxed) && t.room() == room_id && lowered.contains(&bit_for(t.kind, t.source))).collect();
    for t in &victims {
        super::pubs::unlist_tracks(sfu, w, room_id, std::slice::from_ref(t));
        super::pubs::unpublish(sfu, &peer, t);
        tracing::info!(kind = "ev", spec = "PUB.S.perm_down_revoke", room = %room_id, user = %user, track = %t.track_id, "publication revoked by permission");
    }
    if victims.is_empty() {
        tracing::debug!(kind = "ev", spec = "PUB.S.perm_down", room = %room_id, user = %user, "permission lowered — nothing to revoke");
    }
}
