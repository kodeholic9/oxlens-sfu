// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-OPS-SURFACE · 정:RULE-OPS-OBSERVE · 정:IMPL-OPS-DESTROY-REAP · 정:RULE-FLR-SNAPSHOT · 정:IMPL-ROOM-DERIVED · model: claude-opus-5-5
//! C 평면의 sfud 쪽 — 방 목록 · 방 상세(정본 · `hidden_count` · 발언권 타이머) · 사용자 미디어 연결(배정표 줄 · 재기록기) · 버림 계수 · `reap` · `destroy`(판 값 확인).

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use common::handle::Reply;
use oxsig::err::{ErrCode, Failure};
use oxsig::room::AffiliationCause;
use serde_json::{json, Value};

use crate::cfg::Ctx;
use crate::link::{lock, Peer};

fn not_found() -> Reply {
    Reply::Http { status: 404, body: json!({"error": "not found"}) }
}

pub fn admin(ctx: &Ctx, method: &str, path: &str, query: &BTreeMap<String, String>, _body: &Value) -> Reply {
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["admin", "rooms"]) => {
            let rooms: Vec<Value> = ctx.rooms.all().iter().map(|r| json!({"room_id": r.id, "members": r.summary().user_count, "member_ids": r.member_ids()})).collect();
            let total = rooms.len();
            Reply::ok(json!({"rooms": rooms, "total": total}))
        }
        ("GET", ["admin", "rooms", id, "snapshot"]) => room_snapshot(ctx, id),
        ("GET", ["admin", "users", user, "media"]) => match ctx.links.peer(user) {
            Some(p) => Reply::ok(json!({"links": [media(ctx, &p)]})),
            None => Reply::fail(Failure::new(ErrCode::PeerNotFound)),
        },
        ("GET", ["admin", "links", user]) => match ctx.links.peer(user) {
            Some(p) => Reply::ok(json!({"links": brief(ctx, &p)})),
            None => Reply::ok(json!({"links": []})),
        },
        ("GET", ["admin", "drops"]) => Reply::ok(ctx.drops.view()),
        ("POST", ["admin", "rooms", id, "reap", user]) => reap(ctx, id, user),
        ("POST", ["admin", "rooms", id, "destroy"]) => destroy(ctx, id, query),
        _ => not_found(),
    }
}

fn room_snapshot(ctx: &Ctx, id: &str) -> Reply {
    let Some(r) = ctx.rooms.get(id) else { return Reply::fail(Failure::new(ErrCode::RoomNotFound)) };
    let (participants, user_count, rec, hidden) = {
        let g = lock(&r.inner);
        (g.participants(), g.user_count(), g.rec(), g.members.values().filter(|m| m.hidden).count())
    };
    let (tracks, version) = ctx.tracks.snapshot(&r);
    Reply::ok(json!({"room": {
        "room_id": r.id, "name": r.name, "capacity": r.capacity,
        "user_count": user_count, "rec": rec, "version": version,
        "participants": participants, "hidden_count": hidden, "tracks": tracks,
        "floor": r.floor.snapshot(),
        "slots": {"audio": r.audio.speaker(), "video": r.video.speaker()},
    }}))
}

fn media(ctx: &Ctx, p: &Peer) -> Value {
    let t = lock(&p.table);
    let downs = lock(&p.downs);
    let mut rows: Vec<&crate::link::table::Row> = t.rows.iter().collect();
    rows.sort_by_key(|r| r.mid);
    let mlines: Vec<Value> = rows
        .iter()
        .map(|r| {
            let rw = downs.get(&r.mid).and_then(|d| {
                let st = lock(&d.st);
                let last = st.rw.last?;
                Some(json!({
                    "src": st.src_user.as_ref().map(|u| json!({"user_id": u, "track_id": st.src_track})),
                    "seq_off": st.rw.seq_off, "ts_off": st.rw.ts_off,
                    "last_seq": last.seq, "last_ts": last.ts, "last_at": d.last_at.load(Ordering::Acquire),
                    "switches": st.rw.switches,
                }))
            });
            json!({"entry": r.entry(), "since": r.since_ms, "rewrite": rw})
        })
        .collect();
    json!({"sfu_id": ctx.boot.epoch, "ufrag": p.sub_transport().ufrag(), "arev": t.arev, "acked": t.ack, "mlines": mlines})
}

fn brief(ctx: &Ctx, p: &Peer) -> Vec<Value> {
    p.transports()
        .iter()
        .map(|t| {
            json!({
                "sfu_id": ctx.boot.epoch, "role": format!("{:?}", t.role).to_lowercase(), "ufrag": t.ufrag(),
                "state": t.peer_state().name(), "dtls": t.has_dtls(), "dc": lock(&t.dc).open,
                "rx": t.rx_packets.load(Ordering::Relaxed), "tx": t.tx_packets.load(Ordering::Relaxed),
                "rooms": p.room_list(), "pub_room": p.pub_room(),
            })
        })
        .collect()
}

fn reap(ctx: &Ctx, id: &str, user: &str) -> Reply {
    let Some(r) = ctx.rooms.get(id) else { return Reply::fail(Failure::new(ErrCode::RoomNotFound)) };
    let Some(p) = ctx.links.peer(user).filter(|p| p.in_room(id)) else { return Reply::Http { status: 404, body: json!({"room_id": id, "user_id": user, "left": false}) } };
    crate::gc::forced(ctx, &r, &p, AffiliationCause::Kick);
    Reply::ok(json!({"room_id": id, "user_id": user, "left": true, "version": r.version()}))
}

fn destroy(ctx: &Ctx, id: &str, q: &BTreeMap<String, String>) -> Reply {
    let Some(r) = ctx.rooms.get(id) else { return Reply::fail(Failure::new(ErrCode::RoomNotFound)) };
    let Some(want) = q.get("if_version") else { return Reply::fail(Failure::new(ErrCode::MissingField).msg("if_version")) };
    let v = r.version();
    let have = format!("{}:{}", v.epoch, v.seq);
    if want != &have {
        return Reply::fail(Failure::new(ErrCode::PreconditionFailed).details(json!({"expected": want, "actual": have})));
    }
    let n = crate::gc::destroy(ctx, &r);
    Reply::ok(json!({"room_id": id, "destroyed": true, "notified": n}))
}
