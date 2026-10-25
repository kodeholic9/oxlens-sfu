// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-PUB-ADD 정:IMPL-PUB-REMOVE 정:IMPL-PUB-ACCEPT 정:IMPL-PUB-TRACK-SET 정:IMPL-TRK-FROM-PUB 정:IMPL-ROOM-EMIT 정:IMPL-SUB-PIPELINE 정:IMPL-SUB-TABLE 정:RULE-FWD-BRANCH-AUTHORITY 연:PUB.S.add 연:PUB.S.remove 연:PUB.S.remove_3005 연:PUB.S.set_duplex 연:PUB.S.set_noop 연:PUB.S.mute 연:PUB.S.mute_3006 연:TRK.S.pub_full 연:TRK.S.full_to_half 연:TRK.S.half_to_full 연:TRK.S.unpub_full 연:TRK.S.mute 연:RULE-TRK-TRACK-STATE-TO-ALL 연:RULE-PUB-SET-PROC 연:RULE-PUB-SET-EXCLUSIVE · model: claude-opus-5-5
//! 발행 조율 — 검사(발행 영역) 뒤 논리 먼저 · 물리 나중 · 트랙 층 바뀜은 그 방 `seq++` 한 번 + `TRACK_EVENT` 한 통 · 구독자마다 배정표 → `MLINE_EVENT`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::envelope::{Ctx, Reply};
use oxsig::msg::{
    Duplex, Kind, PublishTracksReqAddAction, PublishTracksRes, PublishTracksResAddTracks, TrackEventBody, TrackSetReq, TrackSetRes, TrackStateBody,
    TrackStateBodyType,
};
use oxsig::{Code, Op};
use serde_json::Value;

use super::Sfu;
use super::call::body_json;
use super::peer::Peer;
use crate::fwd::Ingress;
use crate::fwd::shell::now_ms;
use crate::r#pub::{AddInput, ExtIds, Stream, check_add};
use crate::room::{Listed, Room};
use crate::sub::table::{Table, Want};

pub fn track_id() -> String {
    format!("t{}", common::id::random_hex(6))
}

pub fn listed_of(s: &Stream) -> Listed {
    Listed {
        track_id: s.track_id.clone(),
        user: s.user.clone(),
        kind: s.kind,
        source: s.source,
        active: !s.half(),
        muted: s.muted(),
        simulcast: s.simulcast,
        fmtp: s.fmtp.clone(),
    }
}

pub fn want_of(sfu: &Sfu, room: &Room, l: &Listed, sub: &Peer, now: u64) -> Option<Want> {
    if !l.active || l.user == sub.user {
        return None;
    }
    let prefs = sub.lock().prefs.get(&(room.id.clone(), l.track_id.clone())).cloned();
    if prefs.as_ref().is_some_and(|p| p.paused.load(Ordering::Relaxed)) {
        return None;
    }
    let score = match l.kind {
        Kind::Video => u64::from(prefs.map(|p| p.priority.load(Ordering::Relaxed)).unwrap_or(0)),
        Kind::Audio => sfu.peer(&l.user).and_then(|p| p.lock().pubs.by_id(&l.track_id)).map(|s| s.last_loud_ms.load(Ordering::Relaxed)).unwrap_or(0),
    };
    Some(Want {
        room: room.id.clone(),
        track_id: l.track_id.clone(),
        user: l.user.clone(),
        kind: l.kind,
        codec: room.codec_of(l.kind).to_string(),
        score,
        since_ms: now,
    })
}

pub fn receivable(sfu: &Sfu, room: &Room, peer: &Peer) -> Vec<Want> {
    let now = now_ms();
    let listed: Vec<Listed> = room.lock().tracks.clone();
    listed.iter().filter_map(|l| want_of(sfu, room, l, peer, now)).collect()
}

pub fn members(room: &Room) -> Vec<String> {
    room.lock().members.keys().cloned().collect()
}

pub fn table_change(sfu: &Arc<Sfu>, peer: &Arc<Peer>, f: impl FnOnce(&mut Table)) {
    let wire = {
        let mut st = peer.lock();
        f(&mut st.table);
        st.table.finish().map(|_| st.table.wire())
    };
    super::sub::sync(sfu, peer);
    if let Some(t) = wire {
        common::ev!(spec = "CP-OP-MLINE-EVENT", user = peer.user.as_str(), arev = t.arev, "mline_event mlines={}", t.mlines.len());
        sfu.emit_user(&peer.user, Op::MlineEvent, &oxsig::msg::MlineEventBody { table: t }, None);
    }
}

