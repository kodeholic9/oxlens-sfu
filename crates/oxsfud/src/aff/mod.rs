// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-AFF-ONE-SET · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · 연:RULE-AFF-PROC · 연:RULE-AFF-RES-FORM · 연:RULE-AFF-PUB-SELECT-CODEC-RECHECK · 연:AFF.S.reject · 연:AFF.S.reject_perm · 연:AFF.S.select_moved · model: claude-opus-5-5
//! AFF — `pub_room` 은 RCU 값(매 패킷 이 값으로 방을 정한다) · 적용 순서 deselect → select · 옮길 때 재검사 둘(코덱 · 권한) 하나라도 걸리면 아무것도 안 옮긴다 · 전이중(쉬는 것 포함)이 따라간다.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use oxsig::aff::{AffiliationReq, AffiliationRes};
use oxsig::err::{ErrCode, Failure};
use oxsig::trk::TrackType;
use serde_json::json;

use crate::cfg::Ctx;
use crate::link::{lock, Peer};
use crate::r#pub::core::{allowed, codec_matches, room_fmtp};

fn away(ctx: &Ctx, p: &Arc<Peer>, room_id: &str) {
    if let Some(r) = ctx.rooms.get(room_id) {
        let u = p.user_id.clone();
        r.floor.apply("pub_away", move |s| s.ws_pub_away(&u));
    }
}

fn select(ctx: &Ctx, p: &Arc<Peer>, room_id: &str) {
    if let Some(r) = ctx.rooms.get(room_id) {
        let u = p.user_id.clone();
        r.floor.apply("pub_select", move |s| s.ws_pub_select(&u));
    }
}

pub fn recheck(ctx: &Ctx, p: &Arc<Peer>, to: &str) -> Result<(), Failure> {
    let Some(room) = ctx.rooms.get(to) else { return Err(Failure::new(ErrCode::RoomNotFound)) };
    let perm = room.member(&p.user_id).map(|m| m.permission).unwrap_or_default();
    let pubs: Vec<Arc<crate::r#pub::PubStream>> = lock(&p.pubs).iter().filter(|s| !s.gone.load(Ordering::Acquire)).cloned().collect();
    for s in &pubs {
        let name = s.codec.clone().unwrap_or_else(|| room.media.audio().to_string());
        if !codec_matches(s.kind, &name, s.fmtp.as_deref(), &room.media) {
            let codec = if s.kind == oxsig::trk::MediaKind::Audio { room.media.audio() } else { room.media.video() };
            return Err(Failure::new(ErrCode::CodecMismatch).details(json!({"codec": codec, "fmtp": room_fmtp(&room.media, s.kind)})));
        }
    }
    let denied: Vec<&str> = pubs.iter().filter(|s| !allowed(&perm, s.kind, s.source)).map(|s| if s.kind == oxsig::trk::MediaKind::Audio { "publish_audio" } else if s.source == Some(oxsig::trk::Source::Screen) { "publish_screen" } else { "publish_video" }).collect();
    if !denied.is_empty() {
        return Err(Failure::new(ErrCode::NotAuthorized).details(json!({"denied": denied})));
    }
    Ok(())
}

pub fn move_tracks(ctx: &Ctx, p: &Arc<Peer>, from: Option<&str>, to: &str) {
    let Some(dst) = ctx.rooms.get(to) else { return };
    let pubs: Vec<Arc<crate::r#pub::PubStream>> = lock(&p.pubs).iter().filter(|s| !s.gone.load(Ordering::Acquire)).cloned().collect();
    for s in pubs {
        let old = s.room_id();
        let listed_old = ctx.tracks.entry(&old, &s.track_id).map(|t| t.kind_of == TrackType::Individual).unwrap_or(false);
        let half = s.half.load(Ordering::Acquire);
        if half && !listed_old {
            if let Ok(mut r) = s.room.lock() {
                r.clone_from(&to.to_string());
            }
            continue;
        }
        if old == to {
            continue;
        }
        if listed_old
            && let Some(src) = ctx.rooms.get(&old) {
                ctx.tracks.remove(&src, &s.track_id);
            }
        if let Ok(mut r) = s.room.lock() {
            r.clone_from(&to.to_string());
        }
        if listed_old || (!half && s.listed.load(Ordering::Acquire)) {
            let e = s.entry(dst.media.video());
            ctx.tracks.put(&dst, e.clone());
            if !half {
                crate::ops::wire::move_track(ctx, &e, &dst);
            }
        }
        tracing::info!(kind = "ev", spec = "AFF.S.select_moved", user = %p.user_id, track = %s.track_id, from = ?from, to = %to, area = "AFF", key = %p.user_id, cause = "AFFILIATION", "발행 트랙 따라감");
    }
}

pub fn apply(ctx: &Ctx, p: &Arc<Peer>, req: &AffiliationReq) -> Result<AffiliationRes, Failure> {
    let sel_ok = req.pub_select.as_ref().is_some_and(|s| p.in_room(s));
    if let Some(d) = &req.pub_deselect
        && p.pub_room().as_deref() == Some(d.as_str()) && !sel_ok {
            away(ctx, p, d);
            p.pub_room.store(None);
            tracing::info!(kind = "ev", spec = "AFF.S.away", user = %p.user_id, room = %d, area = "AFF", key = %p.user_id, cause = "AFFILIATION", "발행 방 해제");
        }
    if let Some(s) = &req.pub_select
        && p.in_room(s) && p.pub_room().as_deref() != Some(s.as_str()) {
            recheck(ctx, p, s)?;
            let old = p.pub_room();
            if let Some(o) = &old {
                away(ctx, p, o);
            }
            p.pub_room.store(Some(Arc::new(s.clone())));
            select(ctx, p, s);
            move_tracks(ctx, p, old.as_deref(), s);
            tracing::info!(kind = "ev", spec = if old.is_some() { "AFF.S.select_moved" } else { "AFF.S.select" }, user = %p.user_id, room = %s, area = "AFF", key = %p.user_id, cause = "AFFILIATION", "발행 방 선택");
        }
    Ok(AffiliationRes { sub_rooms: p.room_list(), pub_room: p.pub_room(), cause: "user".to_string(), change_id: req.change_id.clone() })
}

pub fn join_select(ctx: &Ctx, p: &Arc<Peer>, room: &str) {
    if p.pub_room().as_deref() == Some(room) {
        return;
    }
    let old = p.pub_room();
    if let Some(o) = &old {
        away(ctx, p, o);
    }
    p.pub_room.store(Some(Arc::new(room.to_string())));
    select(ctx, p, room);
    move_tracks(ctx, p, old.as_deref(), room);
}
