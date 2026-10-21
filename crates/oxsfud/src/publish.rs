// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-PUB-ADD 정:IMPL-PUB-REMOVE 정:IMPL-PUB-ACCEPT 정:IMPL-PUB-TRACK-SET 정:IMPL-AFF-PUB-ROOM 정:IMPL-AFF-PUB-MOVE 정:IMPL-TRK-FROM-PUB 연:RULE-PUB-PUBLISH-PROC 연:RULE-PUB-SET-PROC 연:RULE-AFF-PROC 연:RULE-ROOM-PERMISSION-MAP · model: claude-opus-5-5
//! 발행 · 트랙 상태 · 발행 방 — 트랙(논리) 먼저 · 업링크 스트림(물리) 나중. 전체 거절은 원자다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use oxsig::Op;
use oxsig::code::Code;
use oxsig::wire::{
    AffiliationReq, AffiliationRes, AffiliationResCause, Duplex, Kind, Permission, PublishTrack, PublishTracksReqAddAction,
    PublishTracksResAddTracks, Publication, Source, TrackEventBody, TrackSetReq, TrackSetRes, TrackStateBody,
    TrackStateBodyType,
};
use serde_json::{Value, json};

use crate::media::{ExtIn, PubStream, PubTrack};
use crate::peer::Peer;
use crate::room::{Room, entry, remove_entry};
use crate::sfu::{Sfu, lock};

pub type Fail = (Code, String, Option<Value>);

fn fail(c: Code, m: impl Into<String>) -> Fail {
    (c, m.into(), None)
}

fn bit_name(kind: Kind, source: Option<Source>) -> &'static str {
    match (kind, source) {
        (Kind::Audio, _) => "publish_audio",
        (Kind::Video, Some(Source::Screen)) => "publish_screen",
        (Kind::Video, _) => "publish_video",
    }
}

fn allowed(p: &Permission, bit: &str) -> bool {
    match bit {
        "publish_audio" => p.publish_audio,
        "publish_screen" => p.publish_screen,
        _ => p.publish_video,
    }
}

fn member_perm(room: &Room, user: &str) -> Option<Permission> {
    room.lock().members.get(user).map(|m| m.permission)
}

fn lenient_kind(v: &Value) -> Kind {
    if v.get("kind").and_then(Value::as_str) == Some("video") { Kind::Video } else { Kind::Audio }
}

fn lenient_source(v: &Value) -> Option<Source> {
    match v.get("source").and_then(Value::as_str) {
        Some("screen") => Some(Source::Screen),
        Some(_) => Some(Source::Camera),
        None => None,
    }
}

fn lenient_half(v: &Value) -> bool {
    v.get("duplex").and_then(Value::as_str) == Some("half")
}

fn serde_code(e: &serde_json::Error) -> Code {
    if e.to_string().contains("missing field") { Code::MISSING_FIELD } else { Code::INVALID_PAYLOAD }
}

fn ext_id(v: &Value, k: &str) -> Option<u8> {
    v.get(k).and_then(Value::as_u64).map(|x| x as u8)
}

pub fn publish(sfu: &Arc<Sfu>, peer: &Arc<Peer>, body: &str) -> Result<String, Fail> {
    let v: Value = serde_json::from_str(body).map_err(|e| fail(Code::INVALID_PAYLOAD, e.to_string()))?;
    let room_id = v.get("room_id").and_then(Value::as_str).ok_or_else(|| fail(Code::MISSING_FIELD, "room_id"))?.to_string();
    let action = v.get("action").and_then(Value::as_str).unwrap_or("add");
    let room = sfu.room(&room_id).filter(|_| peer.in_room(&room_id)).ok_or_else(|| fail(Code::NOT_IN_ROOM, "not in room"))?;
    match action {
        "remove" => remove(sfu, peer, &room, &v),
        "add" => add(sfu, peer, &room, &v),
        _ => Err(fail(Code::INVALID_PAYLOAD, "action")),
    }
}

