// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-PUB-ADD · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-TRACK-SET · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-LINK-ICE-RESTART · 정:IMPL-TRK-FROM-PUB · 연:PUB.op.PUBLISH_TRACKS · 연:PUB.op.TRACK_SET · 연:AFF.op.AFFILIATION · 연:LINK.op.ICE_RESTART · 연:RULE-PUB-REVOKED · 연:RULE-AFF-SWITCH-TRACKS · 연:RULE-AFF-PUB-SELECT-CODEC-RECHECK · 연:PROP-TRK-PUB-MOVE-FOLLOWS · 연:PROP-TRK-SLOT-FOLLOWS-PUB · 연:PUB.S.remove · 연:PUB.S.remove_3005 · 연:PUB.S.set_duplex · 연:PUB.S.set_noop · 연:PUB.S.set_3005 · 연:PUB.S.set_3011 · 연:PUB.S.mute · 연:PUB.S.mute_3006 · 연:PUB.S.mute_noop · 연:PUB.S.mute_3005 · 연:PUB.S.at_pub · 연:PUB.S.off_pub · 연:AFF.S.reject · 연:AFF.S.reject_perm · 연:LINK.S.restart · 연:LINK.S.restart_3010 · model: claude-opus-5-5
//! 발행 · 소속 · 연결 op — 발행(차례 검사 → 논리 먼저 · 물리 나중 → 목록 · 배정표) · 해제 · TRACK_SET(축 배타) · AFFILIATION(코덱 · 권한 재검사 → pub_room 교체 → 트랙이 따라간다) · ICE_RESTART(자격만 바뀐다).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::bus::envelope::Reply;
use oxsig::types::Kind;
use oxsig::{parse, AffiliationReq, AffiliationRes, AffiliationResCause, Code, Duplex, Failure, IceRestartRes, IceRestartResPublish, IceRestartResSubscribe, Op, PublishTracksReq, PublishTracksRes, PublishTracksResAddTracks, PublishTracksReqAddAction, TrackSetReq, TrackSetRes, TrackStateBody, TrackStateBodyType};
use serde_json::json;

use super::call::Who;
use super::{subs, Sfu, World};
use crate::aff::{judge, Move, Peer};
use crate::gc::track_event;
use crate::link::lock;
use crate::r#pub::core::{bit_for, check_add, codec_matches, AddCtx, Bit};
use crate::r#pub::{new_track_id, Phys, PubTrack};

fn body_fail(sfu: &Sfu, e: parse::BodyError) -> Reply {
    sfu.fail(e.code().failure_with(e.message(), None))
}

pub fn unpublish(sfu: &Sfu, p: &Peer, tr: &Arc<PubTrack>) {
    for d in tr.detach_all() {
        d.track.store(Arc::new(None));
        d.cur.store(Arc::new(None));
    }
    let ssrcs: Vec<u32> = tr.phys.load().iter().flat_map(|x| [x.ssrc, x.rtx_ssrc.load(Ordering::Relaxed)]).chain([tr.decl_ssrc, tr.decl_rtx_ssrc.unwrap_or(0)]).filter(|s| *s != 0).collect();
    p.pubt.set_ingress(|ing| {
        for s in &ssrcs {
            ing.by_ssrc.remove(s);
        }
        ing.by_mid.retain(|_, t| t.track_id != tr.track_id);
    });
    for h in sfu.hot.load().values() {
        for s in h.slots.iter() {
            if s.speaker.load().as_ref().as_ref().map(|(_, t)| t.track_id == tr.track_id).unwrap_or(false) {
                s.speaker.store(Arc::new(None));
            }
        }
    }
    p.drop_track(&tr.track_id);
    if !p.has_half() {
        let now = Sfu::now();
        for room in p.rooms() {
            if let Some(h) = sfu.room_hot(&room) {
                let _ = h.floor.reg_half(&p.user, false, now);
            }
        }
    }
}

