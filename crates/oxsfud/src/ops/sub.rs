// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SUB-PIPELINE 정:IMPL-SUB-TABLE 정:IMPL-SUB-READY-GATE 정:IMPL-PUB-RID-SPATIAL 정:IMPL-FLR-FIRST-NOTICE 정:RULE-FWD-STEPS 연:SUB.S.ready 연:LINK.S.ack 연:LINK.S.ack_old 연:RULE-SUB-READY-PROC 연:RULE-SUB-LAYER-PROC 연:RULE-SUB-EGRESS-ACKED · model: claude-opus-5-5
//! 구독 조율 — 배정표의 줄을 다운링크 스트림(Egress)으로 잇기(발행측 목록에 `Weak` attach · 슬롯 목록) · READY(게이트 · 방 단위 키프레임 · 처음 알리기 · 확인된 차례) · 레이어 상한.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::Code;
use oxsig::envelope::{Ctx, Reply};
use oxsig::msg::{Kind, SubscribeLayerReq};
use serde_json::Value;

use super::Sfu;
use super::call::body_json;
use super::peer::Peer;
use crate::fwd::shell::now_ms;
use crate::fwd::{Egress, LoadRef};
use crate::sub::table::Class;
use crate::sub::{Prefs, Ready};

fn detach(sfu: &Sfu, e: &Arc<Egress>, old: &LoadRef) {
    match &old.user {
        Some(u) => {
            if let Some(p) = sfu.peer(u) {
                for i in p.ingress_of_track(&old.track_id) {
                    i.detach(e);
                }
            }
        }
        None => {
            if let Some(rm) = sfu.plane.room(&old.room) {
                rm.slot(e.kind).detach(e);
            }
        }
    }
}

pub fn detach_all(sfu: &Sfu, peer: &Peer) {
    for c in &peer.conns {
        for e in c.egress.load().iter() {
            if let Some(l) = e.load.load_full() {
                detach(sfu, e, &l);
            }
        }
        c.set_egress(Vec::new());
    }
}

pub fn sync(sfu: &Arc<Sfu>, peer: &Arc<Peer>) {
    let _serial = peer.sync_lock.lock().unwrap_or_else(|p| p.into_inner());
    let conn = peer.sub_conn().clone();
    let cur = conn.egress.load_full();
    let now = now_ms();
    let (rows, gates, prefs) = {
        let st = peer.lock();
        (st.table.rows.clone(), st.gates.clone(), st.prefs.clone())
    };
    let mut next = Vec::with_capacity(rows.len());
    for row in &rows {
        let e = cur.iter().find(|e| e.mid == row.mid).cloned().unwrap_or_else(|| Arc::new(Egress::new(Arc::downgrade(&conn), &peer.user, row)));
        e.pt.store(row.pt, Ordering::Relaxed);
        e.rtx_pt.store(row.rtx_pt.unwrap_or(0), Ordering::Relaxed);
        e.since.store(row.since, Ordering::Release);
        let old = e.load.load_full();
        let same = match (&old, &row.load) {
            (Some(o), Some(n)) => o.room == n.room && o.track_id == n.track_id,
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(o) = &old {
                detach(sfu, &e, o);
            }
            match &row.load {
                None => e.load.store(None),
                Some(l) => {
                    let stream = l.user.as_ref().and_then(|u| sfu.peer(u)).and_then(|p| p.lock().pubs.by_id(&l.track_id));
                    e.load.store(Some(Arc::new(LoadRef { room: l.room.clone(), track_id: l.track_id.clone(), user: l.user.clone(), stream })));
                    e.loaded_at.store(now, Ordering::Release);
                    e.stall_from.store(0, Ordering::Release);
                }
            }
        }
        if let Some(l) = &row.load {
            match row.class {
                Class::Slot => {
                    if let Some(rm) = sfu.plane.room(&l.room) {
                        rm.slot(row.kind).attach(&e);
                    }
                }
                Class::Full => {
                    if let Some(u) = &l.user
                        && let Some(p) = sfu.peer(u)
                    {
                        for i in p.ingress_of_track(&l.track_id) {
                            i.attach(&e);
                        }
                    }
                }
            }
            if row.kind == Kind::Video && row.class == Class::Full {
                e.gate.store(gates.get(&l.room).cloned());
            } else {
                e.gate.store(None);
            }
            e.prefs.store(prefs.get(&(l.room.clone(), l.track_id.clone())).cloned());
        } else {
            e.gate.store(None);
            e.prefs.store(None);
        }
        next.push(e);
    }
    conn.set_egress(next);
}