fn add(sfu: &Arc<Sfu>, peer: &Arc<Peer>, room: &Arc<Room>, v: &Value) -> Result<String, Fail> {
    let items = v.get("tracks").and_then(Value::as_array).ok_or_else(|| fail(Code::MISSING_FIELD, "tracks"))?;
    let perm = member_perm(room, &peer.user_id).unwrap_or(crate::room::ALL);
    let denied: Vec<&str> = {
        let mut d: Vec<&str> = items.iter().map(|it| bit_name(lenient_kind(it), lenient_source(it))).filter(|b| !allowed(&perm, b)).collect();
        d.sort();
        d.dedup();
        d
    };
    if !denied.is_empty() {
        return Err((Code::NOT_AUTHORIZED, "permission".into(), Some(json!({ "denied": denied }))));
    }
    let pub_room = peer.pub_room_id();
    if items.iter().any(|it| !lenient_half(it)) && pub_room.as_deref() != Some(room.id.as_str()) {
        return Err(fail(Code::NOT_PUB_ROOM, "full-duplex publish needs pub_room"));
    }
    let active = peer.active_count();
    if items.len() > 8 || active + items.len() > 16 {
        return Err((Code::TRACK_LIMIT, "track limit".into(), Some(json!({ "limit": 16, "per_request": 8, "current": active }))));
    }
    let mut tracks: Vec<PublishTrack> = Vec::with_capacity(items.len());
    for it in items {
        let t: PublishTrack = serde_json::from_value(it.clone()).map_err(|e| fail(serde_code(&e), e.to_string()))?;
        let sim = t.simulcast.unwrap_or(false);
        if t.kind == Kind::Audio && (t.source.is_some() || sim) {
            return Err(fail(Code::INVALID_PAYLOAD, "audio item carries source/simulcast"));
        }
        if sim != (t.ssrc == 0) {
            return Err(fail(Code::MISSING_FIELD, "ssrc/simulcast mismatch"));
        }
        if t.mid.is_empty() {
            return Err(fail(Code::MISSING_FIELD, "mid"));
        }
        tracks.push(t);
    }
    if peer.pc_mode == oxsig::wire::PcMode::V1pc {
        let used = lock(&peer.sub_conn.table).ssrcs.clone();
        for t in &tracks {
            for s in [Some(t.ssrc), t.rtx_ssrc].into_iter().flatten() {
                if s != 0 && used.contains(&s) {
                    return Err((Code::SSRC_COLLISION, "ssrc collision".into(), Some(json!({ "ssrc": s }))));
                }
            }
        }
    }
    let vroom = room.codecs.video.clone().unwrap_or_else(|| "H264".into());
    let aroom = room.codecs.audio.clone().unwrap_or_else(|| "opus".into());
    for t in &tracks {
        if t.kind == Kind::Video {
            let Some(c) = t.codec.as_deref() else {
                return Err((Code::CODEC_REQUIRED, "video codec required".into(), Some(json!({ "supported": ["VP8", "H264"] }))));
            };
            if !crate::rtp::has_keyframe_judge(c) {
                return Err((Code::CODEC_REQUIRED, format!("unsupported codec {c}"), Some(json!({ "supported": ["VP8", "H264"] }))));
            }
            if !crate::rtp::codec_matches(&vroom, crate::rtp::sub_fmtp(&vroom), c, t.fmtp.as_deref()) {
                return Err(mismatch(&vroom));
            }
        } else if let Some(c) = t.codec.as_deref()
            && !c.eq_ignore_ascii_case(&aroom)
        {
            return Err(mismatch(&aroom));
        }
    }
    let ext = ExtIn {
        twcc: ext_id(v, "twcc_extmap_id"),
        rid: ext_id(v, "rid_extmap_id"),
        rrid: ext_id(v, "repair_rid_extmap_id"),
        audio_level: ext_id(v, "audio_level_extmap_id"),
    };
    let conn = peer.pub_conn.clone();
    let mut out = Vec::new();
    let mut made: Vec<Arc<PubStream>> = Vec::new();
    for t in tracks {
        let sim = t.simulcast.unwrap_or(false);
        let half = t.duplex == Some(Duplex::Half);
        let codec = match t.kind {
            Kind::Video => crate::rtp::canon_codec(t.codec.as_deref().unwrap_or(&vroom)),
            Kind::Audio => crate::rtp::canon_codec(&aroom),
        };
        let stream = Arc::new(PubStream {
            track_id: format!("{}-{}", peer.user_id, common::ident::random_hex(4)),
            user_id: peer.user_id.clone(),
            kind: t.kind,
            source: if t.kind == Kind::Video { Some(t.source.unwrap_or(Source::Camera)) } else { None },
            codec,
            fmtp: t.fmtp.clone(),
            pt: t.pt,
            ssrc: t.ssrc,
            rtx_ssrc: t.rtx_ssrc,
            simulcast: sim && !half,
            half: AtomicBool::new(half),
            muted: AtomicBool::new(false),
            room_id: Mutex::new(room.id.clone()),
            listed: AtomicBool::new(false),
            ext: ext.clone(),
            tracks: ArcSwap::from_pointee(Vec::new()),
            subs: ArcSwap::from_pointee(Vec::new()),
            conn: Arc::downgrade(&conn),
            first_rtp_at: AtomicU64::new(0),
            pt_checked: AtomicBool::new(false),
            level_at: AtomicU64::new(0),
            notify_at: AtomicU64::new(0),
            gone: AtomicBool::new(false),
        });
        lock(&peer.streams).push(stream.clone());
        if stream.simulcast {
            let mut s: Vec<Arc<PubStream>> = conn.sims.load().iter().cloned().collect();
            s.push(stream.clone());
            conn.sims.store(Arc::new(s));
        } else {
            let phys = PubTrack::new(t.ssrc, None, crate::media::LAYER_H, &stream);
            if let Some(r) = t.rtx_ssrc {
                phys.rtx_ssrc.store(r, Ordering::Relaxed);
            }
            stream.tracks.store(Arc::new(vec![phys.clone()]));
            let _g = lock(&sfu.index_lock);
            let mut m: HashMap<u32, Arc<PubTrack>> = (**conn.pubs.load()).clone();
            m.insert(t.ssrc, phys.clone());
            conn.pubs.store(Arc::new(m));
            if let Some(r) = t.rtx_ssrc {
                let mut x = (**conn.rtx_of.load()).clone();
                x.insert(r, phys);
                conn.rtx_of.store(Arc::new(x));
            }
        }
        out.push(PublishTracksResAddTracks { mid: t.mid, track_id: stream.track_id.clone() });
        made.push(stream);
    }
    let mut listed_any = false;
    {
        let mut g = room.lock();
        for s in &made {
            if !s.is_half() && !s.simulcast {
                s.listed.store(true, Ordering::Relaxed);
                g.tracks.push(s.clone());
                listed_any = true;
            }
        }
        if listed_any {
            room.publish_listed(&g);
            let entries: Vec<_> = made.iter().filter(|s| s.listed.load(Ordering::Relaxed)).map(|s| entry(room, s)).collect();
            room.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
                action: PublishTracksReqAddAction::Add,
                room_id: room.id.clone(),
                tracks: entries,
                version,
            });
        }
    }
    if made.iter().any(|s| s.is_half()) {
        for r in peer.room_ids() {
            if let Some(rm) = sfu.room(&r) {
                crate::floor::reg_half(sfu, &rm, &peer.user_id);
            }
        }
    }
    if listed_any {
        refill_room(sfu, room);
    }
    tracing::info!(kind = "ev", spec = "IMPL-PUB-ADD", user = peer.user_id.as_str(), room = room.id.as_str(), n = made.len(), "published");
    Ok(serde_json::to_string(&oxsig::wire::PublishTracksRes::Add { tracks: out }).unwrap_or_default())
}