fn precheck(w: &World, who: &Who, body: &serde_json::Value) -> Result<(), Failure> {
    if body.get("action").and_then(|a| a.as_str()) == Some("remove") {
        return Ok(());
    }
    let Some(room_id) = body.get("room_id").and_then(|r| r.as_str()) else { return Err(Code::MissingField.failure_with("room_id", None)) };
    let Some(items) = body.get("tracks").and_then(|t| t.as_array()) else { return Err(Code::MissingField.failure_with("tracks", None)) };
    let Some(room) = w.rooms.get(room_id) else { return Err(Code::RoomNotFound.failure()) };
    let Some(member) = room.member(&who.user) else { return Err(Code::NotInRoom.failure()) };
    let mut denied: Vec<&str> = Vec::new();
    for it in items {
        let kind = match it.get("kind").and_then(|k| k.as_str()) {
            Some("video") => Kind::Video,
            _ => Kind::Audio,
        };
        let source = match it.get("source").and_then(|k| k.as_str()) {
            Some("screen") => Some(oxsig::Source::Screen),
            Some("camera") => Some(oxsig::Source::Camera),
            _ => None,
        };
        let b = bit_for(kind, source);
        if !Bit::of(&member.permission, b) && !denied.contains(&b.name()) {
            denied.push(b.name());
        }
    }
    if !denied.is_empty() {
        return Err(Code::NotAuthorized.failure_with("권한 비트가 막는다", Some(json!({"denied": denied}))));
    }
    let peer = w.peers.get(&who.user);
    let is_pub = peer.and_then(|p| p.pub_room()).as_deref() == Some(room_id);
    if !is_pub && items.iter().any(|it| it.get("duplex").and_then(|d| d.as_str()) != Some("half")) {
        return Err(Code::NotPubRoom.failure());
    }
    let active = peer.map(|p| p.tracks().len()).unwrap_or(0);
    let limit = crate::r#pub::core::TRACK_LIMIT_ACTIVE;
    if items.len() > crate::r#pub::core::TRACK_LIMIT_REQ || active + items.len() > limit {
        return Err(Code::TrackLimit.failure_with("트랙 상한", Some(json!({"limit": limit, "current": active}))));
    }
    for (i, it) in items.iter().enumerate() {
        if let Err(e) = parse::typed::<oxsig::PublishTrack>(it.clone()) {
            return Err(e.code().failure_with(format!("tracks[{i}]: {}", e.message()), None));
        }
    }
    Ok(())
}

