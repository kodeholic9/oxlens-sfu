// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-SERVER-CONFIG · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-EMIT · 정:IMPL-SES-RESUME · 정:IMPL-AFF-PUB-MOVE · 정:RULE-GC-EDGES · 연:ROOM.op.ROOM_JOIN · 연:ROOM.op.ROOM_LEAVE · 연:ROOM.http.CREATE_ROOM · 연:ROOM.http.GET_ROOM · 연:ROOM.http.LIST_ROOMS · 연:RULE-ROOM-CREATE-PROC · 연:RULE-ROOM-JOIN-START · 연:RULE-ROOM-DETAIL-FIELDS · 연:RULE-ROOM-DETAIL-NO-FLOOR · 연:RULE-ROOM-MEDIA-CODECS · 연:RULE-SES-RESUME-RES-TWO · 연:ROOM.S.leave_3002 · 연:AFF.C.any.implicit_join_unselected · model: claude-opus-5-5
//! 방 op — 생성(멱등 · 토큰 선언) · 입장(재입장 축출 → 정원 → 방 수 → 배정표(원자) → 등록 · 판번호 · 통지) · 퇴장 · 세션 폐기 · RESUME 조각 · 상세(재동기) · 목록.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use common::bus::envelope::{Home, Reply, ResumePart, RoomList};
use oxsig::types::Kind;
use oxsig::{parse, Code, CreateRoomReq, CreateRoomRes, GetRoomRes, MediaCodecs, Op, ParticipantEventBody, ParticipantEventBodyType, PcMode, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes, RoomSnapshot, ServerConfig, ServerConfigDtls, ServerConfigDtlsSetup, ServerConfigIce, TrackEntry};
use serde_json::json;

use super::call::Who;
use super::{subs, Sfu};
use crate::aff::{judge, Peer};
use crate::fwd::RoomHot;
use crate::gc::{leave_seven, reap_peer, Path};
use crate::link::table::{RowClass, Tag};
use crate::link::{lock, Role, Transport};
use crate::room::core::{check_capacity, check_rooms};
use crate::room::{Member, Room};

pub fn codecs() -> Vec<oxsig::CodecPolicy> {
    let v = |n: &str| oxsig::CodecPolicy { kind: Kind::Video, name: n.to_string(), rtcp_fb: vec!["nack".into(), "nack pli".into(), "ccm fir".into(), "transport-cc".into()] };
    vec![oxsig::CodecPolicy { kind: Kind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] }, v("VP8"), v("H264")]
}

pub fn server_config(sfu: &Sfu, p: &Peer) -> ServerConfig {
    let pc = p.pubt.cred();
    let sc = p.subt.cred();
    ServerConfig {
        sfu_id: sfu.epoch.clone(),
        pc_mode: p.pc_mode,
        ice: ServerConfigIce { ip: sfu.net.ip.to_string(), port: sfu.net.port, publish_ufrag: pc.ufrag, publish_pwd: pc.pwd, subscribe_ufrag: sc.ufrag, subscribe_pwd: sc.pwd, tcp_port: sfu.net.tcp_port },
        dtls: ServerConfigDtls { fingerprint: sfu.net.fingerprint.clone(), setup: ServerConfigDtlsSetup::Passive },
        codecs: codecs(),
        codecs_sub: None,
        extmap: crate::sub::server_extmap(),
        max_bitrate_bps: sfu.boot.policy.media.max_bitrate_bps as u64,
    }
}

pub fn tracks_of(r: &Room, hot: Option<&RoomHot>) -> Vec<TrackEntry> {
    let video = r.video_codec();
    let mut v: Vec<TrackEntry> = r.tracks.iter().map(|t| crate::trk::entry_of(t, &r.id, !t.half.load(std::sync::atomic::Ordering::Relaxed), Some(&video))).collect();
    if let Some(h) = hot {
        v.extend(h.slots.iter().map(|s| s.entry()));
    }
    v
}

pub fn snapshot(sfu: &Sfu, r: &Room, me: &str) -> Option<RoomSnapshot> {
    let m = r.member(me)?;
    Some(RoomSnapshot { participants: r.participants(), me: m.info(), tracks: tracks_of(r, sfu.room_hot(&r.id).as_deref()), version: r.version(&sfu.epoch) })
}

fn media_of(req: &Option<MediaCodecs>) -> MediaCodecs {
    let video = req.as_ref().and_then(|m| m.video.clone()).and_then(|v| crate::link::table::codec_norm(&v).map(str::to_string)).filter(|v| v != "opus").unwrap_or_else(|| "H264".into());
    MediaCodecs { audio: Some("opus".into()), video: Some(video) }
}