fn mismatch(codec: &str) -> Fail {
    (Code::CODEC_MISMATCH, "room media codec".into(), Some(json!({ "codec": codec, "fmtp": crate::rtp::sub_fmtp(codec) })))
}

pub fn refill_room(sfu: &Arc<Sfu>, room: &Room) {
    let users: Vec<String> = room.lock().members.keys().cloned().collect();
    for u in users {
        for p in sfu.peer_of_user(&u) {
            if p.in_room(&room.id) {
                crate::table::fill(sfu, &p, true);
            }
        }
    }
}

pub fn refill_peers(sfu: &Arc<Sfu>, peers: Vec<Arc<Peer>>) {
    for p in peers {
        crate::table::fill(sfu, &p, true);
    }
}

pub fn list_sim(sfu: &Arc<Sfu>, stream: &Arc<PubStream>) {
    if stream.listed.swap(true, Ordering::SeqCst) || stream.is_half() {
        return;
    }
    let Some(room) = sfu.room(&stream.room()) else { return };
    {
        let mut g = room.lock();
        if !g.tracks.iter().any(|s| Arc::ptr_eq(s, stream)) {
            g.tracks.push(stream.clone());
        }
        room.publish_listed(&g);
        let e = entry(&room, stream);
        room.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
            action: PublishTracksReqAddAction::Add,
            room_id: room.id.clone(),
            tracks: vec![e],
            version,
        });
    }
    let s = sfu.clone();
    let r = room.clone();
    tokio::spawn(async move { refill_room(&s, &r) });
}