pub async fn publish(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    {
        let w = sfu.world.lock().await;
        if let Err(f) = precheck(&w, who, &body) {
            tracing::info!(kind = "ev", spec = "IMPL-PUB-ADD", user = %who.user, code = f.code, "PUBLISH_TRACKS refused before field checks");
            return sfu.fail(f);
        }
    }
    let req: PublishTracksReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return body_fail(sfu, e),
    };
    let mut w = sfu.world.lock().await;
    match req {
        PublishTracksReq::Remove { room_id, track_ids, .. } => remove(sfu, &mut w, who, &room_id, &track_ids),
        PublishTracksReq::Add { action, room_id, tracks, twcc_extmap_id, rid_extmap_id, repair_rid_extmap_id, mid_extmap_id, audio_level_extmap_id } => {
            if action == Some(PublishTracksReqAddAction::Remove) {
                return sfu.fail(Code::MissingField.failure_with("track_ids", None));
            }
            let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::NotInRoom.failure()) };
            let Some(room) = w.rooms.get(&room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
            let Some(member) = room.member(&who.user) else { return sfu.fail(Code::NotInRoom.failure()) };
            let taken_set: std::collections::HashSet<u32> = if peer.pc_mode == oxsig::PcMode::V1pc { lock(&peer.subt.table).ssrcs() } else { Default::default() };
            let taken = |s: u32| taken_set.contains(&s);
            let negotiated = |_k: Kind| true;
            let sim_without_rid = tracks.iter().any(|t| t.simulcast == Some(true)) && rid_extmap_id.is_none();
            if sim_without_rid {
                return sfu.fail(Code::MissingField.failure_with("rid_extmap_id", None));
            }
            let video = room.video_codec();
            let ctx = AddCtx {
                permission: member.permission,
                is_pub_room: peer.pub_room().as_deref() == Some(room_id.as_str()),
                active: peer.tracks().len(),
                uplink_ssrcs_taken: &taken,
                room_video: &video,
                room_video_fmtp: Some("packetization-mode=1;profile-level-id=42e01f"),
                negotiated: &negotiated,
            };
            if let Err(f) = check_add(&ctx, &tracks) {
                tracing::info!(kind = "ev", spec = format!("CP-CODE-{}", f.code).as_str(), user = %who.user, room = %room_id, "PUBLISH_TRACKS refused");
                return sfu.fail(f);
            }
            peer.pubt.set_ingress(|ing| {
                if let Some(x) = twcc_extmap_id {
                    ing.twcc_id = x;
                }
                if let Some(x) = rid_extmap_id {
                    ing.rid_id = x;
                }
                if let Some(x) = repair_rid_extmap_id {
                    ing.rrid_id = x;
                }
                if let Some(x) = mid_extmap_id {
                    ing.mid_id = x;
                }
                if let Some(x) = audio_level_extmap_id {
                    ing.level_id = x;
                }
            });
            let pub_room = peer.pub_room().unwrap_or_else(|| room_id.clone());
            let mut out = Vec::new();
            let mut added_full = Vec::new();
            let mut any_half = false;
            for t in &tracks {
                let duplex = t.duplex.unwrap_or(Duplex::Full);
                let simulcast = t.simulcast.unwrap_or(false) && duplex == Duplex::Full && t.kind == Kind::Video;
                let codec = match t.kind {
                    Kind::Video => t.codec.as_deref().and_then(crate::link::table::codec_norm).map(str::to_string),
                    Kind::Audio => Some("opus".to_string()),
                };
                let track = PubTrack::new(new_track_id(&who.user), &who.user, t.kind, t.source, &t.mid, codec, t.fmtp.clone(), t.pt, t.rtx_pt, simulcast, t.ssrc, t.rtx_ssrc, duplex, &pub_room);
                peer.add_track(track.clone());
                let clock = track.clock();
                if simulcast {
                    let tr = track.clone();
                    peer.pubt.set_ingress(|ing| {
                        ing.by_mid.insert(tr.mid.clone(), tr.clone());
                    });
                } else {
                    let phys = Phys::new(t.ssrc, t.rtx_ssrc, None, None, clock);
                    track.add_phys(phys.clone());
                    let (tr, ph, rtx) = (track.clone(), phys.clone(), t.rtx_ssrc);
                    peer.pubt.set_ingress(|ing| {
                        ing.by_ssrc.insert(ph.ssrc, (tr.clone(), ph.clone(), false));
                        if let Some(r) = rtx {
                            ing.by_ssrc.insert(r, (tr.clone(), ph.clone(), true));
                        }
                        ing.by_mid.insert(tr.mid.clone(), tr.clone());
                    });
                }
                tracing::info!(kind = "ev", spec = "PUB.S.add", user = %who.user, room = %room_id, track = %track.track_id, kind_ = t.kind.as_str(), duplex = ?duplex, simulcast, ssrc = t.ssrc, "track published (logical)");
                if duplex == Duplex::Half {
                    any_half = true;
                } else if !simulcast {
                    added_full.push(track.clone());
                }
                out.push(PublishTracksResAddTracks { mid: t.mid.clone(), track_id: track.track_id.clone() });
            }
            if any_half {
                let now = Sfu::now();
                for r in peer.rooms() {
                    if let Some(h) = sfu.room_hot(&r) {
                        let o = h.floor.reg_half(&who.user, true, now);
                        tracing::debug!(kind = "ev", spec = "FLR.S.ws.reg_half", user = %who.user, room = %r, labels = ?o.labels, "half registered");
                    }
                }
            }
            list_tracks(sfu, &mut w, &pub_room, &added_full);
            Reply::ok(&PublishTracksRes::Add { tracks: out })
        }
    }
}

