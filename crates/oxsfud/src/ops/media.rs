// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-PUB-ADD · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-TRACK-SET · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-READY-GATE · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-LINK-ICE-RESTART · 정:IMPL-ROOM-EMIT · 정:CAND-PUB-NOT-PUB-ROOM-ORDER · 연:RULE-AFF-PUB-ONLY · 연:RULE-AFF-TWO-ROOMS-ONE-REQ · 연:RULE-AFF-FAIL · 연:RULE-SUB-READY-WHAT · model: claude-opus-5-5
//! 미디어 op 조율 — 발행(검사 차례 ①~⑦ · 논리 먼저 물리 나중) · 구독 배관(대상 산출 → 배정표 → 줄 붙이기/떼기 → `MLINE_EVENT`) · `READY` · `SUBSCRIBE_LAYER` · `TRACK_SET` · `ICE_RESTART` · `AFFILIATION` · `MESSAGE`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::msg::*;
use oxsig::{Code, Failure, Op};
use serde_json::{Value, json};

use super::handle::Ctx;
use crate::fwd::LAYER_PAUSE;
use crate::link::{Cred, Peer};
use crate::r#pub::{PubStream, PubTrack, PublishState, StreamInit, index_add, index_remove};
use crate::room::{Room, notice};
use crate::sub::{Cand, Change, Class, SyncOut};

pub const PER_REQUEST: usize = 8;
pub const PER_PERSON: usize = 16;

fn sfu() -> Result<&'static Arc<super::Sfu>, Failure> {
    super::sfu().ok_or_else(|| Code::InternalError.failure())
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Failure> {
    oxsig::code::body(v)
}

fn ok<T: serde::Serialize>(v: &T) -> Result<Value, Failure> {
    serde_json::to_value(v).map_err(|_| Code::InternalError.failure())
}

pub fn joined(c: &Ctx, room_id: &str) -> Result<(Arc<Peer>, Arc<Room>), Failure> {
    let s = sfu()?;
    let room = s.rooms.get(room_id).ok_or_else(|| Code::RoomNotFound.failure())?;
    let peer = s.peer(&c.user).ok_or_else(|| Code::NotInRoom.failure())?;
    if !peer.in_room(room_id) {
        return Err(Code::NotInRoom.failure());
    }
    Ok((peer, room))
}

fn voice_rank(s: &PubStream) -> i64 {
    s.physical().iter().map(|t| t.last_voice_ms.load(Ordering::Acquire)).max().unwrap_or(0) as i64
}

pub fn cands_for(p: &Peer) -> Vec<Cand> {
    let Some(s) = super::sfu() else { return Vec::new() };
    let table = s.table(p);
    let mut out = Vec::new();
    for rid in p.rooms() {
        let Some(room) = s.rooms.get(&rid) else { continue };
        for k in [Kind::Audio, Kind::Video] {
            let codec = match k {
                Kind::Audio => room.media_codecs.audio.clone().unwrap_or_else(|| crate::trk::DEFAULT_AUDIO.into()),
                Kind::Video => crate::trk::room_video(&room.media_codecs).to_string(),
            };
            out.push(Cand { track_id: crate::trk::slot_track_id(&rid, k), room: rid.clone(), kind: k, class: Class::Slot, codec, rank: 0, paused: false });
        }
        for st in room_streams(&rid) {
            if st.user == p.user || st.is_half() {
                continue;
            }
            let (prio, paused) = table.as_ref().map(|t| t.pref(&st.track_id)).unwrap_or((0, false));
            let rank = if st.kind == Kind::Video { prio as i64 } else { voice_rank(&st) };
            out.push(Cand { track_id: st.track_id.clone(), room: rid.clone(), kind: st.kind, class: Class::Full, codec: st.codec_name().to_string(), rank, paused });
        }
    }
    out
}

pub fn room_streams(room: &str) -> Vec<Arc<PubStream>> {
    let Some(s) = super::sfu() else { return Vec::new() };
    let all: Vec<Arc<PubStream>> = s.tracks.lock().map(|g| g.values().cloned().collect()).unwrap_or_default();
    all.into_iter().filter(|st| crate::trk::listed(st, room)).collect()
}

