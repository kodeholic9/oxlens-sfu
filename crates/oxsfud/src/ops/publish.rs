// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-PUB-ADD · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-TRACK-SET · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-SUB-READY-GATE · 정:IMPL-LINK-ICE-RESTART · 정:IMPL-AFF-PUB-ROOM · 연:PUB.S.add · 연:PUB.S.remove · 연:PUB.S.set_duplex · 연:PUB.S.mute · 연:SUB.S.ready · 연:LINK.S.restart · model: claude-opus-5-5
//! 발행 · `READY` · `SUBSCRIBE_LAYER` · `TRACK_SET` · `ICE_RESTART` · `AFFILIATION` 의 길 — 검사는 영역 core, 배관은 wire.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use common::handle::Envelope;
use oxsig::aff::AffiliationReq;
use oxsig::err::{ErrCode, Failure};
use oxsig::link::{IceCred, IceRestartReq, IceRestartRes};
use oxsig::r#pub::{Duplex, PublishAddRes, PublishParseError, PublishRemoveRes, PublishTracksReq, PublishedTrack, TrackSetReq, TrackSetRes};
use oxsig::ses::PcMode;
use oxsig::sub::{ReadyReq, SubscribeLayerReq};
use oxsig::trk::MediaKind;
use serde_json::Value;

use super::{fail, wire};
use crate::cfg::Ctx;
use crate::flr::core::{Msg, T};
use crate::link::{lock, Links, Peer, Role};
use crate::r#pub::core::{check_add, AddCtx};
use crate::r#pub::{InExt, PubStream, PubTrack};
use crate::room::Room;

fn now() -> u64 {
    common::clock::wall_ms()
}

fn member(ctx: &Ctx, env: &Envelope, room_id: &str) -> Result<(Arc<Room>, Arc<Peer>), Failure> {
    let room = ctx.rooms.get(room_id).ok_or_else(|| fail(ErrCode::NotInRoom))?;
    let p = ctx.links.peer(&env.user_id).filter(|p| p.in_room(room_id) && room.has(&env.user_id)).ok_or_else(|| fail(ErrCode::NotInRoom))?;
    Ok((room, p))
}

fn reg_half(p: &Arc<Peer>, ctx: &Ctx) {
    let half = lock(&p.pubs).iter().any(|s| s.half.load(Ordering::Acquire) && !s.gone.load(Ordering::Acquire));
    for r in p.room_list() {
        if let Some(room) = ctx.rooms.get(&r) {
            let u = p.user_id.clone();
            room.floor.apply("reg_half", move |s| s.ws_reg_half(&u, half));
        }
    }
}

fn release_if_holder(ctx: &Ctx, p: &Peer) {
    if let Some(r) = p.pub_room().and_then(|r| ctx.rooms.get(&r)) {
        crate::gc::release_floor(&r, &p.user_id);
    }
}