pub fn list_tracks(sfu: &Sfu, w: &mut World, room: &str, tracks: &[Arc<PubTrack>]) {
    if tracks.is_empty() {
        return;
    }
    let Some(r) = w.rooms.get_mut(room) else { return };
    let video = r.video_codec();
    let mut entries = Vec::new();
    for t in tracks {
        t.room.store(Arc::new(room.to_string()));
        t.listed.store(true, Ordering::Release);
        r.list(t.clone());
        entries.push(crate::trk::entry_of(t, room, !t.half.load(Ordering::Relaxed), Some(&video)));
        tracing::info!(kind = "ev", spec = if t.half.load(Ordering::Relaxed) { "TRK.S.rest_in" } else { "TRK.S.pub_full" }, room = %room, track = %t.track_id, user = %t.user, "track listed");
    }
    track_event(sfu, w, room, PublishTracksReqAddAction::Add, entries);
    for t in tracks.iter().filter(|t| !t.half.load(Ordering::Relaxed)) {
        subs::track_on(sfu, w, room, t, true);
    }
}

pub fn unlist_tracks(sfu: &Sfu, w: &mut World, room: &str, tracks: &[Arc<PubTrack>]) {
    let Some(r) = w.rooms.get_mut(room) else { return };
    let video = r.video_codec();
    let mut entries = Vec::new();
    for t in tracks {
        if r.unlist(&t.track_id).is_some() {
            entries.push(crate::trk::entry_of(t, room, !t.half.load(Ordering::Relaxed), Some(&video)));
            tracing::info!(kind = "ev", spec = if t.half.load(Ordering::Relaxed) { "TRK.S.unpub_resting" } else { "TRK.S.unpub_full" }, room = %room, track = %t.track_id, "track unlisted");
        }
    }
    track_event(sfu, w, room, PublishTracksReqAddAction::Remove, entries);
    for t in tracks {
        subs::track_off(sfu, w, room, &t.track_id, true);
    }
}

fn remove(sfu: &Arc<Sfu>, w: &mut World, who: &Who, room_id: &str, ids: &[String]) -> Reply {
    let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::TrackNotFound.failure()) };
    if !w.rooms.get(room_id).map(|r| r.has(&who.user)).unwrap_or(false) {
        return sfu.fail(Code::NotInRoom.failure());
    }
    let mut found = Vec::new();
    for id in ids {
        match peer.track(id) {
            Some(t) => found.push(t),
            None => {
                tracing::info!(kind = "ev", spec = "PUB.S.remove_3005", user = %who.user, track = %id, "remove of unknown track");
                return sfu.fail(Code::TrackNotFound.failure_with("track_id", Some(json!({"track_id": id}))));
            }
        }
    }
    let now = Sfu::now();
    for t in &found {
        if t.half.load(Ordering::Relaxed)
            && let Some(pr) = peer.pub_room()
            && let Some(h) = sfu.room_hot(&pr)
            && h.floor.speaker_is(&who.user)
        {
            let o = h.floor.release_for(&who.user, now);
            crate::fwd::floor_out(sfu, &h, o);
        }
        let rooms: Vec<String> = w.rooms.iter().filter(|(_, r)| r.listed(&t.track_id).is_some()).map(|(k, _)| k.clone()).collect();
        for r in rooms {
            unlist_tracks(sfu, w, &r, std::slice::from_ref(t));
        }
        unpublish(sfu, &peer, t);
        tracing::info!(kind = "ev", spec = "PUB.S.remove", user = %who.user, track = %t.track_id, "track removed");
    }
    Reply::ok(&PublishTracksRes::Remove {})
}