pub fn apply_changes(changes: &[Change]) {
    let Some(s) = super::sfu() else { return };
    for c in changes {
        if let Some((tid, rid)) = &c.old {
            match c.class {
                Class::Full => {
                    if let Some(st) = s.stream(tid) {
                        st.rows.rcu(|v| v.iter().filter(|r| !Arc::ptr_eq(r, &c.down)).cloned().collect::<Vec<_>>());
                    }
                }
                Class::Slot => {
                    if let Some(room) = s.rooms.get(rid) {
                        room.slot(c.down.kind).rows.rcu(|v| v.iter().filter(|r| !Arc::ptr_eq(r, &c.down)).cloned().collect::<Vec<_>>());
                    }
                }
            }
        }
        c.down.src_track.store(None);
        c.down.layer.store(LAYER_PAUSE, Ordering::Release);
        if let Some((tid, rid)) = &c.new {
            match c.class {
                Class::Full => {
                    if let Some(st) = s.stream(tid) {
                        let t = c.down.cap.load(Ordering::Acquire).min(c.down.auto_cap.load(Ordering::Acquire));
                        c.down.target.store(t, Ordering::Release);
                        c.down.pending_since.store(common::now_ms(), Ordering::Release);
                        let d = c.down.clone();
                        st.rows.rcu(move |v| {
                            let mut n = (**v).clone();
                            n.push(d.clone());
                            n
                        });
                    }
                }
                Class::Slot => {
                    if let Some(room) = s.rooms.get(rid) {
                        let d = c.down.clone();
                        room.slot(c.down.kind).rows.rcu(move |v| {
                            let mut n = (**v).clone();
                            n.push(d.clone());
                            n
                        });
                    }
                }
            }
        }
    }
}

pub fn resync_peer(p: &Peer, notify: bool) -> SyncOut {
    let Some(s) = super::sfu() else { return SyncOut::default() };
    let Some(t) = s.table(p) else { return SyncOut::default() };
    let out = t.sync(&cands_for(p), common::now_ms());
    apply_changes(&out.changes);
    if out.changed && notify {
        let body = MlineEventBody { table: t.snapshot(&s.epoch) };
        super::user_notice(&p.user, Op::MlineEvent, &body, None);
    }
    out
}

pub fn resync_room(room: &str) {
    let Some(s) = super::sfu() else { return };
    let Some(r) = s.rooms.get(room) else { return };
    for u in r.users() {
        if let Some(p) = s.peer(&u) {
            resync_peer(&p, true);
        }
    }
}

pub fn track_event(room: &Room, action: TrackAction, tracks: Vec<TrackEntry>) {
    room.bump_and_send(|v| {
        let body = TrackEventBody { action, room_id: room.id.clone(), tracks, version: v };
        Some(notice(Op::TrackEvent, &body, Vec::new(), &room.id))
    });
}

pub fn announce_add(st: &Arc<PubStream>) {
    let Some(s) = super::sfu() else { return };
    let rid = st.room.load_full();
    let Some(room) = s.rooms.get(&rid) else { return };
    track_event(&room, TrackAction::Add, vec![crate::trk::entry(st)]);
    if let Ok(mut g) = st.state.lock()
        && *g != PublishState::Active
    {
        let from = g.as_str();
        let _old = std::mem::replace(&mut *g, PublishState::Active);
        super::transition("PUB", &st.track_id, "SV-STATE-PUBLISH-STATE", from, "Active", "first_rtp", &[("user", &st.user)]);
    }
    resync_room(&rid);
}

fn new_track_id() -> String {
    format!("t{}", common::random_hex(8))
}

fn perm_of(c: &Ctx, room: &Room) -> Permission {
    room.permission_for(&c.user, c.claims.as_ref().and_then(|x| x.permission))
}

fn item<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Option<T> {
    v.get(k).and_then(|x| serde_json::from_value(x.clone()).ok())
}

