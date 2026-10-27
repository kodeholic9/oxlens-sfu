// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-OPS-SURFACE · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-OPS-DROPS · 정:RULE-FLR-SNAPSHOT · 정:IMPL-AFF-FORCED · 정:CAND-BUS-HANDLE-SHAPE · 운영:RULE-READ-USERS · 운영:RULE-READ-DROPS · 운영:RULE-READ-ROOM-DETAIL · 운영:RULE-READ-USER-MEDIA · 운영:RULE-CTL-ROOM · 운영:RULE-FMT-NO-INVENT · model: claude-opus-5-5
//! C 평면의 sfud 몫 — 게이트웨이가 `Call::Admin` 봉투로 넘긴 것: 연결(`links`) · 버림 계수 · 방 상세(정본) · 미디어 연결(배정표 · 재기록기) · 한 사람 퇴장(`reap`) · 즉시 폭파(`destroy` — 확인값 = `version`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::bus::env::Reply;
use oxsig::code::{Code, Failure};
use oxsig::op::Op;
use serde_json::{Value, json};

use super::floor::Exec;
use super::{Sfu, now_ms};

fn wall_of(mono: u64) -> u64 {
    let now_m = now_ms();
    let wall = common::time::wall_ms();
    if mono >= now_m { wall + (mono - now_m) } else { wall.saturating_sub(now_m - mono) }
}

fn http(status: u16, code: Code) -> Reply {
    Reply::Http { status, body: serde_json::to_value(Failure::of(code)).unwrap_or_default() }
}

pub fn handle(sfu: &Arc<Sfu>, method: &str, path: &str, query: &BTreeMap<String, String>, _body: Option<&Value>, exec: &mut Exec) -> Reply {
    let seg: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, seg.as_slice()) {
        ("GET", ["admin", "users", u, "links"]) => links(sfu, u),
        ("GET", ["admin", "drops"]) => {
            let mut b = super::drops::DROPS.body();
            b["bus_ev_drop"] = json!(sfu.bus.dropped.load(Ordering::Relaxed) + super::drops::DROPS.bus_ev_drop.load(Ordering::Relaxed));
            Reply::Ok { body: b }
        }
        ("GET", ["admin", "rooms", r, "snapshot"]) => snapshot(sfu, r),
        ("GET", ["admin", "users", u, "media"]) => media(sfu, u),
        ("POST", ["admin", "rooms", r, "reap", u]) => reap(sfu, r, u, exec),
        ("POST", ["admin", "rooms", r, "destroy"]) => destroy(sfu, r, query.get("if_version").map(String::as_str), exec),
        _ => http(404, Code::UNKNOWN_OP),
    }
}

fn links(sfu: &Sfu, user: &str) -> Reply {
    let Ok(st) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let Some(p) = st.peers.get(user) else { return Reply::Ok { body: json!({"links": []}) } };
    let rows: Vec<Value> = p
        .transports()
        .iter()
        .map(|t| {
            let state = t.state();
            let since = match state {
                crate::link::PeerState::Suspect => t.suspect_since.load(Ordering::Acquire),
                crate::link::PeerState::Alive => t.created_ms,
            };
            json!({"sfu_id": sfu.epoch, "ufrag": t.ufrag(), "state": state.as_str(), "since": wall_of(since), "role": t.role.as_str()})
        })
        .collect();
    Reply::Ok { body: json!({"links": rows}) }
}

fn snapshot(sfu: &Sfu, room: &str) -> Reply {
    let Ok(st) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let Some(r) = st.rooms.get(room) else { return http(404, Code::ROOM_NOT_FOUND) };
    let last = r.hot.audio.speaker_last_ms.load(Ordering::Acquire).max(r.hot.video.speaker_last_ms.load(Ordering::Acquire));
    let f = &r.floor;
    let timers: Vec<Value> = f
        .timers(last)
        .into_iter()
        .map(|(name, user, at)| match user {
            Some(u) => json!({"name": name, "user_id": u, "at": wall_of(at)}),
            None => json!({"name": name, "at": wall_of(at)}),
        })
        .collect();
    let q: Vec<Value> = f.srv.queue.iter().map(|w| json!({"user_id": w.u, "priority": w.p})).collect();
    let first = r.hot.audio.speaker_first_ms.load(Ordering::Acquire);
    let floor = json!({
        "state": f.srv.g.as_str(),
        "speaker": f.srv.holder(),
        "priority": if f.srv.g == crate::flr::core::G::Taken { f.srv.sprio } else { f.srv.rprio },
        "queue": q,
        "first_rtp_at": (first != 0).then(|| wall_of(first)),
        "last_rtp_at": (last != 0).then(|| wall_of(last)),
        "timers": timers,
    });
    let slot = |k: oxsig::types::Kind| r.hot.slot(k).bound.load().as_ref().map(|b| b.user.to_string());
    let hidden = r.members.values().filter(|m| m.hidden).count();
    Reply::Ok {
        body: json!({"room": {
            "room_id": r.id,
            "name": r.name,
            "capacity": r.capacity,
            "user_count": r.user_count(),
            "rec": r.rec(),
            "version": r.version(),
            "participants": r.participants(),
            "hidden_count": hidden,
            "tracks": r.track_entries(),
            "floor": floor,
            "slots": {"audio": slot(oxsig::types::Kind::Audio), "video": slot(oxsig::types::Kind::Video)},
        }}),
    }
}