pub async fn track_set(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: TrackSetReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return body_fail(sfu, e),
    };
    if req.muted.is_some() && req.duplex.is_some() {
        return sfu.fail(Code::FieldConflict.failure_with("muted · duplex 는 배타", None));
    }
    if req.muted.is_none() && req.duplex.is_none() {
        return sfu.fail(Code::MissingField.failure_with("muted · duplex", None));
    }
    let mut w = sfu.world.lock().await;
    let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::TrackNotFound.failure()) };
    let track = match (&req.track_id, req.ssrc) {
        (Some(id), _) => peer.track(id),
        (None, Some(s)) => peer.track_by_ssrc(s),
        _ => None,
    };
    let Some(t) = track else {
        tracing::info!(kind = "ev", spec = if req.muted.is_some() { "PUB.S.mute_3005" } else { "PUB.S.set_3005" }, user = %who.user, "TRACK_SET on unknown track");
        return sfu.fail(Code::TrackNotFound.failure());
    };
    let ssrc = t.decl_ssrc;
    if let Some(m) = req.muted {
        if t.half.load(Ordering::Relaxed) {
            tracing::info!(kind = "ev", spec = "PUB.S.mute_3006", user = %who.user, track = %t.track_id, "muted on half track");
            return sfu.fail(Code::TrackOpUnsupported.failure_with("반이중에는 mute 가 없다", None));
        }
        if t.muted.load(Ordering::Relaxed) == m {
            tracing::debug!(kind = "ev", spec = "PUB.S.mute_noop", user = %who.user, track = %t.track_id, "same muted");
            return Reply::ok(&TrackSetRes { ssrc, muted: Some(m), duplex: None, noop: Some(true) });
        }
        t.muted.store(m, Ordering::Release);
        let room = t.room();
        if let Some(r) = w.rooms.get_mut(&room)
            && r.listed(&t.track_id).is_some()
        {
            r.bump();
            let body = TrackStateBody { type_: TrackStateBodyType::Muted, user_id: who.user.clone(), track_id: t.track_id.clone(), kind: t.kind, room_id: room.clone(), version: r.version(&sfu.epoch), muted: m };
            sfu.to_room(&room, Op::TrackState, serde_json::to_value(body).unwrap_or_default(), Vec::new());
            tracing::info!(kind = "ev", spec = "TRK.S.mute", user = %who.user, track = %t.track_id, room = %room, muted = m, seq = r.seq, "muted changed");
        }
        tracing::info!(kind = "ev", spec = "PUB.S.mute", user = %who.user, track = %t.track_id, muted = m, "TRACK_SET muted");
        return Reply::ok(&TrackSetRes { ssrc, muted: Some(m), duplex: None, noop: None });
    }
    let want = req.duplex.unwrap_or(Duplex::Full);
    if t.simulcast {
        return sfu.fail(Code::TrackOpUnsupported.failure_with("시뮬캐스트 트랙의 duplex 전환", None));
    }
    let is_pub = peer.pub_room().as_deref() == Some(req.room_id.as_str());
    if want == Duplex::Full && !is_pub {
        tracing::info!(kind = "ev", spec = "PUB.S.set_3011", user = %who.user, track = %t.track_id, "duplex full outside pub room");
        return sfu.fail(Code::NotPubRoom.failure());
    }
    if t.duplex() == want {
        tracing::debug!(kind = "ev", spec = "PUB.S.set_noop", user = %who.user, track = %t.track_id, "same duplex");
        return Reply::ok(&TrackSetRes { ssrc, muted: None, duplex: Some(want), noop: Some(true) });
    }
    let now = Sfu::now();
    let room = peer.pub_room().unwrap_or_else(|| t.room());
    if want == Duplex::Full
        && let Some(h) = sfu.room_hot(&room)
        && h.floor.speaker_is(&who.user)
    {
        let o = h.floor.release_for(&who.user, now);
        crate::fwd::floor_out(sfu, &h, o);
    }
    t.half.store(want == Duplex::Half, Ordering::Release);
    t.ever_full.store(true, Ordering::Release);
    t.muted.store(false, Ordering::Release);
    let video = w.rooms.get(&room).map(|r| r.video_codec());
    if let Some(r) = w.rooms.get_mut(&room) {
        t.room.store(Arc::new(room.clone()));
        r.list(t.clone());
    }
    let entry = crate::trk::entry_of(&t, &room, want == Duplex::Full, video.as_deref());
    track_event(sfu, &mut w, &room, PublishTracksReqAddAction::Add, vec![entry]);
    if want == Duplex::Half {
        subs::track_off(sfu, &w, &room, &t.track_id, true);
        for r in peer.rooms() {
            if let Some(h) = sfu.room_hot(&r) {
                let _ = h.floor.reg_half(&who.user, true, now);
            }
        }
        tracing::info!(kind = "ev", spec = "TRK.S.full_to_half", user = %who.user, track = %t.track_id, room = %room, "resting entry");
    } else {
        subs::track_on(sfu, &w, &room, &t, true);
        if t.kind == Kind::Video
            && let Some(p) = t.main_phys()
        {
            t.kf_burst_until.store(now + 1500, Ordering::Release);
            crate::fwd::pli(sfu, &p, now, true);
        }
        if !peer.has_half() {
            for r in peer.rooms() {
                if let Some(h) = sfu.room_hot(&r) {
                    let _ = h.floor.reg_half(&who.user, false, now);
                }
            }
        }
        tracing::info!(kind = "ev", spec = "TRK.S.half_to_full", user = %who.user, track = %t.track_id, room = %room, "active entry");
    }
    tracing::info!(kind = "ev", spec = "PUB.S.set_duplex", user = %who.user, track = %t.track_id, duplex = ?want, "TRACK_SET duplex");
    Reply::ok(&TrackSetRes { ssrc, muted: None, duplex: Some(want), noop: None })
}