pub fn publish(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    match oxsig::code::field::<String>(body, "action")?.as_deref() {
        None | Some("add") => {}
        Some("remove") => {
            let room_id: String = oxsig::code::need(body, "room_id")?;
            let track_ids: Vec<String> = oxsig::code::need(body, "track_ids")?;
            return unpublish(c, &room_id, &track_ids);
        }
        Some(_) => return Err(Code::InvalidPayload.failure().with_message("action")),
    }
    let room_id: String = oxsig::code::need(body, "room_id")?;
    let items: Vec<Value> = oxsig::code::need(body, "tracks")?;
    let ext = |k: &str| -> Result<Option<u8>, Failure> { oxsig::code::field(body, k) };
    let (twcc_extmap_id, rid_extmap_id, repair_rid_extmap_id, mid_extmap_id, audio_level_extmap_id) =
        (ext("twcc_extmap_id")?, ext("rid_extmap_id")?, ext("repair_rid_extmap_id")?, ext("mid_extmap_id")?, ext("audio_level_extmap_id")?);
    {
        {
            let s = sfu()?;
            let (peer, room) = joined(c, &room_id)?;
            let perm = perm_of(c, &room);
            let denied: Vec<&str> = items
                .iter()
                .filter_map(|v| item::<Kind>(v, "kind").map(|k| (k, item::<Source>(v, "source"))))
                .filter(|(k, src)| !crate::aff::bit_allows(&perm, *k, *src))
                .map(|(k, src)| crate::aff::bit_name(k, src))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if !denied.is_empty() {
                return Err(Code::NotAuthorized.failure().with_details(json!({ "denied": denied })));
            }
            let pub_room = crate::aff::pub_room(&peer);
            if items.iter().any(|v| item::<Duplex>(v, "duplex").unwrap_or(Duplex::Full) == Duplex::Full) && pub_room.as_deref() != Some(room_id.as_str()) {
                return Err(Code::NotPubRoom.failure());
            }
            let ctx = s.pubs.ctx(&c.user);
            if items.len() > PER_REQUEST {
                return Err(Code::TrackLimit.failure().with_details(json!({ "limit": PER_REQUEST, "current": items.len() })));
            }
            if ctx.active() + items.len() > PER_PERSON {
                return Err(Code::TrackLimit.failure().with_details(json!({ "limit": PER_PERSON, "current": ctx.active() + items.len() })));
            }
            let mut tracks: Vec<PublishTrack> = Vec::with_capacity(items.len());
            for v in &items {
                for k in ["kind", "ssrc", "mid", "pt"] {
                    if v.get(k).map(|x| x.is_null()).unwrap_or(true) {
                        return Err(Code::MissingField.failure().with_message(k));
                    }
                }
                let t: PublishTrack = serde_json::from_value(v.clone()).map_err(|e| Code::InvalidPayload.failure().with_message(e.to_string()))?;
                if t.mid.is_empty() {
                    return Err(Code::MissingField.failure().with_message("mid"));
                }
                if t.simulcast.unwrap_or(false) && rid_extmap_id.is_none() {
                    return Err(Code::MissingField.failure().with_message("rid_extmap_id"));
                }
                tracks.push(t);
            }
            if peer.pc_mode == PcMode::One
                && let Some(t) = s.table(&peer)
            {
                let down = t.down_ssrcs();
                if tracks.iter().any(|x| down.contains(&x.ssrc) || x.rtx_ssrc.map(|r| down.contains(&r)).unwrap_or(false)) {
                    return Err(Code::SsrcCollision.failure());
                }
            }
            for t in &tracks {
                if t.kind != Kind::Video {
                    continue;
                }
                let Some(name) = t.codec.as_deref().and_then(crate::trk::canonical_video) else {
                    return Err(Code::CodecRequired.failure().with_details(json!({ "supported": crate::trk::supported_list() })));
                };
                if !crate::trk::matches_room(Kind::Video, Some(name), t.fmtp.as_deref(), &room.media_codecs) {
                    return Err(crate::trk::mismatch(Kind::Video, &room.media_codecs));
                }
            }
            let tr = peer.publish.clone();
            let mut ids = *tr.ext_ids.load_full();
            for (slot, v) in [
                (&mut ids.twcc, twcc_extmap_id),
                (&mut ids.rid, rid_extmap_id),
                (&mut ids.repair_rid, repair_rid_extmap_id),
                (&mut ids.mid, mid_extmap_id),
                (&mut ids.audio_level, audio_level_extmap_id),
            ] {
                if let Some(x) = v {
                    let _old = std::mem::replace(slot, x);
                }
            }
            tr.ext_ids.store(Arc::new(ids));
            let mut added = Vec::new();
            let mut to_announce = Vec::new();
            let mut any_half = false;
            for t in &tracks {
                let duplex = t.duplex.unwrap_or(Duplex::Full);
                let simulcast = t.simulcast.unwrap_or(false) && t.kind == Kind::Video && duplex == Duplex::Full;
                let codec = match t.kind {
                    Kind::Video => t.codec.as_deref().and_then(crate::trk::canonical_video).map(|x| x.to_string()),
                    Kind::Audio => t.codec.clone(),
                };
                let st = PubStream::new(StreamInit {
                    track_id: new_track_id(),
                    user: c.user.clone(),
                    kind: t.kind,
                    mid: t.mid.clone(),
                    pt: t.pt,
                    rtx_pt: t.rtx_pt,
                    codec,
                    fmtp: t.fmtp.clone(),
                    source: t.source,
                    simulcast,
                    duplex,
                    room: room_id.clone(),
                    ssrc: t.ssrc,
                    rtx_ssrc: t.rtx_ssrc,
                });
                if let Ok(mut g) = ctx.streams.lock() {
                    g.insert(st.track_id.clone(), st.clone());
                }
                if let Ok(mut g) = s.tracks.lock() {
                    g.insert(st.track_id.clone(), st.clone());
                }
                if let Some(from) = st.set_state(PublishState::Intended) {
                    super::transition("PUB", &st.track_id, "SV-STATE-PUBLISH-STATE", from.as_str(), "Intended", "publish", &[("user", &c.user)]);
                }
                if !simulcast {
                    let pt = PubTrack::new(st.clone(), t.ssrc, t.rtx_ssrc, None, &tr);
                    if let Ok(mut g) = st.tracks.lock() {
                        g.push(pt.clone());
                    }
                    index_add(&tr, &pt);
                }
                if peer.pc_mode == PcMode::One
                    && let Some(tb) = s.table(&peer)
                {
                    let mut v = vec![t.ssrc];
                    v.extend(t.rtx_ssrc);
                    tb.add_uplink(&v);
                }
                if duplex == Duplex::Half {
                    any_half = true;
                } else if !simulcast {
                    st.listed.store(true, Ordering::Release);
                    to_announce.push(st.clone());
                }
                added.push(AddedTrack { mid: t.mid.clone(), track_id: st.track_id.clone() });
            }
            if any_half {
                reg_half(&peer, true);
            }
            if !to_announce.is_empty() {
                track_event(&room, TrackAction::Add, to_announce.iter().map(|x| crate::trk::entry(x)).collect());
                resync_room(&room_id);
            }
            ok(&PublishTracksRes::Add { tracks: added })
        }
    }
}

