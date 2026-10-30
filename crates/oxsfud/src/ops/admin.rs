// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-OPS-SURFACE · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-OPS-DROPS · 정:RULE-OPS-TRANSIT · 정:RULE-FLR-SNAPSHOT · 운영:RULE-CTL-LOG-LEVEL · 정:IMPL-AFF-FORCED · 연:RULE-AFF-NO-FORCED-IN-RES · 연:RULE-ROOM-ROOM-EVENT-PARTIAL · model: claude-opus-5-5
//! 운영 경로(sfud 몫) — 연결 · 배정표 · 버림 · 체류 · 방 상세 · 방 목록 · `reap` · `destroy`(확인값 = 그 방 `version`) · 로그 수준 일시 덮기.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::envelope::Envelope;
use common::policy::LoggingLevel;
use oxsig::msg::*;
use oxsig::{Code, Failure, Op};
use serde_json::{Value, json};

use crate::fwd::rt::{TRANSIT_NAMES, media};
use crate::gc::Why;
use crate::link::Peer;

fn sfu() -> Result<&'static Arc<super::Sfu>, Failure> {
    super::sfu().ok_or_else(|| Code::InternalError.failure())
}

pub fn handle(env: &Envelope) -> Result<Value, Failure> {
    let method = env.method.as_deref().unwrap_or("GET");
    let path = env.path.as_deref().unwrap_or("");
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["admin", "drops"]) => Ok(super::drops_json()),
        ("GET", ["admin", "transit"]) => Ok(transit()),
        ("GET", ["admin", "rooms"]) => rooms(),
        ("GET", ["admin", "rooms", r, "snapshot"]) => room_snapshot(r),
        ("GET", ["admin", "users", u, "links"]) => links(u),
        ("GET", ["admin", "users", u, "media"]) => user_media(u),
        ("POST", ["admin", "rooms", r, "reap", u]) => reap(r, u),
        ("POST", ["admin", "rooms", r, "destroy"]) => destroy(r, &env.body),
        _ => Err(Code::UnknownOp.failure().with_message(format!("{method} {path}"))),
    }
}

fn transit() -> Value {
    let mut o = serde_json::Map::new();
    let mut b = [0u64; 8];
    let (mut count, mut sum) = (0u64, 0u64);
    if let Some(m) = media() {
        for w in &m.workers {
            for (i, x) in w.transit.buckets.iter().enumerate() {
                b[i] += x.load(Ordering::Relaxed);
            }
            count += w.transit.count.load(Ordering::Relaxed);
            sum += w.transit.sum_us.load(Ordering::Relaxed);
        }
    }
    for (i, n) in TRANSIT_NAMES.iter().enumerate() {
        o.insert(n.to_string(), json!(b[i]));
    }
    o.insert("count".into(), json!(count));
    o.insert("sum_us".into(), json!(sum));
    Value::Object(o)
}

fn rooms() -> Result<Value, Failure> {
    let s = sfu()?;
    let mut v: Vec<Value> = s
        .rooms
        .all()
        .iter()
        .map(|r| json!({ "room_id": r.id, "members": r.registered() }))
        .collect();
    v.sort_by(|a, b| a["room_id"].as_str().cmp(&b["room_id"].as_str()));
    Ok(json!({ "rooms": v }))
}

fn room_snapshot(rid: &str) -> Result<Value, Failure> {
    let s = sfu()?;
    let room = s.rooms.get(rid).ok_or_else(|| Code::RoomNotFound.failure())?;
    let slot = |k: Kind| json!(room.slot(k).speaker.load_full().map(|x| x.as_str().to_string()));
    Ok(json!({ "room": {
        "room_id": room.id,
        "name": room.name,
        "capacity": room.capacity,
        "user_count": room.user_count(),
        "rec": room.rec(),
        "version": room.version(),
        "participants": room.participants(),
        "hidden_count": room.hidden_count(),
        "tracks": super::rooms::tracks_of(&room),
        "floor": super::floor::snapshot(&room),
        "slots": { "audio": slot(Kind::Audio), "video": slot(Kind::Video) },
    }}))
}

fn link_rows(s: &super::Sfu, p: &Peer) -> Vec<Value> {
    p.transports()
        .iter()
        .map(|t| json!({ "sfu_id": s.epoch, "ufrag": t.ufrag(), "state": t.peer_state().as_str(), "since": t.state_since.load(Ordering::Acquire) }))
        .collect()
}