pub fn unlist(sfu: &Arc<Sfu>, room: &Arc<Room>, streams: &[Arc<PubStream>]) -> Vec<Arc<Peer>> {
    let mut touched = Vec::new();
    {
        let mut g = room.lock();
        let gone: Vec<Arc<PubStream>> = g.tracks.iter().filter(|s| streams.iter().any(|x| Arc::ptr_eq(x, s))).cloned().collect();
        if gone.is_empty() {
            return touched;
        }
        g.tracks.retain(|s| !streams.iter().any(|x| Arc::ptr_eq(x, s)));
        room.publish_listed(&g);
        let entries: Vec<_> = gone.iter().map(|s| remove_entry(room, s)).collect();
        room.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
            action: PublishTracksReqAddAction::Remove,
            room_id: room.id.clone(),
            tracks: entries,
            version,
        });
        for s in &gone {
            touched.extend(crate::table::drop_stream_rows(s));
        }
    }
    touched
}

pub fn drop_stream(sfu: &Arc<Sfu>, peer: &Peer, stream: &Arc<PubStream>) {
    stream.gone.store(true, Ordering::SeqCst);
    lock(&peer.streams).retain(|s| !Arc::ptr_eq(s, stream));
    let conn = &peer.pub_conn;
    let _g = lock(&sfu.index_lock);
    let mut m = (**conn.pubs.load()).clone();
    m.retain(|_, t| !t.stream.upgrade().is_some_and(|s| Arc::ptr_eq(&s, stream)));
    conn.pubs.store(Arc::new(m));
    let mut x = (**conn.rtx_of.load()).clone();
    x.retain(|_, t| !t.stream.upgrade().is_some_and(|s| Arc::ptr_eq(&s, stream)));
    conn.rtx_of.store(Arc::new(x));
    let sims: Vec<Arc<PubStream>> = conn.sims.load().iter().filter(|s| !Arc::ptr_eq(s, stream)).cloned().collect();
    conn.sims.store(Arc::new(sims));
}