pub async fn create(sfu: &Arc<Sfu>, req: CreateRoomReq, room_id: String, unused: u32, departure: u32, capacity: u32) -> Reply {
    let res = {
        let mut w = sfu.world.lock().await;
        if let Some(r) = w.rooms.get(&room_id) {
            tracing::info!(kind = "ev", spec = "RULE-ROOM-CREATE-PROC", room = %room_id, "create of existing room — returned as is");
            return Reply::ok(&CreateRoomRes { room_id: r.id.clone(), name: r.name.clone(), capacity: r.capacity, created_at: r.created_at, media_codecs: r.media.clone() });
        }
        let media = media_of(&req.media_codecs);
        let r = Room::new(&room_id, &req.name, capacity, unused, departure, media.clone(), Sfu::wall(), Sfu::now());
        let hot = RoomHot::new(&room_id, &media, &sfu.boot.policy);
        sfu.put_hot(Arc::new(hot));
        let res = CreateRoomRes { room_id: room_id.clone(), name: r.name.clone(), capacity, created_at: r.created_at, media_codecs: media };
        w.rooms.insert(room_id.clone(), r);
        res
    };
    sfu.bus.declare_room(&room_id).await;
    tracing::info!(kind = "ev", spec = "CP-OP-CREATE-ROOM", room = %room_id, capacity, unused_ttl_secs = unused, departure_ttl_secs = departure, "room created");
    Reply::ok(&res)
}

pub async fn list(sfu: &Arc<Sfu>) -> Reply {
    let w = sfu.world.lock().await;
    Reply::ok(&RoomList { rooms: w.rooms.values().map(Room::summary).collect() })
}