pub fn publish(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req = PublishTracksReq::parse(body).map_err(|e| match e {
        PublishParseError::Missing => fail(ErrCode::MissingField),
        PublishParseError::Invalid => fail(ErrCode::InvalidPayload),
    })?;
    let (room, p) = member(ctx, env, req.room_id())?;
    match req {
        PublishTracksReq::Remove { track_ids, .. } => {
            let found: Vec<Arc<PubStream>> = track_ids.iter().filter_map(|id| p.pub_by_id(id)).collect();
            if found.len() != track_ids.len() || found.is_empty() {
                return Err(fail(ErrCode::TrackNotFound));
            }
            for s in found {
                remove_stream(ctx, &p, &s);
                tracing::info!(kind = "ev", spec = "PUB.S.remove", user = %p.user_id, track = %s.track_id, room = %room.id, area = "PUB", key = %s.track_id, cause = "PUBLISH_TRACKS", "발행 제거");
            }
            lock(&p.pubs).retain(|s| !s.gone.load(Ordering::Acquire));
            reg_half(&p, ctx);
            Ok(serde_json::to_value(PublishRemoveRes { action: "remove".to_string() }).unwrap_or_default())
        }
        PublishTracksReq::Add { tracks, ext, .. } => {
            let perm = room.member(&p.user_id).map(|m| m.permission).unwrap_or_default();
            let downlink = Links::downlink_ssrcs(&p);
            let items = check_add(&tracks, &AddCtx { perm, active: p.active_pubs(), pc_mode: p.pc_mode, downlink: &downlink, codecs: &room.media, at_pub: p.pub_room().as_deref() == Some(room.id.as_str()), ext: &ext })?;
            let inx = InExt { twcc: ext.twcc, rid: ext.rid, rrid: ext.repair_rid, mid: ext.mid, audio_level: ext.audio_level };
            if let Some(id) = ext.twcc {
                p.pub_t.twcc_id.store(id, Ordering::Release);
            }
            let mut out = Vec::new();
            for t in items {
                let half = t.duplex == Some(Duplex::Half);
                let sim = t.simulcast.unwrap_or(false);
                let codec = t.codec.clone().or_else(|| (t.kind == MediaKind::Audio).then(|| room.media.audio().to_string()));
                let track_id = format!("t{}", common::ident::hex(&common::ident::random_u64().to_be_bytes()[..6]));
                let s = Arc::new(PubStream {
                    track_id: track_id.clone(),
                    user_id: p.user_id.clone(),
                    kind: t.kind,
                    mid: t.mid.clone(),
                    pt: t.pt,
                    rtx_pt: t.rtx_pt,
                    codec,
                    fmtp: t.fmtp.clone(),
                    source: t.source,
                    simulcast: sim,
                    ssrc: t.ssrc,
                    rtx_ssrc: t.rtx_ssrc,
                    ext: inx,
                    half: AtomicBool::new(half),
                    muted: AtomicBool::new(false),
                    listed: AtomicBool::new(false),
                    room: Mutex::new(room.id.clone()),
                    phys: ArcSwap::from_pointee(Vec::new()),
                    subs: ArcSwap::from_pointee(Vec::new()),
                    first_rtp_at: AtomicU64::new(0),
                    pt_warned: AtomicBool::new(false),
                    last_talk_ms: AtomicU64::new(0),
                    gone: AtomicBool::new(false),
                });
                if !sim {
                    let ph = Arc::new(PubTrack::new(t.ssrc, None, &s));
                    s.add_phys(ph.clone());
                    p.pub_t.ingress.insert(t.ssrc, ph.clone());
                    if let Some(r) = t.rtx_ssrc {
                        ph.rtx_ssrc.store(r as u64, Ordering::Release);
                        p.pub_t.rtx_in.insert(r, ph);
                    }
                }
                if p.pc_mode == PcMode::One {
                    let mut tb = lock(&p.table);
                    tb.uplink.insert(t.ssrc);
                    if let Some(r) = t.rtx_ssrc {
                        tb.uplink.insert(r);
                    }
                }
                lock(&p.pubs).push(s.clone());
                if !half && !sim {
                    s.listed.store(true, Ordering::Release);
                    let e = s.entry(room.media.video());
                    ctx.tracks.put(&room, e.clone());
                    wire::track_on(ctx, &room, &e);
                }
                tracing::info!(kind = "ev", spec = "PUB.S.add", user = %p.user_id, track = %track_id, room = %room.id, ssrc = t.ssrc, kind_of = t.kind.as_str(), duplex = if half { "half" } else { "full" }, simulcast = sim, area = "PUB", key = %track_id, cause = "PUBLISH_TRACKS", "발행");
                out.push(PublishedTrack { mid: t.mid, track_id });
            }
            reg_half(&p, ctx);
            Ok(serde_json::to_value(PublishAddRes { action: "add".to_string(), tracks: out }).unwrap_or_default())
        }
    }
}