pub fn reg_half(p: &Peer, half: bool) {
    let Some(s) = super::sfu() else { return };
    for rid in p.rooms() {
        if let Some(room) = s.rooms.get(&rid) {
            super::floor::run(&room, |f, _| f.ws_reg_half(&p.user, half));
        }
    }
}

pub fn has_half(user: &str) -> bool {
    super::sfu().and_then(|s| s.pubs.get(user)).map(|c| c.has_half()).unwrap_or(false)
}

fn release_floor_if_speaker(user: &str, room_id: &str) {
    let Some(s) = super::sfu() else { return };
    let Some(room) = s.rooms.get(room_id) else { return };
    let holder = room.floor.lock().ok().and_then(|f| f.holder());
    if holder.as_deref() == Some(user) {
        let m = crate::flr::core::Msg::new(crate::flr::core::Mt::Release, None, 0, 0);
        super::floor::run(&room, |f, t2| f.server_rx(user, &m, t2));
    }
}

pub fn drop_stream(peer: &Peer, st: &Arc<PubStream>) {
    let Some(s) = super::sfu() else { return };
    if let Some(ctx) = s.pubs.get(&peer.user)
        && let Ok(mut g) = ctx.streams.lock()
    {
        g.remove(&st.track_id);
    }
    if let Ok(mut g) = s.tracks.lock() {
        g.remove(&st.track_id);
    }
    index_remove(&peer.publish, st);
    if peer.pc_mode == PcMode::One
        && let Some(tb) = s.table(peer)
    {
        let mut v: Vec<u32> = st.physical().iter().map(|t| t.ssrc).collect();
        v.push(st.ssrc);
        v.extend(st.rtx_ssrc);
        tb.remove_uplink(&v);
    }
    st.rows.store(Arc::new(Vec::new()));
}