pub async fn get(sfu: &Arc<Sfu>, room_id: &str, tracks: bool, user: Option<&str>) -> Reply {
    let w = sfu.world.lock().await;
    let Some(r) = w.rooms.get(room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
    let me = user.and_then(|u| r.member(u)).map(Member::info);
    let res = GetRoomRes {
        room_id: r.id.clone(),
        name: r.name.clone(),
        capacity: r.capacity,
        user_count: r.user_count(),
        participants: r.participants(),
        me,
        created_at: r.created_at,
        media_codecs: r.media.clone(),
        version: r.version(&sfu.epoch),
        tracks: tracks.then(|| tracks_of(r, sfu.room_hot(room_id).as_deref())),
    };
    Reply::ok(&res)
}

fn new_peer(sfu: &Sfu, who: &Who) -> Arc<Peer> {
    let pub_room = Arc::new(ArcSwap::from_pointee(None));
    let budget = Arc::new(Mutex::new(crate::rtcp::core::Budget::default()));
    let (pubt, subt) = match who.pc_mode {
        PcMode::V1pc => {
            let t = Transport::new(&sfu.net, &who.user, &who.session, Role::Single, who.pc_mode, &sfu.epoch, pub_room.clone(), budget.clone());
            (t.clone(), t)
        }
        PcMode::V2pc => (
            Transport::new(&sfu.net, &who.user, &who.session, Role::Publish, who.pc_mode, &sfu.epoch, pub_room.clone(), budget.clone()),
            Transport::new(&sfu.net, &who.user, &who.session, Role::Subscribe, who.pc_mode, &sfu.epoch, pub_room.clone(), budget.clone()),
        ),
    };
    Arc::new(Peer {
        user: who.user.clone(),
        session: who.session.clone(),
        home: Mutex::new(who.home.clone()),
        pc_mode: who.pc_mode,
        participant_type: who.participant_type,
        hidden: who.hidden,
        metadata: who.metadata.clone(),
        token_permission: who.permission,
        rooms: Mutex::new(BTreeSet::new()),
        pub_room,
        pubt,
        subt,
        tracks: ArcSwap::from_pointee(Vec::new()),
        budget,
        ext_tags: Mutex::new(None),
        created_ms: Sfu::now(),
    })
}

pub async fn join(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: RoomJoinReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return sfu.fail(e.code().failure_with(e.message(), None)),
    };
    let mut w = sfu.world.lock().await;
    let now = Sfu::now();
    if !w.rooms.contains_key(&req.room_id) {
        tracing::info!(kind = "ev", spec = "ROOM.S.join_3001", user = %who.user, room = %req.room_id, "join of unknown room");
        return sfu.fail(Code::RoomNotFound.failure());
    }
    let mut again = false;
    if let Some(old) = w.peers.get(&who.user).cloned() {
        if old.session != who.session {
            reap_peer(sfu, &mut w, &who.user, Path::Evict);
        } else if old.in_room(&req.room_id) {
            again = true;
            for r in old.rooms() {
                if r == req.room_id {
                    leave_seven(sfu, &mut w, &r, &who.user, Path::Evict, true);
                } else {
                    leave_seven(sfu, &mut w, &r, &who.user, Path::Evict, false);
                }
            }
            if let Some(p) = w.peers.remove(&who.user) {
                for tr in p.tracks() {
                    super::pubs::unpublish(sfu, &p, &tr);
                }
                p.close();
            }
            tracing::info!(kind = "ev", spec = "ROOM.S.join_again", user = %who.user, room = %req.room_id, "re-join — old peer evicted, new peer with new ICE credentials");
        }
    }
    let room = w.rooms.get(&req.room_id).map(|r| (r.user_count(), r.capacity));
    let Some((count, cap)) = room else { return sfu.fail(Code::RoomNotFound.failure()) };
    if !again && let Err(f) = check_capacity(count, cap, who.hidden) {
        tracing::info!(kind = "ev", spec = "ROOM.S.join_4001", user = %who.user, room = %req.room_id, "room full");
        return sfu.fail(f);
    }
    let current_rooms = w.peers.get(&who.user).map(|p| p.rooms().len()).unwrap_or(0);
    if let Err(f) = check_rooms(current_rooms) {
        return sfu.fail(f);
    }
    let fresh = !w.peers.contains_key(&who.user);
    let peer = match w.peers.get(&who.user) {
        Some(p) => p.clone(),
        None => new_peer(sfu, who),
    };
    if who.pc_mode == PcMode::V1pc
        && let (Some(ext), Some(codecs)) = (&req.extmap, &req.codecs)
    {
        let tags: Vec<Tag> = codecs.iter().map(|c| Tag { kind: c.kind, pt: c.pt, name: c.name.clone(), fmtp: c.fmtp.clone(), rtx_pt: c.rtx_pt }).collect();
        if !lock(&peer.subt.table).pts.seed_tags(&tags) {
            sfu.counters.drop("join_tags_ignored");
        }
        let mut slot = lock(&peer.ext_tags);
        if slot.is_none() {
            let _ = slot.replace(ext.clone());
        }
    }
    let hot = sfu.room_hot(&req.room_id);
    let snapshot_table = lock(&peer.subt.table).clone();
    {
        let Some(r) = w.rooms.get(&req.room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
        let Some(h) = hot.as_deref() else { return sfu.fail(Code::InternalError.failure()) };
        let slot_budget = sfu.boot.policy.media.slot_mid_reserve as usize;
        let occupied = lock(&peer.subt.table).occupied(RowClass::Slot, None);
        if occupied + 2 > slot_budget {
            if fresh {
                peer.close();
            }
            return sfu.fail(Code::MidLimit.failure_with("슬롯 몫", Some(json!({"limit": slot_budget, "current": occupied}))));
        }
        let mut short = false;
        for recv in subs::receivable(r, h, &who.user) {
            let ok = subs::place(sfu, &w, &peer.subt, &recv);
            if !ok && recv.class == RowClass::Slot {
                short = true;
            }
        }
        if short {
            let limit = lock(&peer.subt.table).budget();
            let current = lock(&peer.subt.table).rows.len();
            lock(&peer.subt.table).rollback_to(snapshot_table);
            if fresh {
                peer.close();
            }
            return sfu.fail(Code::MidLimit.failure_with("구독 몫", Some(json!({"limit": limit, "current": current}))));
        }
    }
    if fresh {
        w.peers.insert(who.user.clone(), peer.clone());
        sfu.sync_peers(&w);
    }
    lock(&peer.rooms).insert(req.room_id.clone());
    let select = req.select.unwrap_or(true);
    let (seq_label, version) = {
        let Some(r) = w.rooms.get_mut(&req.room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
        let permission = r.effective_permission(&who.user, who.permission);
        if !r.has(&who.user) {
            r.members.push(Member { user: who.user.clone(), role: req.role, select, participant_type: who.participant_type, metadata: who.metadata.clone(), hidden: who.hidden, permission, joined_ms: now, ready_at: 0 });
        } else if let Some(m) = r.member_mut(&who.user) {
            m.select = select;
            m.role = req.role;
            m.ready_at = 0;
        }
        r.ever_joined = true;
        r.empty_since = 0;
        let label = if again {
            "ROOM.S.join_again"
        } else if who.hidden {
            "ROOM.S.join_hidden"
        } else {
            r.bump();
            "ROOM.S.join"
        };
        (label, r.version(&sfu.epoch))
    };
    if !again && !who.hidden
        && let Some(r) = w.rooms.get(&req.room_id)
        && let Some(m) = r.member(&who.user)
    {
        let info = m.info();
        let body = ParticipantEventBody { type_: ParticipantEventBodyType::Joined, room_id: req.room_id.clone(), user_id: who.user.clone(), role: info.role, select: Some(info.select), participant_type: Some(info.participant_type), metadata: info.metadata, permission: info.permission, version: version.clone() };
        sfu.to_room(&req.room_id, Op::ParticipantEvent, serde_json::to_value(body).unwrap_or_default(), vec![who.user.clone()]);
    }
    if let Some(h) = &hot {
        let perm = w.rooms.get(&req.room_id).and_then(|r| r.member(&who.user)).map(|m| m.permission.floor_request).unwrap_or(true);
        let o = h.floor.join(&who.user, perm, peer.has_half(), peer.pub_room().as_deref() == Some(req.room_id.as_str()), now);
        crate::fwd::floor_out(sfu, h, o);
    }
    crate::sub::gate_lock(&peer.subt, &req.room_id, now);
    subs::sync(sfu, &w, &peer, false);
    if select {
        let (mv, to) = judge(peer.pub_room().as_deref(), Some(&req.room_id), None);
        if super::pubs::move_pub(sfu, &mut w, &peer, mv, to, true).is_err() {
            tracing::info!(kind = "ev", spec = "AFF.C.any.implicit_join_unselected", user = %who.user, room = %req.room_id, "join kept — publish room not moved");
        }
        subs::sync(sfu, &w, &peer, false);
    } else {
        tracing::debug!(kind = "ev", spec = "AFF.S.join_listen", user = %who.user, room = %req.room_id, "listen join");
    }
    let Some(r) = w.rooms.get(&req.room_id) else { return sfu.fail(Code::RoomNotFound.failure()) };
    let Some(me) = r.member(&who.user).map(Member::info) else { return sfu.fail(Code::InternalError.failure()) };
    let res = RoomJoinRes {
        room_id: req.room_id.clone(),
        participants: r.participants(),
        me,
        affiliation: peer.affiliation(),
        server_config: server_config(sfu, &peer),
        tracks: tracks_of(r, hot.as_deref()),
        version: r.version(&sfu.epoch),
        media_codecs: r.media.clone(),
        mline_table: lock(&peer.subt.table).wire(),
    };
    tracing::info!(kind = "ev", spec = seq_label, user = %who.user, session = %who.session, room = %req.room_id, seq = r.seq, hidden = who.hidden, pc_mode = who.pc_mode.as_str(), rows = res.mline_table.mlines.len(), "joined");
    Reply::ok(&res)
}

pub async fn leave(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: RoomLeaveReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return sfu.fail(e.code().failure_with(e.message(), None)),
    };
    let mut w = sfu.world.lock().await;
    if !w.rooms.contains_key(&req.room_id) {
        return sfu.fail(Code::RoomNotFound.failure());
    }
    if !w.rooms.get(&req.room_id).map(|r| r.has(&who.user)).unwrap_or(false) {
        tracing::info!(kind = "ev", spec = "ROOM.S.leave_3002", user = %who.user, room = %req.room_id, "leave of a room not joined");
        return sfu.fail(Code::NotInRoom.failure());
    }
    leave_seven(sfu, &mut w, &req.room_id, &who.user, Path::Leave, false);
    let aff = crate::gc::affiliation_after(w.peers.get(&who.user).map(|p| p.as_ref()));
    Reply::ok(&RoomLeaveRes { room_id: req.room_id, affiliation: aff })
}

pub async fn session_gone(sfu: &Arc<Sfu>, user: &str, session: &str, cause: &str) -> Reply {
    let mut w = sfu.world.lock().await;
    let same = w.peers.get(user).map(|p| p.session == session).unwrap_or(false);
    if same {
        let path = if cause == "evict" { Path::Evict } else { Path::Session };
        reap_peer(sfu, &mut w, user, path);
        tracing::info!(kind = "ev", spec = "SES.S.drop_reclaim", user = %user, session = %session, cause = %cause, "session gone — peer reclaimed");
    }
    Reply::Ok(json!({"reaped": same}))
}

pub async fn resume(sfu: &Arc<Sfu>, user: &str, session: &str, home: &Home) -> Reply {
    let w = sfu.world.lock().await;
    let Some(p) = w.peers.get(user).filter(|p| p.session == session).cloned() else {
        return Reply::ok(&ResumePart { rooms: Default::default(), publications: Vec::new(), mline_table: None });
    };
    p.set_home(home);
    let mut rooms = std::collections::BTreeMap::new();
    for rid in p.rooms() {
        if let Some(r) = w.rooms.get(&rid)
            && let Some(s) = snapshot(sfu, r, user)
        {
            rooms.insert(rid, s);
        }
    }
    let publications = p.tracks().iter().map(|t| t.publication()).collect();
    let table = lock(&p.subt.table).wire();
    tracing::info!(kind = "ev", spec = "IMPL-SES-RESUME", user = %user, session = %session, rooms = rooms.len(), "resume part");
    Reply::ok(&ResumePart { rooms, publications, mline_table: Some(table) })
}