fn remove(sfu: &Arc<Sfu>, peer: &Arc<Peer>, room: &Arc<Room>, v: &Value) -> Result<String, Fail> {
    let ids: Vec<String> = v
        .get("track_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| fail(Code::MISSING_FIELD, "track_ids"))?
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let mut streams = Vec::new();
    for id in &ids {
        match peer.stream(id) {
            Some(s) => streams.push(s),
            None => return Err(fail(Code::TRACK_NOT_FOUND, format!("track {id}"))),
        }
    }
    for s in &streams {
        if s.is_half()
            && let Some(pr) = peer.pub_room.load_full()
            && crate::floor::is_holder(&pr, &peer.user_id)
        {
            crate::floor::release_track(sfu, &pr, &peer.user_id);
        }
        drop_stream(sfu, peer, s);
    }
    let mut touched = Vec::new();
    let listed: Vec<Arc<PubStream>> = streams.iter().filter(|s| s.listed.load(Ordering::Relaxed)).cloned().collect();
    let rooms: Vec<String> = {
        let mut r: Vec<String> = listed.iter().map(|s| s.room()).collect();
        r.sort();
        r.dedup();
        r
    };
    for rid in rooms {
        if let Some(rm) = sfu.room(&rid) {
            touched.extend(unlist(sfu, &rm, &listed));
        }
    }
    let _ = room;
    refill_peers(sfu, touched);
    Ok(serde_json::to_string(&oxsig::wire::PublishTracksRes::Remove {}).unwrap_or_default())
}

pub fn track_set(sfu: &Arc<Sfu>, peer: &Arc<Peer>, body: &str) -> Result<String, Fail> {
    let req: TrackSetReq = serde_json::from_str(body).map_err(|e| fail(serde_code(&e), e.to_string()))?;
    if !peer.in_room(&req.room_id) {
        return Err(fail(Code::NOT_IN_ROOM, "not in room"));
    }
    if req.muted.is_some() == req.duplex.is_some() {
        return Err(fail(Code::FIELD_CONFLICT, "muted xor duplex"));
    }
    let stream = match (&req.track_id, req.ssrc) {
        (Some(t), _) => peer.stream(t),
        (None, Some(s)) => peer.stream_by_ssrc(s),
        (None, None) => None,
    }
    .ok_or_else(|| fail(Code::TRACK_NOT_FOUND, "track"))?;
    let ssrc = stream.ssrc;
    if let Some(m) = req.muted {
        if stream.is_half() {
            return Err(fail(Code::TRACK_OP_UNSUPPORTED, "half-duplex has no mute"));
        }
        if stream.muted.load(Ordering::Relaxed) == m {
            return Ok(serde_json::to_string(&TrackSetRes { ssrc, muted: Some(m), duplex: None, noop: Some(true) }).unwrap_or_default());
        }
        stream.muted.store(m, Ordering::Relaxed);
        if stream.listed.load(Ordering::Relaxed)
            && let Some(rm) = sfu.room(&stream.room())
        {
            let g = rm.lock();
            room_state(sfu, &rm, &g, &stream, m);
        }
        return Ok(serde_json::to_string(&TrackSetRes { ssrc, muted: Some(m), duplex: None, noop: None }).unwrap_or_default());
    }
    let want_half = req.duplex == Some(Duplex::Half);
    if stream.simulcast {
        return Err(fail(Code::TRACK_OP_UNSUPPORTED, "simulcast cannot switch duplex"));
    }
    if !want_half && peer.pub_room_id().as_deref() != Some(req.room_id.as_str()) {
        return Err(fail(Code::NOT_PUB_ROOM, "full needs pub_room"));
    }
    if stream.is_half() == want_half {
        return Ok(serde_json::to_string(&TrackSetRes { ssrc, muted: None, duplex: req.duplex, noop: Some(true) }).unwrap_or_default());
    }
    if !want_half
        && let Some(pr) = peer.pub_room.load_full()
        && crate::floor::is_holder(&pr, &peer.user_id)
    {
        crate::floor::release_track(sfu, &pr, &peer.user_id);
    }
    stream.half.store(want_half, Ordering::SeqCst);
    stream.muted.store(false, Ordering::Relaxed);
    let room = if want_half {
        sfu.room(&stream.room())
    } else {
        let r = peer.pub_room.load_full();
        if let Some(r) = &r {
            let mut g = lock(&stream.room_id);
            let _ = std::mem::replace(&mut *g, r.id.clone());
        }
        r
    };
    let mut touched = Vec::new();
    if let Some(rm) = &room {
        let mut g = rm.lock();
        if !g.tracks.iter().any(|s| Arc::ptr_eq(s, &stream)) {
            g.tracks.push(stream.clone());
        }
        stream.listed.store(true, Ordering::Relaxed);
        rm.publish_listed(&g);
        let e = entry(rm, &stream);
        rm.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
            action: PublishTracksReqAddAction::Add,
            room_id: rm.id.clone(),
            tracks: vec![e],
            version,
        });
        if want_half {
            touched.extend(crate::table::drop_stream_rows(&stream));
        }
    }
    if want_half {
        for r in peer.room_ids() {
            if let Some(rm) = sfu.room(&r) {
                crate::floor::reg_half(sfu, &rm, &peer.user_id);
            }
        }
        refill_peers(sfu, touched);
    } else if let Some(rm) = &room {
        refill_room(sfu, rm);
        if stream.is_video() {
            crate::media::request_keyframe(sfu, &stream, None, true);
        }
    }
    Ok(serde_json::to_string(&TrackSetRes { ssrc, muted: None, duplex: req.duplex, noop: None }).unwrap_or_default())
}