fn unpublish(c: &Ctx, room_id: &str, ids: &[String]) -> Result<Value, Failure> {
    let s = sfu()?;
    let (peer, _) = joined(c, room_id)?;
    let ctx = s.pubs.get(&c.user).ok_or_else(|| Code::TrackNotFound.failure())?;
    let mut list = Vec::new();
    for id in ids {
        list.push(ctx.find(id).ok_or_else(|| Code::TrackNotFound.failure().with_details(json!({ "track_id": id })))?);
    }
    let mut removed: std::collections::BTreeMap<String, Vec<TrackEntry>> = std::collections::BTreeMap::new();
    for st in &list {
        if st.is_half() {
            release_floor_if_speaker(&c.user, &st.room.load());
        }
        let listed = st.listed.swap(false, Ordering::AcqRel);
        drop_stream(&peer, st);
        let at = st.room.load().as_str().to_string();
        if listed && !at.is_empty() {
            removed.entry(at).or_default().push(crate::trk::entry(st));
        }
    }
    if !has_half(&c.user) {
        reg_half(&peer, false);
    }
    for (at, es) in removed {
        if let Some(r) = s.rooms.get(&at) {
            track_event(&r, TrackAction::Remove, es);
            resync_room(&at);
        }
    }
    ok(&PublishTracksRes::Remove {})
}

pub fn ready(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: ReadyReq = parse(body)?;
    let s = sfu()?;
    let (peer, room) = joined(c, &req.room_id)?;
    if let Some(t) = s.table(&peer) {
        let (first, became) = t.ready(&req.room_id, req.arev, common::now_ms());
        for (tid, _) in became {
            super::transition("SUB", &tid, "SV-STATE-SUBSCRIBE-STATE", "Created", "Active", "ready", &[("user", &c.user)]);
        }
        let _ = first;
    }
    for st in room_streams(&req.room_id) {
        if st.kind == Kind::Video && st.user != c.user && !st.is_half() {
            for t in st.physical() {
                crate::rtcp::request_pli(&t, true);
            }
        }
    }
    let speaker = room.slot(Kind::Video).speaker.load_full();
    if let Some(sp) = speaker
        && sp.as_str() != c.user
        && let Some(ctx) = s.pubs.get(&sp)
    {
        for st in ctx.streams() {
            if st.kind == Kind::Video && st.is_half() && st.room.load().as_str() == req.room_id {
                for t in st.physical() {
                    crate::rtcp::request_pli(&t, true);
                }
            }
        }
    }
    let user = c.user.clone();
    super::floor::run(&room, |f, _| f.ready(&user));
    ok(&Empty {})
}

pub fn subscribe_layer(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: SubscribeLayerReq = parse(body)?;
    let s = sfu()?;
    let (peer, _room) = joined(c, &req.room_id)?;
    let Some(t) = s.table(&peer) else { return ok(&Empty {}) };
    let now = common::now_ms();
    let mut resync = false;
    for x in &req.targets {
        let Some(st) = s.stream(&x.track_id) else { continue };
        if t.set_pref(&x.track_id, x.priority, x.paused) {
            resync = true;
        }
        if !st.simulcast {
            continue;
        }
        let (Some(row), Some(any)) = (t.row_of_track(&x.track_id), st.physical().into_iter().next()) else { continue };
        crate::sim::subscribe_layer(&row, &any, x.spatial, x.paused, x.priority, now);
    }
    if resync {
        resync_peer(&peer, true);
    }
    ok(&Empty {})
}

