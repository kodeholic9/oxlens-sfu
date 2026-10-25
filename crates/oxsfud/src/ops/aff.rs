// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-AFF-PUB-ROOM 정:IMPL-AFF-PUB-MOVE 정:RULE-AFF-ONE-SET 정:IMPL-FWD-ROOM 정:RULE-FLR-SNAPSHOT 연:AFF.S.select 연:AFF.S.select_moved 연:AFF.S.away 연:AFF.S.same 연:AFF.S.reject 연:AFF.S.reject_perm 연:AFF.S.join_select 연:RULE-AFF-PROC 연:RULE-AFF-RES-FORM 연:RULE-AFF-SWITCH-TRACKS 연:RULE-AFF-PUB-SELECT-CODEC-RECHECK 연:RULE-ROOM-HALF-FULL-SPLIT · model: claude-opus-5-5
//! 발행 방 옮기기 — 재검사 둘(코덱 · 권한 비트) → 옛 방 발언권 반환 · 대기 철회 → `pub_room` 교체(RCU) → 전이중 트랙(쉬는 것까지)을 새 방으로.

use std::sync::Arc;

use oxsig::envelope::{Ctx, Reply};
use oxsig::msg::{AffiliationReq, AffiliationRes, AffiliationResCause, Kind};
use oxsig::{Code, Failure};
use serde_json::json;

use super::Sfu;
use super::peer::Peer;
use crate::fwd::shell::now_ms;
use crate::room::Listed;
use crate::trk::codec;

fn recheck(sfu: &Sfu, peer: &Peer, room: &str) -> Result<(), Failure> {
    let Some(r) = sfu.room(room) else { return Err(Code::RoomNotFound.failure()) };
    let streams = peer.streams();
    for s in &streams {
        let rc = r.codec_of(s.kind);
        if !codec::same_as_room(s.kind, rc, s.codec.as_deref(), s.fmtp.as_deref()) {
            return Err(Code::CodecMismatch.failure().with_details(json!({ "codec": rc, "fmtp": codec::row_fmtp(rc) })));
        }
    }
    let perm = r.lock().members.get(&peer.user).map(|m| m.perm.clone()).unwrap_or_else(crate::room::all_perm);
    let mut denied: Vec<&'static str> = Vec::new();
    for s in &streams {
        let bit = crate::r#pub::bit_of(s.kind, s.source);
        if !crate::r#pub::allowed(&perm, bit) && !denied.contains(&bit) {
            denied.push(bit);
        }
    }
    if !denied.is_empty() {
        return Err(Code::NotAuthorized.failure().with_details(json!({ "denied": denied })));
    }
    Ok(())
}

fn movable(peer: &Peer) -> Vec<Listed> {
    peer.streams().iter().filter(|s| s.listed.load(std::sync::atomic::Ordering::Acquire)).map(|s| super::publish::listed_of(s)).collect()
}

pub async fn move_pub(sfu: &Arc<Sfu>, peer: &Arc<Peer>, to: Option<&str>) -> Result<(), Failure> {
    let old = peer.aff.pub_room().map(|s| (*s).clone());
    if old.as_deref() == to {
        return Ok(());
    }
    if let Some(t) = to {
        recheck(sfu, peer, t)?;
    }
    let now = now_ms();
    if let Some(o) = &old
        && let Some(r) = sfu.room(o)
    {
        let step = r.floor.pub_away(&peer.user, now);
        super::floor::exec(sfu, &r, step).await;
    }
    peer.aff.set_pub(to);
    if let Some(t) = to
        && let Some(r) = sfu.room(t)
    {
        let step = r.floor.pub_select(&peer.user, now);
        super::floor::exec(sfu, &r, step).await;
    }
    let tracks = movable(peer);
    if tracks.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = tracks.iter().map(|l| l.track_id.clone()).collect();
    let both: Vec<String> = match (&old, to) {
        (Some(o), Some(t)) => {
            let a = sfu.room(o).map(|r| super::publish::members(&r)).unwrap_or_default();
            let b = sfu.room(t).map(|r| super::publish::members(&r)).unwrap_or_default();
            a.into_iter().filter(|u| b.contains(u)).collect()
        }
        _ => Vec::new(),
    };
    if let (Some(o), Some(t)) = (&old, to) {
        for u in &both {
            if let Some(p) = sfu.peer(u) {
                let (o2, t2) = (o.clone(), t.to_string());
                let ids2 = ids.clone();
                super::publish::table_change(sfu, &p, |tb| {
                    for id in &ids2 {
                        tb.move_room(id, &o2, &t2);
                    }
                });
            }
        }
    }
    if let Some(o) = &old
        && let Some(r) = sfu.room(o)
    {
        super::publish::unlist(sfu, &r, &ids);
    }
    if let Some(t) = to
        && let Some(r) = sfu.room(t)
    {
        super::publish::list(sfu, &r, &tracks);
        for s in peer.streams().iter().filter(|s| s.kind == Kind::Video && !s.half()) {
            for i in peer.ingress_of_track(&s.track_id) {
                crate::rtcp::shell::pli(sfu, &i, now, true);
            }
        }
    }
    Ok(())
}

pub async fn affiliation(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let req: AffiliationReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Reply::fail(Code::InvalidPayload),
    };
    if req.pub_select.is_none() && req.pub_deselect.is_none() {
        return Reply::failure(Code::MissingField.failure_msg("pub_select · pub_deselect"));
    }
    let Some(peer) = sfu.peer(&ctx.user_id) else {
        return Reply::ok(&AffiliationRes { sub_rooms: Vec::new(), pub_room: None, cause: AffiliationResCause::User, change_id: req.change_id });
    };
    let _op = peer.op.lock().await;
    let sel = req.pub_select.clone().filter(|r| peer.aff.has(r));
    if let Some(s) = &sel
        && let Err(f) = recheck(sfu, &peer, s)
    {
        return Reply::failure(f);
    }
    let cur = peer.aff.pub_room().map(|s| (*s).clone());
    let label;
    match (&sel, &req.pub_deselect) {
        (Some(s), _) => {
            label = if cur.is_none() {
                "AFF.S.select"
            } else if cur.as_deref() == Some(s) {
                "AFF.S.same"
            } else {
                "AFF.S.select_moved"
            };
            if let Err(f) = move_pub(sfu, &peer, Some(s)).await {
                return Reply::failure(f);
            }
        }
        (None, Some(d)) if cur.as_deref() == Some(d.as_str()) => {
            label = "AFF.S.away";
            let _ = move_pub(sfu, &peer, None).await;
        }
        _ => label = "AFF.S.same",
    }
    common::log::transition(&common::log::Step {
        area: "AFF",
        key: &ctx.user_id,
        from: cur.as_deref().unwrap_or("NONE"),
        to: Some(peer.aff.pub_room().as_deref().map(String::as_str).unwrap_or("NONE")),
        cause: "AFFILIATION",
        label: Some(label),
        ins: &[],
        room: None,
        user: Some(&ctx.user_id),
    });
    let a = peer.aff.snapshot();
    Reply::ok(&AffiliationRes { sub_rooms: a.sub_rooms, pub_room: a.pub_room, cause: AffiliationResCause::User, change_id: req.change_id })
}