fn links(u: &str) -> Result<Value, Failure> {
    let s = sfu()?;
    let p = s.peer(u).ok_or_else(|| Code::PeerNotFound.failure())?;
    Ok(json!({ "links": link_rows(s, &p) }))
}

fn user_media(u: &str) -> Result<Value, Failure> {
    let s = sfu()?;
    let p = s.peer(u).ok_or_else(|| Code::PeerNotFound.failure())?;
    let Some(t) = s.table(&p) else { return Ok(json!({ "links": [] })) };
    let snap = t.snapshot(&s.epoch);
    let mlines: Vec<Value> = {
        let st = t.st.lock().map_err(|_| Code::InternalError.failure())?;
        st.rows
            .iter()
            .map(|r| {
                let rw = r.down.rw.lock().ok().and_then(|w| {
                    let l = w.last?;
                    let src = r.down.src_track.load_full().map(|x| json!({ "user_id": x.stream.user, "track_id": x.stream.track_id }));
                    Some(json!({ "src": src, "seq_off": w.seq_off, "ts_off": w.ts_off, "last_seq": l.seq, "last_ts": l.ts, "last_at": w.last_out_at, "switches": w.switches }))
                });
                json!({ "entry": r.entry(), "since": r.loaded_at, "rewrite": rw })
            })
            .collect()
    };
    Ok(json!({ "links": [{
        "sfu_id": s.epoch,
        "ufrag": p.subscribe.ufrag(),
        "arev": snap.arev,
        "acked": t.acked.load(Ordering::Acquire),
        "mlines": mlines,
    }]}))
}

fn forced(p: &Arc<Peer>, room: &str, cause: AffiliationEventCause) {
    let a = crate::aff::affiliation(p);
    let body = RoomEventBody::Affiliation { room_id: room.to_string(), affiliation: a, cause };
    super::user_notice(&p.user, Op::RoomEvent, &body, Some(room));
}

fn reap(rid: &str, u: &str) -> Result<Value, Failure> {
    let s = sfu()?;
    let room = s.rooms.get(rid).ok_or_else(|| Code::RoomNotFound.failure())?;
    let p = s.peer(u).filter(|p| p.in_room(rid)).ok_or_else(|| Code::NotInRoom.failure())?;
    let left = crate::gc::leave(&p, rid, Why::Kick, false);
    forced(&p, rid, AffiliationEventCause::Kick);
    Ok(json!({ "room_id": rid, "user_id": u, "left": left, "version": room.version() }))
}

fn destroy(rid: &str, body: &Value) -> Result<Value, Failure> {
    let s = sfu()?;
    let room = s.rooms.get(rid).ok_or_else(|| Code::RoomNotFound.failure())?;
    let want = body.get("if_version").and_then(|v| v.as_str()).ok_or_else(|| Code::MissingField.failure().with_message("if_version"))?;
    let v = room.version();
    let have = format!("{}:{}", v.epoch, v.seq);
    if want != have {
        return Err(Code::PreconditionFailed.failure().with_details(json!({ "expected": want, "actual": have })));
    }
    let mut notified = 0u32;
    for u in room.users() {
        if let Some(p) = s.peer(&u) {
            crate::gc::leave(&p, rid, Why::Destroy, false);
            forced(&p, rid, AffiliationEventCause::RoomClosed);
            notified += 1;
        } else {
            room.remove(&u);
        }
    }
    let r = room.clone();
    super::spawn(async move { super::rooms::destroy_room(&r).await });
    Ok(json!({ "room_id": rid, "destroyed": true, "notified": notified }))
}

pub fn log_level(env: &Envelope) -> Result<Value, Failure> {
    let lvl = env
        .body
        .get("level")
        .and_then(|v| v.as_str())
        .and_then(LoggingLevel::parse)
        .ok_or_else(|| Code::InvalidPayload.failure().with_message("level"))?;
    let until = env.body.get("until").and_then(|v| v.as_u64());
    let l = common::log::global().ok_or_else(|| Code::InternalError.failure())?;
    let from = l.level(common::now_ms());
    l.set_level(lvl, until);
    Ok(json!({ "from": from.as_str(), "to": lvl.as_str(), "until": until }))
}