pub fn track_set(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: TrackSetReq = parse(body)?;
    let s = sfu()?;
    let (peer, room) = joined(c, &req.room_id)?;
    if req.muted.is_some() == req.duplex.is_some() {
        return Err(Code::FieldConflict.failure().with_message("muted xor duplex"));
    }
    let ctx = s.pubs.get(&c.user).ok_or_else(|| Code::TrackNotFound.failure())?;
    let st = match (&req.track_id, req.ssrc) {
        (Some(id), _) => ctx.find(id),
        (None, Some(x)) => ctx.by_ssrc(x),
        _ => return Err(Code::MissingField.failure().with_message("track_id/ssrc")),
    }
    .ok_or_else(|| Code::TrackNotFound.failure())?;
    if let Some(m) = req.muted {
        if st.is_half() {
            return Err(Code::TrackOpUnsupported.failure().with_message("muted on half"));
        }
        if st.muted.swap(m, Ordering::AcqRel) == m {
            return ok(&TrackSetRes { ssrc: st.ssrc, muted: Some(m), duplex: None, noop: Some(true) });
        }
        if st.listed.load(Ordering::Acquire) {
            let rid = st.room.load_full();
            if let Some(r) = s.rooms.get(&rid) {
                r.bump_and_send(|v| {
                    let body = TrackStateBody {
                        type_: TrackStateType::Muted,
                        user_id: c.user.clone(),
                        track_id: st.track_id.clone(),
                        kind: st.kind,
                        room_id: r.id.clone(),
                        version: v,
                        muted: m,
                    };
                    Some(notice(Op::TrackState, &body, Vec::new(), &r.id))
                });
            }
        }
        return ok(&TrackSetRes { ssrc: st.ssrc, muted: Some(m), duplex: None, noop: None });
    }
    let d = req.duplex.unwrap_or(Duplex::Full);
    if st.duplex() == d {
        return ok(&TrackSetRes { ssrc: st.ssrc, muted: None, duplex: Some(d), noop: Some(true) });
    }
    if st.simulcast {
        return Err(Code::TrackOpUnsupported.failure().with_message("simulcast duplex"));
    }
    if d == Duplex::Full && crate::aff::pub_room(&peer).as_deref() != Some(req.room_id.as_str()) {
        return Err(Code::NotPubRoom.failure());
    }
    if d == Duplex::Full {
        release_floor_if_speaker(&c.user, &st.room.load());
    }
    st.set_duplex(d);
    st.muted.store(false, Ordering::Release);
    st.listed.store(true, Ordering::Release);
    track_event(&room, TrackAction::Add, vec![crate::trk::entry(&st)]);
    reg_half(&peer, has_half(&c.user));
    resync_room(&req.room_id);
    if d == Duplex::Full && st.kind == Kind::Video {
        for t in st.physical() {
            crate::rtcp::pli_burst(&t);
        }
    }
    ok(&TrackSetRes { ssrc: st.ssrc, muted: None, duplex: Some(d), noop: None })
}

pub fn ice_restart(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: IceRestartReq = parse(body)?;
    let s = sfu()?;
    if req.sfu_id != s.epoch {
        return Err(Code::PeerNotFound.failure());
    }
    let peer = s.peer(&c.user).ok_or_else(|| Code::PeerNotFound.failure())?;
    let m = crate::fwd::rt::media().ok_or_else(|| Code::InternalError.failure())?;
    let mut creds = Vec::new();
    for tr in peer.transports() {
        let next = Cred::fresh();
        let old = tr.creds.lock().map_err(|_| Code::InternalError.failure())?.restart(next.clone());
        if let Some(o) = old {
            m.drop_ufrag(&o);
        }
        m.register(&next.ufrag, &tr);
        creds.push(IceCred { ufrag: next.ufrag, pwd: next.pwd });
    }
    let publish = creds.first().cloned().ok_or_else(|| Code::InternalError.failure())?;
    let subscribe = creds.get(1).cloned();
    tracing::info!(kind = "diag", user = %c.user, "ice restart issued {}", publish.ufrag);
    ok(&IceRestartRes { publish, subscribe })
}