pub fn list(sfu: &Arc<Sfu>, room: &Arc<Room>, entries: &[Listed]) {
    if entries.is_empty() {
        return;
    }
    let users = {
        let mut st = room.lock();
        for l in entries {
            st.upsert_track(l.clone());
        }
        let seq = st.bump();
        let tracks = entries.iter().map(|l| crate::trk::individual(room, l)).collect();
        sfu.emit_room(
            &room.id,
            Op::TrackEvent,
            &TrackEventBody { action: PublishTracksReqAddAction::Add, room_id: room.id.clone(), tracks, version: room.version(seq) },
            Vec::new(),
        );
        st.members.keys().cloned().collect::<Vec<String>>()
    };
    let now = now_ms();
    for u in users {
        let Some(p) = sfu.peer(&u) else { continue };
        let wants: Vec<Want> = entries.iter().filter_map(|l| want_of(sfu, room, l, &p, now)).collect();
        let offs: Vec<String> = entries.iter().filter(|l| !l.active).map(|l| l.track_id.clone()).collect();
        table_change(sfu, &p, |t| {
            for id in &offs {
                t.unwant(&room.id, id, now);
            }
            for w in wants {
                t.want(w, now);
            }
        });
    }
}

pub fn unlist(sfu: &Arc<Sfu>, room: &Arc<Room>, ids: &[String]) -> Vec<Listed> {
    let (gone, users) = {
        let mut st = room.lock();
        let gone: Vec<Listed> = ids.iter().filter_map(|id| st.take_track(id)).collect();
        if !gone.is_empty() {
            let seq = st.bump();
            let tracks = gone.iter().map(|l| crate::trk::removed(&room.id, l)).collect();
            sfu.emit_room(
                &room.id,
                Op::TrackEvent,
                &TrackEventBody { action: PublishTracksReqAddAction::Remove, room_id: room.id.clone(), tracks, version: room.version(seq) },
                Vec::new(),
            );
        }
        (gone, st.members.keys().cloned().collect::<Vec<String>>())
    };
    if gone.is_empty() {
        return gone;
    }
    let now = now_ms();
    for u in users {
        let Some(p) = sfu.peer(&u) else { continue };
        table_change(sfu, &p, |t| {
            for l in &gone {
                t.unwant(&room.id, &l.track_id, now);
            }
        });
    }
    gone
}

fn detach_ingress(peer: &Peer, ings: &[Arc<Ingress>]) {
    let _ = peer;
    for i in ings {
        i.subs.store(Arc::new(Vec::new()));
    }
}

pub fn drop_stream(peer: &Peer, track: &str) {
    let gone = peer.pub_conn().remove_ingress(track);
    detach_ingress(peer, &gone);
}

async fn floor_release_if_speaker(sfu: &Arc<Sfu>, peer: &Arc<Peer>) {
    let Some(r) = peer.aff.pub_room() else { return };
    let Some(room) = sfu.room(&r) else { return };
    if room.floor.holder().as_deref() == Some(peer.user.as_str()) {
        let step = room.floor.release(&peer.user, now_ms());
        super::floor::exec(sfu, &room, step).await;
    }
}

async fn register_half(sfu: &Arc<Sfu>, peer: &Arc<Peer>) {
    for r in peer.aff.rooms() {
        let Some(room) = sfu.room(&r) else { continue };
        let perm = room.lock().members.get(&peer.user).map(|m| m.perm.clone());
        let Some(perm) = perm else { continue };
        let fp = super::join::floor_perm(peer, &perm);
        if let Some(step) = room.floor.set_half(&peer.user, true, fp, now_ms()) {
            super::floor::exec(sfu, &room, step).await;
        }
    }
}