pub fn keyframes_for_room(sfu: &Arc<Sfu>, room: &str, me: &str) {
    let now = now_ms();
    let Some(r) = sfu.room(room) else { return };
    let listed: Vec<(String, String)> =
        r.lock().tracks.iter().filter(|l| l.kind == Kind::Video && l.active && l.user != me).map(|l| (l.user.clone(), l.track_id.clone())).collect();
    for (u, t) in listed {
        if let Some(p) = sfu.peer(&u) {
            for i in p.ingress_of_track(&t) {
                crate::rtcp::shell::pli(sfu, &i, now, true);
            }
        }
    }
    if let Some(rm) = sfu.plane.room(room)
        && let Some(b) = rm.video.bound.load_full()
        && b.user != me
        && let Some(p) = sfu.peer(&b.user)
    {
        for s in p.streams().iter().filter(|s| s.half() && s.kind == Kind::Video) {
            for i in p.ingress_of_track(&s.track_id) {
                crate::rtcp::shell::pli(sfu, &i, now, true);
            }
        }
    }
}

pub async fn ready(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let v = match body_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let room_id = match super::call::need_str(&v, "room_id") {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(arev) = v.get("arev").and_then(Value::as_u64) else {
        return Reply::failure(Code::MissingField.failure_msg("arev"));
    };
    let Some(room) = sfu.room(&room_id) else { return Reply::fail(Code::RoomNotFound) };
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.aff.has(&room_id)) else { return Reply::fail(Code::NotInRoom) };
    let _op = peer.op.lock().await;
    let now = now_ms();
    let first = match peer.gate(&room_id) {
        Some(g) => matches!(g.ready(now), Ready::First),
        None => false,
    };
    let conn = peer.sub_conn();
    let prev = conn.acked.fetch_max(arev, Ordering::AcqRel);
    let label = if arev > prev { "LINK.S.ack" } else { "LINK.S.ack_old" };
    common::log::transition(&common::log::Step {
        area: "SUB",
        key: &room_id,
        from: if first { "gated" } else { "ready" },
        to: Some("ready"),
        cause: "READY",
        label: Some("SUB.S.ready"),
        ins: &[("arev", arev.to_string()), ("ack", label.to_string())],
        room: Some(&room_id),
        user: Some(&ctx.user_id),
    });
    keyframes_for_room(sfu, &room_id, &ctx.user_id);
    let step = room.floor.ready(&ctx.user_id, now);
    super::floor::exec(sfu, &room, step).await;
    Reply::ok(&serde_json::json!({}))
}

pub async fn subscribe_layer(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let req: SubscribeLayerReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Reply::fail(Code::InvalidPayload),
    };
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.aff.has(&req.room_id)) else { return Reply::fail(Code::NotInRoom) };
    let Some(room) = sfu.room(&req.room_id) else { return Reply::fail(Code::NotInRoom) };
    let _op = peer.op.lock().await;
    let now = now_ms();
    let mut unpaused: Vec<String> = Vec::new();
    let mut reconsider = false;
    for t in &req.targets {
        let Some(l) = room.lock().track(&t.track_id).cloned() else { continue };
        let key = (req.room_id.clone(), t.track_id.clone());
        let prefs = peer.lock().prefs.entry(key).or_insert_with(|| Arc::new(Prefs::default())).clone();
        if let Some(sp) = t.spatial {
            prefs.spatial.store(Prefs::clamp_spatial(sp), Ordering::Relaxed);
        }
        if let Some(tp) = t.temporal {
            prefs.temporal.store(Prefs::clamp_temporal(tp), Ordering::Relaxed);
        }
        if let Some(pr) = t.priority {
            let before = prefs.priority.swap(pr, Ordering::Relaxed);
            if before != pr {
                peer.lock().table.set_score(&req.room_id, &t.track_id, u64::from(pr));
                reconsider = true;
            }
        }
        if let Some(p) = t.paused {
            let before = prefs.paused.swap(p, Ordering::Relaxed);
            if before != p {
                if p {
                    let room_id = req.room_id.clone();
                    let id = t.track_id.clone();
                    super::publish::table_change(sfu, &peer, |tb| tb.unwant(&room_id, &id, now));
                } else {
                    if let Some(w) = super::publish::want_of(sfu, &room, &l, &peer, now) {
                        super::publish::table_change(sfu, &peer, |tb| tb.want(w, now));
                    }
                    unpaused.push(t.track_id.clone());
                }
            }
        }
    }
    if reconsider {
        super::publish::table_change(sfu, &peer, |tb| {
            let waiting = tb.waiting.clone();
            for w in waiting {
                tb.want(w, now);
            }
        });
    }
    sync(sfu, &peer);
    for id in unpaused {
        let user = room.lock().track(&id).map(|l| l.user.clone());
        if let Some(u) = user
            && let Some(p) = sfu.peer(&u)
        {
            for i in p.ingress_of_track(&id) {
                crate::rtcp::shell::pli(sfu, &i, now, true);
            }
        }
    }
    Reply::ok(&serde_json::json!({}))
}