fn room_state(sfu: &Sfu, room: &Room, g: &crate::room::Inner, stream: &PubStream, muted: bool) {
    room.emit(sfu, g, Op::TrackState, None, |version| TrackStateBody {
        type_: TrackStateBodyType::Muted,
        user_id: stream.user_id.clone(),
        track_id: stream.track_id.clone(),
        kind: stream.kind,
        room_id: room.id.clone(),
        version,
        muted,
    });
}

fn recheck(peer: &Peer, room: &Room) -> Result<(), Fail> {
    let vroom = room.codecs.video.clone().unwrap_or_else(|| "H264".into());
    let aroom = room.codecs.audio.clone().unwrap_or_else(|| "opus".into());
    let streams = lock(&peer.streams).clone();
    for s in &streams {
        let ok = match s.kind {
            Kind::Video => crate::rtp::codec_matches(&vroom, crate::rtp::sub_fmtp(&vroom), &s.codec, s.fmtp.as_deref()),
            Kind::Audio => s.codec.eq_ignore_ascii_case(&aroom),
        };
        if !ok {
            let c = if s.kind == Kind::Video { vroom.clone() } else { aroom.clone() };
            return Err(mismatch(&c));
        }
    }
    let perm = member_perm(room, &peer.user_id).unwrap_or(crate::room::ALL);
    let mut denied: Vec<&str> = streams.iter().map(|s| bit_name(s.kind, s.source)).filter(|b| !allowed(&perm, b)).collect();
    denied.sort();
    denied.dedup();
    if !denied.is_empty() {
        return Err((Code::NOT_AUTHORIZED, "permission".into(), Some(json!({ "denied": denied }))));
    }
    Ok(())
}