fn recheck(w: &World, peer: &Peer, to: &str) -> Result<(), Failure> {
    let Some(r) = w.rooms.get(to) else { return Err(Code::RoomNotFound.failure()) };
    let video = r.video_codec();
    for t in peer.tracks().iter().filter(|t| t.kind == Kind::Video) {
        let c = t.codec.as_deref().unwrap_or("");
        if !codec_matches(&video, None, c, t.fmtp.as_deref()) {
            return Err(Code::CodecMismatch.failure_with("새 방의 미디어 코덱과 다르다", Some(json!({"codec": video}))));
        }
    }
    let perm = r.member(&peer.user).map(|m| m.permission).unwrap_or(oxsig::Permission::ALL);
    let denied: Vec<&str> = peer.tracks().iter().map(|t| bit_for(t.kind, t.source)).filter(|b| !Bit::of(&perm, *b)).map(Bit::name).collect();
    if !denied.is_empty() {
        return Err(Code::NotAuthorized.failure_with("새 방의 권한 비트", Some(json!({"denied": denied}))));
    }
    Ok(())
}

pub fn move_pub(sfu: &Sfu, w: &mut World, peer: &Arc<Peer>, mv: Move, to: Option<String>, join: bool) -> Result<(), Failure> {
    let from = peer.pub_room();
    if mv == Move::Same {
        tracing::debug!(kind = "ev", spec = mv.label(join), user = %peer.user, "publish room unchanged");
        return Ok(());
    }
    if let Some(dst) = &to
        && let Err(f) = recheck(w, peer, dst)
    {
        tracing::info!(kind = "ev", spec = if f.code == Code::CodecMismatch.number() { "AFF.S.reject" } else { "AFF.S.reject_perm" }, user = %peer.user, room = %dst, "publish room change refused");
        return Err(f);
    }
    let now = Sfu::now();
    if let Some(old) = &from
        && let Some(h) = sfu.room_hot(old)
    {
        let o = h.floor.pub_away(&peer.user, now);
        crate::fwd::floor_out(sfu, &h, o);
    }
    peer.set_pub_room(to.clone());
    if let Some(dst) = &to
        && let Some(h) = sfu.room_hot(dst)
    {
        let o = h.floor.pub_select(&peer.user, now);
        crate::fwd::floor_out(sfu, &h, o);
    }
    let held: Vec<Arc<PubTrack>> = peer.tracks().into_iter().filter(|t| t.ever_full.load(Ordering::Relaxed)).filter(|t| !t.simulcast || !t.phys.load().is_empty()).collect();
    if let Some(old) = &from {
        let listed: Vec<Arc<PubTrack>> = held.iter().filter(|t| w.rooms.get(old).map(|r| r.listed(&t.track_id).is_some()).unwrap_or(false)).cloned().collect();
        unlist_tracks(sfu, w, old, &listed);
    }
    if let Some(dst) = &to {
        list_tracks(sfu, w, dst, &held);
        tracing::info!(kind = "ev", spec = "PUB.S.at_pub", user = %peer.user, room = %dst, "publish room set");
    } else {
        for t in &held {
            t.listed.store(false, Ordering::Release);
        }
        tracing::info!(kind = "ev", spec = "PUB.S.off_pub", user = %peer.user, "publish room cleared — RTP dropped");
    }
    tracing::info!(kind = "ev", spec = mv.label(join), user = %peer.user, from = ?from, to = ?to, moved = held.len(), "publish room changed");
    Ok(())
}