fn media(sfu: &Sfu, user: &str) -> Reply {
    let Ok(st) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let Some(p) = st.peers.get(user) else { return Reply::Ok { body: json!({"sfu_id": sfu.epoch, "error": Code::PEER_NOT_FOUND.0}) } };
    let wire = p.table.wire();
    let mut lines = Vec::new();
    for e in wire.mlines {
        let mid: u8 = e.mid.parse().unwrap_or(0);
        let row = p.table.rows().iter().find(|r| r.mid == mid);
        let rewrite = row.and_then(|r| {
            let rw = r.dl.rewrite.lock().ok()?;
            let last = rw.rw.last_seq()?;
            let src = rw.src.as_ref().and_then(|w| w.upgrade()).and_then(|u| u.stream.upgrade()).map(|s| json!({"user_id": &*s.user, "track_id": &*s.track_id}));
            Some(json!({"src": src, "seq_off": rw.rw.seq_off(), "ts_off": rw.rw.ts_off(), "last_seq": last, "last_ts": rw.rw.last_ts(), "last_at": rw.rw.last_at_ms().map(wall_of), "switches": rw.rw.switches()}))
        });
        let since = row.map(|r| wall_of(r.dl.carry_at_ms.load(Ordering::Acquire))).unwrap_or(0);
        lines.push(json!({"entry": e, "since": since, "rewrite": rewrite}));
    }
    Reply::Ok { body: json!({"sfu_id": sfu.epoch, "ufrag": p.sub_transport().ufrag(), "arev": p.table.arev, "acked": p.table.ack, "mlines": lines}) }
}

fn reap(sfu: &Arc<Sfu>, room: &str, user: &str, exec: &mut Exec) -> Reply {
    let Ok(mut guard) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let st = &mut *guard;
    if !st.rooms.contains_key(room) {
        return http(404, Code::ROOM_NOT_FOUND);
    }
    if !st.peers.get(user).is_some_and(|p| p.rooms.contains(room)) {
        return http(404, Code::NOT_IN_ROOM);
    }
    let hidden = st.rooms.get(room).and_then(|r| r.members.get(user)).is_some_and(|m| m.hidden);
    let sub_after: Vec<String> = st.peers.get(user).map(|p| p.rooms.iter().filter(|r| *r != room).cloned().collect()).unwrap_or_default();
    let pub_after = st.peers.get(user).and_then(|p| p.pub_room.clone()).filter(|r| r != room);
    let ev = oxsig::msg::RoomEventBody::Affiliation { room_id: room.to_string(), affiliation: oxsig::types::Affiliation { sub_rooms: sub_after, pub_room: pub_after }, cause: oxsig::msg::RoomEventBodyAffiliationCause::Kick };
    sfu.notice_user(user, Op::RoomEvent, serde_json::to_value(&ev).unwrap_or_default());
    let label = if hidden { "ROOM.S.kick_hidden" } else { "ROOM.S.kick" };
    crate::gc::leave_seven(sfu, st, room, user, crate::gc::Path::Kick, label, exec);
    let v = st.rooms.get(room).map(|r| r.version());
    Reply::Ok { body: json!({"room_id": room, "user_id": user, "left": true, "version": v}) }
}

fn destroy(sfu: &Arc<Sfu>, room: &str, if_version: Option<&str>, exec: &mut Exec) -> Reply {
    let Some(want) = if_version else { return http(400, Code::MISSING_FIELD) };
    let Ok(mut guard) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let st = &mut *guard;
    let Some(r) = st.rooms.get(room) else { return http(404, Code::ROOM_NOT_FOUND) };
    if super::version_str(&r.version()) != want {
        return http(409, Code::PRECONDITION_FAILED);
    }
    let members: Vec<String> = r.members.keys().cloned().collect();
    for u in &members {
        let sub_after: Vec<String> = st.peers.get(u).map(|p| p.rooms.iter().filter(|x| *x != room).cloned().collect()).unwrap_or_default();
        let pub_after = st.peers.get(u).and_then(|p| p.pub_room.clone()).filter(|x| x != room);
        let ev = oxsig::msg::RoomEventBody::Affiliation { room_id: room.to_string(), affiliation: oxsig::types::Affiliation { sub_rooms: sub_after, pub_room: pub_after }, cause: oxsig::msg::RoomEventBodyAffiliationCause::RoomClosed };
        sfu.notice_user(u, Op::RoomEvent, serde_json::to_value(&ev).unwrap_or_default());
    }
    for u in &members {
        crate::gc::leave_seven(sfu, st, room, u, crate::gc::Path::Destroy, "ROOM.S.close_notify", exec);
    }
    crate::gc::drop_room(sfu, st, room);
    common::ev!(spec = "ROOM.S.close", area = "ROOM", key = %room, from = "open", to = "closed", cause = "destroy", room = %room);
    Reply::Ok { body: json!({"room_id": room, "destroyed": true, "notified": members.len()}) }
}