pub fn remove_stream(ctx: &Ctx, p: &Arc<Peer>, s: &Arc<PubStream>) {
    if s.half.load(Ordering::Acquire) {
        release_if_holder(ctx, p);
    }
    s.gone.store(true, Ordering::Release);
    for ph in s.phys.load().iter() {
        p.pub_t.ingress.remove(&ph.ssrc);
        let r = ph.rtx_ssrc.load(Ordering::Acquire);
        if r != 0 {
            p.pub_t.rtx_in.remove(&(r as u32));
        }
    }
    if let Some(r) = ctx.rooms.get(&s.room_id())
        && ctx.tracks.has(&r.id, &s.track_id) {
            ctx.tracks.remove(&r, &s.track_id);
        }
    wire::track_off(ctx, &s.track_id);
}

pub fn ready(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    if body.get("room_id").is_none() {
        return Err(fail(ErrCode::MissingField));
    }
    let req: ReadyReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let room = ctx.rooms.get(&req.room_id).ok_or_else(|| fail(ErrCode::RoomNotFound))?;
    let (_, p) = member(ctx, env, &room.id)?;
    let r = crate::sub::ready(&p, &room.id, req.arev, &room.floor, now());
    tracing::info!(kind = "ev", spec = "SUB.S.ready", user = %p.user_id, room = %room.id, session = %p.session(), epoch = %ctx.boot.epoch, arev = req.arev, first = r.first, area = "SUB", key = %room.id, cause = "READY", "READY");
    let downs: Vec<Arc<crate::link::Down>> = lock(&p.downs).values().cloned().collect();
    for d in downs.iter().filter(|d| d.kind == MediaKind::Video) {
        if d.loaded().map(|l| l.room_id == room.id).unwrap_or(false) {
            crate::fwd::shell::key_for(d);
        }
    }
    Ok(Value::Null)
}

pub fn layer(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: SubscribeLayerReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let (room, p) = member(ctx, env, &req.room_id)?;
    let changes = crate::sub::layer(&p, &req.targets);
    let mut table_changed = false;
    for c in changes {
        let Some(t) = ctx.tracks.entry(&room.id, &c.track_id) else { continue };
        if c.paused {
            table_changed |= wire::unwant(ctx, &p, &c.track_id);
            continue;
        }
        if c.unpaused
            && let Some(w) = wire::want_of(ctx, &p, &room, &t) {
                table_changed |= wire::want(ctx, &p, w).1;
            }
        if let Some(mid) = lock(&p.table).row_of(&c.track_id).map(|r| r.mid)
            && let Some(d) = p.down(mid) {
                let changed = {
                    let mut st = lock(&d.st);
                    st.sel.cap = c.pref.spatial;
                    st.sel.paused = c.pref.paused;
                    let auto = if ctx.auto() { Some(lock(&p.sub_transport().sim).policy.layer) } else { None };
                    st.sel.aim(auto, now())
                };
                if changed || c.unpaused {
                    crate::fwd::shell::key_for(&d);
                }
            }
    }
    if table_changed {
        wire::emit_table(&p);
    }
    Ok(Value::Null)
}

