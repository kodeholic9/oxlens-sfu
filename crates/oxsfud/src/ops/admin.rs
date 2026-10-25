// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-OPS-SURFACE 정:IMPL-OPS-DESTROY-REAP 정:RULE-OPS-OBSERVE 정:RULE-FLR-SNAPSHOT 운영:RULE-READ-ROOM-DETAIL 운영:RULE-READ-NODE-ROOMS 운영:RULE-READ-USER-MEDIA 운영:RULE-READ-USERS 운영:RULE-READ-DROPS 운영:RULE-CTL-ROOM 운영:RULE-RISK-CONFIRM · model: claude-opus-5-5
//! 운영 물음의 sfud 쪽 정본 — 방 배치 명단 · 버림 계수 · 방 상세(투명 수 · 발언권 판단 입력 · 슬롯) · 그 노드의 방 · 사용자 미디어(배정표 줄 + 재기록기) · 연결 상태 · 강제 퇴장 · 폭파(확인값).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::Code;
use oxsig::envelope::{AdminCall, Reply};
use serde::Serialize;
use serde_json::{Value, json};

use super::Sfu;
use crate::fwd::shell::now_ms;

#[derive(Serialize)]
struct NodeRoom {
    room_id: String,
    member_ids: Vec<String>,
}

#[derive(Serialize)]
struct LinkRow {
    sfu_id: String,
    ufrag: String,
    state: String,
    since: u64,
}

fn rewrite_of(e: &crate::fwd::Egress, sfu: &Sfu) -> Value {
    let Ok(rw) = e.rw.lock() else { return Value::Null };
    let Some(last) = rw.rewriter.last else { return Value::Null };
    let src = e.load.load_full().and_then(|l| match &l.user {
        Some(u) => Some(json!({ "user_id": u, "track_id": l.track_id })),
        None => sfu.plane.room(&l.room).and_then(|rm| rm.slot(e.kind).bound.load_full()).map(|b| json!({ "user_id": b.user, "track_id": l.track_id })),
    });
    json!({
        "src": src,
        "seq_off": rw.rewriter.seq_off,
        "ts_off": rw.rewriter.ts_off,
        "last_seq": last.seq,
        "last_ts": last.ts,
        "last_at": last.at_ms,
        "switches": rw.rewriter.switches,
    })
}