pub async fn affiliation(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: AffiliationReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return body_fail(sfu, e),
    };
    if req.pub_select.is_none() && req.pub_deselect.is_none() {
        return sfu.fail(Code::MissingField.failure_with("pub_select · pub_deselect", None));
    }
    let mut w = sfu.world.lock().await;
    let Some(peer) = w.peers.get(&who.user).cloned() else { return sfu.fail(Code::NotInRoom.failure()) };
    let select = req.pub_select.as_deref().filter(|r| peer.in_room(r));
    let deselect = req.pub_deselect.as_deref().filter(|r| peer.in_room(r));
    let current = peer.pub_room();
    let (mv, to) = judge(current.as_deref(), select, deselect);
    if let Err(f) = move_pub(sfu, &mut w, &peer, mv, to, false) {
        return sfu.fail(f);
    }
    let aff = peer.affiliation();
    Reply::ok(&AffiliationRes { sub_rooms: aff.sub_rooms, pub_room: aff.pub_room, cause: AffiliationResCause::User, change_id: req.change_id })
}

pub async fn ice_restart(sfu: &Arc<Sfu>, who: &Who, _body: serde_json::Value) -> Reply {
    let w = sfu.world.lock().await;
    let Some(peer) = w.peers.get(&who.user).cloned() else {
        tracing::info!(kind = "ev", spec = "LINK.S.restart_3010", user = %who.user, session = %who.session, "ICE_RESTART without peer");
        return sfu.fail(Code::PeerNotFound.failure());
    };
    drop(w);
    let p = peer.pubt.ice_restart();
    let sub = if Arc::ptr_eq(&peer.pubt, &peer.subt) { None } else { Some(peer.subt.ice_restart()) };
    tracing::info!(kind = "ev", spec = "LINK.S.restart", user = %who.user, session = %who.session, ufrag = %p.ufrag, "ICE credentials reissued");
    Reply::ok(&IceRestartRes { publish: IceRestartResPublish { ufrag: p.ufrag, pwd: p.pwd }, subscribe: sub.map(|s| IceRestartResSubscribe { ufrag: s.ufrag, pwd: s.pwd }) })
}