pub fn track_set(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: TrackSetReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    if req.muted.is_some() == req.duplex.is_some() {
        return Err(fail(ErrCode::FieldConflict));
    }
    let (room, p) = member(ctx, env, &req.room_id)?;
    let s = match (&req.track_id, req.ssrc) {
        (Some(id), _) => p.pub_by_id(id),
        (None, Some(ss)) => lock(&p.pubs).iter().find(|s| s.ssrc == ss && !s.gone.load(Ordering::Acquire)).cloned(),
        _ => None,
    }
    .ok_or_else(|| fail(ErrCode::TrackNotFound))?;
    if let Some(m) = req.muted {
        if s.half.load(Ordering::Acquire) {
            return Err(fail(ErrCode::TrackOpUnsupported).msg("반이중 트랙에 muted"));
        }
        if s.muted.load(Ordering::Acquire) == m {
            return Ok(serde_json::to_value(TrackSetRes { ssrc: s.ssrc, muted: None, duplex: None, noop: Some(true) }).unwrap_or_default());
        }
        s.muted.store(m, Ordering::Release);
        if let Some(r) = ctx.rooms.get(&s.room_id()) {
            ctx.tracks.mute(&r, &s.track_id, &p.user_id, s.kind, m);
        }
        tracing::info!(kind = "ev", spec = "PUB.S.mute", user = %p.user_id, track = %s.track_id, muted = m, area = "PUB", key = %s.track_id, cause = "TRACK_SET", "muted");
        return Ok(serde_json::to_value(TrackSetRes { ssrc: s.ssrc, muted: Some(m), duplex: None, noop: None }).unwrap_or_default());
    }
    let to = req.duplex.unwrap_or(Duplex::Full);
    if s.simulcast {
        return Err(fail(ErrCode::TrackOpUnsupported).msg("시뮬캐스트 트랙은 무전 전환 불가"));
    }
    if to == Duplex::Full && p.pub_room().as_deref() != Some(room.id.as_str()) {
        return Err(fail(ErrCode::NotPubRoom));
    }
    if s.duplex() == to {
        return Ok(serde_json::to_value(TrackSetRes { ssrc: s.ssrc, muted: None, duplex: None, noop: Some(true) }).unwrap_or_default());
    }
    let dst = p.pub_room().and_then(|r| ctx.rooms.get(&r)).unwrap_or_else(|| room.clone());
    match to {
        Duplex::Half => {
            s.half.store(true, Ordering::Release);
            s.muted.store(false, Ordering::Release);
            if s.listed.load(Ordering::Acquire) {
                let e = s.entry(dst.media.video());
                ctx.tracks.put(&dst, e);
            }
            wire::track_off(ctx, &s.track_id);
        }
        Duplex::Full => {
            release_if_holder(ctx, &p);
            s.half.store(false, Ordering::Release);
            s.muted.store(false, Ordering::Release);
            if let Ok(mut r) = s.room.lock() {
                r.clone_from(&dst.id);
            }
            s.listed.store(true, Ordering::Release);
            let e = s.entry(dst.media.video());
            ctx.tracks.put(&dst, e.clone());
            wire::track_on(ctx, &dst, &e);
        }
    }
    reg_half(&p, ctx);
    tracing::info!(kind = "ev", spec = "PUB.S.set_duplex", user = %p.user_id, track = %s.track_id, duplex = ?to, area = "PUB", key = %s.track_id, cause = "TRACK_SET", "duplex");
    Ok(serde_json::to_value(TrackSetRes { ssrc: s.ssrc, muted: None, duplex: Some(to), noop: None }).unwrap_or_default())
}

pub fn ice_restart(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: IceRestartReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    if req.sfu_id != ctx.boot.epoch {
        return Err(fail(ErrCode::PeerNotFound));
    }
    let p = ctx.links.peer(&env.user_id).filter(|p| p.session() == env.session_id).ok_or_else(|| fail(ErrCode::PeerNotFound))?;
    let creds = ctx.links.restart(&p);
    let mut res = IceRestartRes { publish: IceCred { ufrag: String::new(), pwd: String::new() }, subscribe: None };
    for (role, c) in creds {
        let ic = IceCred { ufrag: c.ufrag.clone(), pwd: c.pwd.clone() };
        match role {
            Role::Subscribe => res.subscribe = Some(ic),
            _ => res.publish = ic,
        }
    }
    tracing::info!(kind = "ev", spec = "CP-OP-ICE-RESTART", user = %p.user_id, session = %p.session(), epoch = %ctx.boot.epoch, ufrag = %res.publish.ufrag, label = "LINK.S.restart", "ICE 재시작 자격");
    Ok(serde_json::to_value(res).unwrap_or_default())
}

pub fn affiliation(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: AffiliationReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let p = ctx.links.peer(&env.user_id).ok_or_else(|| fail(ErrCode::NotInRoom))?;
    let r = crate::aff::apply(ctx, &p, &req)?;
    Ok(serde_json::to_value(r).unwrap_or_default())
}

pub fn half_kinds(p: &Peer) -> Vec<MediaKind> {
    lock(&p.pubs).iter().filter(|s| s.half.load(Ordering::Acquire)).map(|s| s.kind).collect()
}

pub fn release_msg() -> Msg {
    Msg::new(T::Release, None, 0, 0)
}