pub fn move_pub(peer: &Peer, to: Option<&Arc<Room>>, token: Option<Permission>) -> Result<(), Failure> {
    let Some(s) = super::sfu() else { return Err(Code::InternalError.failure()) };
    let cur = crate::aff::pub_room(peer);
    let streams = s.pubs.get(&peer.user).map(|c| c.streams()).unwrap_or_default();
    if let Some(r) = to {
        if cur.as_deref() == Some(r.id.as_str()) {
            return Ok(());
        }
        crate::aff::recheck(r, &peer.user, &streams, token)?;
    }
    if let Some(or) = cur.as_deref().and_then(|old| s.rooms.get(old)) {
        let u = peer.user.clone();
        super::floor::run(&or, |f, _| f.ws_pub_away(&u));
    }
    crate::aff::set_pub_room(peer, to.map(|r| r.id.as_str()));
    let mut moved_from = Vec::new();
    let mut moved_to = Vec::new();
    for st in &streams {
        let listed = st.listed.load(Ordering::Acquire);
        let was = st.room.load().as_str().to_string();
        if listed && !was.is_empty() {
            moved_from.push(crate::trk::entry(st));
        }
        st.room.store(Arc::new(to.map(|r| r.id.clone()).unwrap_or_default()));
        if listed && to.is_some() {
            moved_to.push(crate::trk::entry(st));
        }
    }
    if let Some(old) = cur.as_deref()
        && !moved_from.is_empty()
        && let Some(or) = s.rooms.get(old)
    {
        for e in moved_from.iter_mut() {
            e.room_id = old.to_string();
        }
        track_event(&or, TrackAction::Remove, moved_from);
        resync_room(old);
    }
    if let Some(r) = to {
        let u = peer.user.clone();
        super::floor::run(r, |f, _| f.ws_pub_select(&u));
        if !moved_to.is_empty() {
            track_event(r, TrackAction::Add, moved_to);
            resync_room(&r.id);
        }
    }
    Ok(())
}

pub fn affiliation(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: AffiliationReq = parse(body)?;
    let s = sfu()?;
    let peer = s.peer(&c.user).ok_or_else(|| Code::NotInRoom.failure())?;
    let token = c.claims.as_ref().and_then(|x| x.permission);
    let old = crate::aff::pub_room(&peer);
    if let Some(sel) = req.pub_select.as_deref() {
        if let Some(room) = s.rooms.get(sel) {
            if !peer.in_room(sel) {
                return Err(Code::NotInRoom.failure());
            }
            if old.as_deref() == Some(sel) {
                aff_line(&c.user, "AFF.S.same", old.as_deref(), old.as_deref(), "AFFILIATION");
            } else if let Err(f) = move_pub(&peer, Some(&room), token) {
                let label = if f.code == Code::NotAuthorized.num() { "AFF.S.reject_perm" } else { "AFF.S.reject" };
                aff_line(&c.user, label, old.as_deref(), old.as_deref(), "AFFILIATION");
                return Err(f);
            } else {
                aff_line(&c.user, "AFF.S.select", old.as_deref(), Some(sel), "AFFILIATION");
            }
        }
    } else if let Some(de) = req.pub_deselect.as_deref()
        && old.as_deref() == Some(de)
    {
        release_floor_if_speaker(&c.user, de);
        move_pub(&peer, None, token)?;
        aff_line(&c.user, "AFF.S.away", old.as_deref(), None, "AFFILIATION");
    }
    let a = crate::aff::affiliation(&peer);
    ok(&AffiliationRes { sub_rooms: a.sub_rooms, pub_room: a.pub_room, cause: AffiliationCause::User, change_id: req.change_id })
}

pub fn aff_line(user: &str, label: &str, from: Option<&str>, to: Option<&str>, cause: &str) {
    super::transition("AFF", user, label, from.unwrap_or("-"), to.unwrap_or("-"), cause, &[("user", user)]);
}

pub fn message(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: MessageReq = parse(body)?;
    let (_peer, room) = joined(c, &req.room_id)?;
    let b = crate::aux::message_body(&req.room_id, &c.user, &req.content);
    crate::bus::room_notice(&room.id, notice(Op::Message, &b, vec![c.user.clone()], &room.id));
    ok(&MessageRes { msg_id: crate::aux::msg_id() })
}