pub async fn publish_tracks(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let v = match body_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let room_id = match super::call::need_str(&v, "room_id") {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.aff.has(&room_id)) else { return Reply::fail(Code::NotInRoom) };
    let Some(room) = sfu.room(&room_id) else { return Reply::fail(Code::NotInRoom) };
    let _op = peer.op.lock().await;
    if v.get("action").and_then(Value::as_str) == Some("remove") {
        return remove(sfu, &peer, &v).await;
    }
    let items: Vec<Value> = match v.get("tracks") {
        Some(Value::Array(a)) => a.clone(),
        None | Some(Value::Null) => return Reply::failure(Code::MissingField.failure_msg("tracks")),
        Some(_) => return Reply::fail(Code::InvalidPayload),
    };
    let ext_id = |k: &str| v.get(k).and_then(Value::as_u64).map(|x| x as u8);
    let ext = ExtIds {
        twcc: ext_id("twcc_extmap_id"),
        rid: ext_id("rid_extmap_id"),
        rrid: ext_id("repair_rid_extmap_id"),
        mid: ext_id("mid_extmap_id"),
        audio_level: ext_id("audio_level_extmap_id"),
    };
    let Some(perm) = room.lock().members.get(&ctx.user_id).map(|m| m.perm.clone()) else { return Reply::fail(Code::NotInRoom) };
    let (active, down) = {
        let st = peer.lock();
        (st.pubs.streams.len(), (ctx.pc_mode == oxsig::msg::PcMode::V1pc).then(|| st.table.downlink_ssrcs()))
    };
    let input =
        AddInput { items: &items, perm: &perm, is_pub_room: peer.aff.is_pub(&room_id), active, ext, room_codecs: &room.codecs, downlink_ssrcs: down.as_ref() };
    let tracks = match check_add(&input) {
        Ok(t) => t,
        Err(f) => return Reply::failure(f),
    };
    if let Some(id) = ext.twcc {
        peer.pub_conn().twcc_id.store(id, Ordering::Relaxed);
    }
    let mut res = Vec::new();
    let mut fresh: Vec<Arc<Stream>> = Vec::new();
    for t in &tracks {
        let s = Arc::new(Stream::new(&ctx.user_id, &room_id, t, ext, track_id()));
        {
            let mut st = peer.lock();
            st.pubs.streams.push(s.clone());
            if ctx.pc_mode == oxsig::msg::PcMode::V1pc {
                let up = st.pubs.uplink_ssrcs();
                st.table.avoid.extend(up);
            }
        }
        if !s.simulcast {
            let ing = Arc::new(Ingress::new(s.ssrc, 1, s.clone(), Arc::downgrade(peer.pub_conn())));
            peer.pub_conn().add_ingress(ing);
            if let Some(r) = s.rtx_ssrc {
                peer.pub_conn().add_rtx(r, s.ssrc);
            }
        }
        res.push(PublishTracksResAddTracks { mid: s.mid.clone(), track_id: s.track_id.clone() });
        fresh.push(s);
        common::log::transition(&common::log::Step {
            area: "PUB",
            key: &res[res.len() - 1].track_id,
            from: "IDLE",
            to: Some("REG"),
            cause: "PUBLISH_TRACKS",
            label: Some("PUB.S.add"),
            ins: &[],
            room: Some(&room_id),
            user: Some(&ctx.user_id),
            arev: None,
        });
    }
    if fresh.iter().any(|s| s.half()) {
        register_half(sfu, &peer).await;
    }
    let full: Vec<Listed> = fresh.iter().filter(|s| !s.half() && !s.simulcast).map(|s| listed_of(s)).collect();
    if !full.is_empty() {
        for s in fresh.iter().filter(|s| !s.half() && !s.simulcast) {
            s.listed.store(true, Ordering::Release);
        }
        list(sfu, &room, &full);
    }
    Reply::ok(&PublishTracksRes::Add { tracks: res })
}

async fn remove(sfu: &Arc<Sfu>, peer: &Arc<Peer>, v: &Value) -> Reply {
    let ids: Vec<String> = match v.get("track_ids") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => return Reply::failure(Code::MissingField.failure_msg("track_ids")),
    };
    let streams: Vec<Option<Arc<Stream>>> = {
        let st = peer.lock();
        ids.iter().map(|i| st.pubs.by_id(i)).collect()
    };
    if streams.iter().any(Option::is_none) {
        return Reply::fail(Code::TrackNotFound);
    }
    for s in streams.into_iter().flatten() {
        if s.half() {
            floor_release_if_speaker(sfu, peer).await;
        }
        peer.lock().pubs.remove(&s.track_id);
        drop_stream(peer, &s.track_id);
        if let Some(r) = peer.aff.pub_room()
            && let Some(room) = sfu.room(&r)
        {
            unlist(sfu, &room, std::slice::from_ref(&s.track_id));
        }
        common::log::transition(&common::log::Step {
            area: "PUB",
            key: &s.track_id,
            from: "REG",
            to: Some("IDLE"),
            cause: "PUBLISH_TRACKS",
            label: Some("PUB.S.remove"),
            ins: &[],
            room: None,
            user: Some(&peer.user),
            arev: None,
        });
    }
    Reply::ok(&PublishTracksRes::Remove {})
}