pub fn move_pub(sfu: &Arc<Sfu>, peer: &Arc<Peer>, to: &Arc<Room>) {
    let old = peer.pub_room.load_full();
    if let Some(o) = &old
        && !Arc::ptr_eq(o, to)
    {
        crate::floor::pub_away(sfu, o, &peer.user_id);
    }
    peer.pub_room.store(Some(to.clone()));
    crate::floor::pub_select(sfu, to, &peer.user_id);
    let moving: Vec<Arc<PubStream>> = lock(&peer.streams).iter().filter(|s| s.room() != to.id && (s.listed.load(Ordering::Relaxed) || !s.is_half())).cloned().collect();
    if moving.is_empty() {
        return;
    }
    let mut by_room: HashMap<String, Vec<Arc<PubStream>>> = HashMap::new();
    for s in &moving {
        by_room.entry(s.room()).or_default().push(s.clone());
    }
    for (rid, ss) in by_room {
        if let Some(rm) = sfu.room(&rid) {
            let mut g = rm.lock();
            let gone: Vec<Arc<PubStream>> = g.tracks.iter().filter(|s| ss.iter().any(|x| Arc::ptr_eq(x, s))).cloned().collect();
            g.tracks.retain(|s| !ss.iter().any(|x| Arc::ptr_eq(x, s)));
            rm.publish_listed(&g);
            if !gone.is_empty() {
                let entries: Vec<_> = gone.iter().map(|s| remove_entry(&rm, s)).collect();
                rm.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
                    action: PublishTracksReqAddAction::Remove,
                    room_id: rm.id.clone(),
                    tracks: entries,
                    version,
                });
            }
        }
    }
    {
        let mut g = to.lock();
        let mut added = Vec::new();
        for s in &moving {
            let mut r = lock(&s.room_id);
            let _ = std::mem::replace(&mut *r, to.id.clone());
            drop(r);
            if s.listed.load(Ordering::Relaxed) {
                g.tracks.push(s.clone());
                added.push(entry(to, s));
            }
        }
        to.publish_listed(&g);
        if !added.is_empty() {
            to.emit(sfu, &g, Op::TrackEvent, None, |version| TrackEventBody {
                action: PublishTracksReqAddAction::Add,
                room_id: to.id.clone(),
                tracks: added,
                version,
            });
        }
    }
    let mut peers: Vec<Arc<Peer>> = Vec::new();
    for s in &moving {
        for w in s.subs.load().iter() {
            if let Some(p) = w.upgrade().and_then(|r| r.conn.upgrade()).and_then(|c| c.peer())
                && !peers.iter().any(|x| Arc::ptr_eq(x, &p))
            {
                peers.push(p);
            }
        }
    }
    refill_peers(sfu, peers);
    refill_room(sfu, to);
}

pub fn affiliation(sfu: &Arc<Sfu>, peer: &Arc<Peer>, body: &str) -> Result<String, Fail> {
    let req: AffiliationReq = serde_json::from_str(body).map_err(|e| fail(Code::INVALID_PAYLOAD, e.to_string()))?;
    let target = req.pub_select.as_ref().filter(|r| peer.in_room(r)).and_then(|r| sfu.room(r));
    if let Some(t) = &target {
        recheck(peer, t)?;
    }
    if let Some(d) = &req.pub_deselect
        && peer.pub_room_id().as_deref() == Some(d.as_str())
        && let Some(o) = peer.pub_room.load_full()
    {
        crate::floor::pub_away(sfu, &o, &peer.user_id);
        peer.pub_room.store(None);
    }
    if let Some(t) = &target
        && peer.pub_room_id().as_deref() != Some(t.id.as_str())
    {
        move_pub(sfu, peer, t);
    }
    let a = peer.affiliation();
    let res = AffiliationRes { sub_rooms: a.sub_rooms, pub_room: a.pub_room, cause: AffiliationResCause::User, change_id: req.change_id };
    Ok(serde_json::to_string(&res).unwrap_or_default())
}

pub fn publications(peer: &Peer) -> Vec<Publication> {
    lock(&peer.streams)
        .iter()
        .filter(|s| !s.gone.load(Ordering::Relaxed))
        .map(|s| Publication {
            track_id: s.track_id.clone(),
            room_id: s.room(),
            kind: s.kind,
            duplex: if s.is_half() { Duplex::Half } else { Duplex::Full },
            codec: Some(s.codec.clone()),
            fmtp: s.fmtp.clone(),
            simulcast: s.simulcast,
        })
        .collect()
}