pub async fn handle(sfu: &Arc<Sfu>, req: AdminCall) -> Reply {
    match req {
        AdminCall::Rooms => {
            let v: Vec<NodeRoom> = sfu.rooms.iter().map(|r| NodeRoom { room_id: r.id.clone(), member_ids: r.lock().members.keys().cloned().collect() }).collect();
            Reply::ok(&v)
        }
        AdminCall::Drops => {
            let mut m = sfu.drops.map();
            m.insert("send_fail".into(), sfu.io.send_fail.load(Ordering::Relaxed));
            m.insert("tcp_queue_full".into(), sfu.io.tcp_drop.load(Ordering::Relaxed));
            m.insert("bus_put_drop".into(), sfu.bus.put_drop.load(Ordering::Relaxed));
            m.insert("log_queue_full".into(), sfu.log.counters().queue_full.load(Ordering::Relaxed));
            m.insert("log_spool_full".into(), sfu.log.counters().spool_full.load(Ordering::Relaxed));
            let dc: u64 = sfu.peers.iter().map(|p| p.dc_drop.load(Ordering::Relaxed)).sum();
            m.insert("dc_overflow".into(), dc);
            Reply::ok(&m)
        }
        AdminCall::RoomSnapshot { room_id } => {
            let Some(room) = sfu.room(&room_id) else { return Reply::fail(Code::RoomNotFound) };
            let st = room.lock();
            let rm = sfu.plane.room(&room_id);
            let speaker = |k: oxsig::msg::Kind| rm.as_ref().and_then(|r| r.slot(k).bound.load_full()).map(|b| b.user.clone());
            let body = json!({
                "room": {
                    "room_id": room.id,
                    "name": room.name,
                    "capacity": room.capacity,
                    "user_count": st.user_count(),
                    "rec": st.rec(),
                    "version": room.version(st.seq),
                    "participants": st.participants(),
                    "hidden_count": st.hidden_count(),
                    "tracks": crate::trk::all(&room, &st),
                    "floor": room.floor.view(),
                    "slots": { "audio": speaker(oxsig::msg::Kind::Audio), "video": speaker(oxsig::msg::Kind::Video) },
                }
            });
            Reply::ok(&body)
        }
        AdminCall::SfuRooms => {
            let mut rooms: Vec<Value> = sfu.rooms.iter().map(|r| json!({ "room_id": r.id, "members": r.lock().members.len() })).collect();
            rooms.sort_by(|a, b| a["room_id"].as_str().cmp(&b["room_id"].as_str()));
            Reply::ok(&json!({ "unit": sfu.cfg.node_id, "sfu_id": sfu.epoch, "total": rooms.len(), "rooms": rooms }))
        }
        AdminCall::UserMedia { user_id } => {
            let Some(p) = sfu.peer(&user_id) else { return Reply::fail(Code::PeerNotFound) };
            let rows = p.lock().table.snapshot();
            let conn = p.sub_conn();
            let egress = conn.egress.load_full();
            let mlines: Vec<Value> = rows
                .0
                .iter()
                .map(|r| {
                    let e = egress.iter().find(|e| e.mid == r.mid);
                    json!({ "entry": r.entry(), "since": r.at_ms, "rewrite": e.map(|e| rewrite_of(e, sfu)).unwrap_or(Value::Null) })
                })
                .collect();
            let v = vec![json!({
                "sfu_id": sfu.epoch,
                "ufrag": conn.link.ufrag(),
                "arev": rows.1,
                "acked": conn.acked.load(Ordering::Acquire),
                "mlines": mlines,
            })];
            Reply::ok(&v)
        }
        AdminCall::Links { users } => {
            let mut out: BTreeMap<String, Vec<LinkRow>> = BTreeMap::new();
            for u in users {
                let Some(p) = sfu.peer(&u) else { continue };
                let rows = p
                    .conns
                    .iter()
                    .map(|c| {
                        let (s, since) = c.link.state();
                        LinkRow { sfu_id: sfu.epoch.clone(), ufrag: c.link.ufrag(), state: s.name().into(), since }
                    })
                    .collect();
                out.insert(u, rows);
            }
            Reply::ok(&out)
        }
        AdminCall::Reap { room_id, user_id } => {
            let Some(room) = sfu.room(&room_id) else { return Reply::fail(Code::RoomNotFound) };
            let Some(p) = sfu.peer(&user_id).filter(|p| p.aff.has(&room_id)) else { return Reply::fail(Code::NotInRoom) };
            let left = crate::gc::kick(sfu, &p, &room).await;
            let v = room.version(room.lock().seq);
            Reply::ok(&json!({ "room_id": room_id, "user_id": user_id, "left": left, "version": v }))
        }
        AdminCall::Destroy { room_id, if_version } => {
            let Some(room) = sfu.room(&room_id) else { return Reply::fail(Code::RoomNotFound) };
            let Some(want) = if_version else { return Reply::failure(Code::MissingField.failure_msg("if_version")) };
            let cur = room.version(room.lock().seq);
            let ok = crate::room::parse_version(&want).is_some_and(|(e, s)| e == cur.epoch && s == cur.seq);
            if !ok {
                return Reply::failure(Code::PreconditionFailed.failure().with_details(json!({ "current": format!("{}:{}", cur.epoch, cur.seq) })));
            }
            let _g = sfu.rooms_op.lock().await;
            let n = crate::gc::close_room(sfu, &room).await;
            common::ev!(spec = "SV-STATE-UNIT-STATE", room = room_id.as_str(), "room destroyed notified={n} at={}", now_ms());
            Reply::ok(&json!({ "room_id": room_id, "destroyed": true, "notified": n }))
        }
    }
}