pub async fn sim_attached(sfu: &Arc<Sfu>, peer: &Arc<Peer>, track: &str) {
    let Some(s) = peer.lock().pubs.by_id(track) else { return };
    let Some(r) = peer.aff.pub_room() else { return };
    let Some(room) = sfu.room(&r) else { return };
    if !s.listed.swap(true, Ordering::AcqRel) {
        list(sfu, &room, &[listed_of(&s)]);
        return;
    }
    for u in members(&room) {
        if let Some(p) = sfu.peer(&u) {
            super::sub::sync(sfu, &p);
        }
    }
}

pub async fn track_set(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let req: TrackSetReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Reply::fail(Code::InvalidPayload),
    };
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.aff.has(&req.room_id)) else { return Reply::fail(Code::NotInRoom) };
    if sfu.room(&req.room_id).is_none() {
        return Reply::fail(Code::NotInRoom);
    }
    if req.muted.is_some() == req.duplex.is_some() {
        return Reply::fail(Code::FieldConflict);
    }
    let _op = peer.op.lock().await;
    let s = {
        let st = peer.lock();
        match (&req.track_id, req.ssrc) {
            (Some(t), _) => st.pubs.by_id(t),
            (None, Some(x)) => st.pubs.by_ssrc(x),
            (None, None) => None,
        }
    };
    let Some(s) = s else { return Reply::fail(Code::TrackNotFound) };
    let now = now_ms();
    if let Some(m) = req.muted {
        if s.half() {
            return Reply::fail(Code::TrackOpUnsupported);
        }
        if s.muted() == m {
            return Reply::ok(&TrackSetRes { ssrc: s.ssrc, muted: None, duplex: None, noop: Some(true) });
        }
        s.swap_muted(m);
        if let Some(r) = peer.aff.pub_room()
            && let Some(rm) = sfu.room(&r)
        {
            let mut st = rm.lock();
            if let Some(l) = st.tracks.iter_mut().find(|l| l.track_id == s.track_id) {
                l.muted = m;
            }
            let seq = st.bump();
            let b = TrackStateBody {
                type_: TrackStateBodyType::Muted,
                user_id: peer.user.clone(),
                track_id: s.track_id.clone(),
                kind: s.kind,
                room_id: rm.id.clone(),
                version: rm.version(seq),
                muted: m,
            };
            sfu.emit_room(&rm.id, Op::TrackState, &b, Vec::new());
        }
        return Reply::ok(&TrackSetRes { ssrc: s.ssrc, muted: Some(m), duplex: None, noop: None });
    }
    let Some(d) = req.duplex else { return Reply::fail(Code::FieldConflict) };
    if s.simulcast {
        return Reply::fail(Code::TrackOpUnsupported);
    }
    if d == Duplex::Full && !peer.aff.is_pub(&req.room_id) {
        return Reply::fail(Code::NotPubRoom);
    }
    if s.duplex() == d {
        return Reply::ok(&TrackSetRes { ssrc: s.ssrc, muted: None, duplex: None, noop: Some(true) });
    }
    if d == Duplex::Full {
        floor_release_if_speaker(sfu, &peer).await;
    }
    s.swap_duplex(d);
    s.swap_muted(false);
    s.listed.store(true, Ordering::Release);
    if d == Duplex::Half {
        register_half(sfu, &peer).await;
    }
    if let Some(r) = peer.aff.pub_room()
        && let Some(rm) = sfu.room(&r)
    {
        list(sfu, &rm, &[listed_of(&s)]);
        if d == Duplex::Full && s.kind == Kind::Video {
            for i in peer.ingress_of_track(&s.track_id) {
                crate::rtcp::shell::pli(sfu, &i, now, true);
            }
        }
    }
    common::log::transition(&common::log::Step {
        area: "PUB",
        key: &s.track_id,
        from: if d == Duplex::Half { "full" } else { "half" },
        to: Some(if d == Duplex::Half { "half" } else { "full" }),
        cause: "TRACK_SET",
        label: Some("PUB.S.set_duplex"),
        ins: &[],
        room: Some(&req.room_id),
        user: Some(&peer.user),
        arev: None,
    });
    Reply::ok(&TrackSetRes { ssrc: s.ssrc, muted: None, duplex: Some(d), noop: None })
}
