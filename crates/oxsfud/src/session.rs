// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-ROOM-JOIN 정:IMPL-ROOM-SERVER-CONFIG 정:RULE-GC-LEAVE-SEVEN 정:IMPL-GC-PEER-REAP 정:RULE-GC-EDGES 정:IMPL-SES-RESUME 정:IMPL-LINK-ICE-RESTART 정:IMPL-BUS-HOME-LOST 정:IMPL-OPS-DESTROY-REAP 정:IMPL-AUX-MESSAGE 정:IMPL-AFF-FORCED 연:RULE-ROOM-JOIN-PROC · model: claude-opus-5-5
//! 참여 — 입장 판정 차례 · 퇴장 정리 일곱 단계(세 경로가 한 절차로) · Peer 거두기 · 재개 스냅샷.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use oxsig::Op;
use oxsig::bplane::{Caller, DestroyRes, EndPath, ReapRes, ResumePart};
use oxsig::code::Code;
use oxsig::wire::{
    CodecPolicy, IceRestartRes, IceRestartResPublish, IceRestartResSubscribe, Kind, MessageBody, MessageReq, MessageRes,
    ParticipantEventBody, ParticipantEventBodyType, PcMode, RoomEventBody, RoomEventBodyAffiliationCause, RoomJoinReq,
    RoomJoinRes, RoomLeaveReq, RoomLeaveRes, ServerConfig, ServerConfigDtls, ServerConfigDtlsSetup, ServerConfigIce,
};
use serde_json::{Value, json};

use crate::peer::{Conn, ExtIds, JoinTags, Peer, Role};
use crate::publish::Fail;
use crate::room::{Member, Room};
use crate::sfu::{Sfu, lock};

const MAX_ROOMS: usize = 100;

fn fail(c: Code, m: impl Into<String>) -> Fail {
    (c, m.into(), None)
}

fn make_peer(sfu: &Arc<Sfu>, c: &Caller, tags: Option<JoinTags>) -> Arc<Peer> {
    let one = c.pc_mode == PcMode::V1pc;
    let pub_conn = Conn::new(sfu, if one { Role::Both } else { Role::Publish }, &c.user_id, if one { 32 } else { 0 });
    let sub_conn = if one { pub_conn.clone() } else { Conn::new(sfu, Role::Subscribe, &c.user_id, 0) };
    if one && let Some(t) = &tags {
        sub_conn.sub_ext.store(Arc::new(ExtIds::from_table(&t.extmap)));
        lock(&sub_conn.table).seed_tags(t);
    }
    let peer = Arc::new(Peer {
        session_id: c.session_id.clone(),
        user_id: c.user_id.clone(),
        pc_mode: c.pc_mode,
        home: c.home.clone(),
        pub_conn: pub_conn.clone(),
        sub_conn: sub_conn.clone(),
        rooms: Mutex::new(BTreeSet::new()),
        pub_room: ArcSwapOption::empty(),
        streams: Mutex::new(Vec::new()),
        ready: Mutex::new(HashMap::new()),
        alive: AtomicBool::new(true),
    });
    let _ = pub_conn.peer.set(Arc::downgrade(&peer));
    if !one {
        let _ = sub_conn.peer.set(Arc::downgrade(&peer));
    }
    lock(&sfu.peers).insert(c.session_id.clone(), peer.clone());
    peer
}

pub fn server_config(sfu: &Sfu, peer: &Peer) -> ServerConfig {
    let (pu, pp) = lock(&peer.pub_conn.creds).cur.clone();
    let (su, sp) = lock(&peer.sub_conn.creds).cur.clone();
    let video_fb = vec!["nack".to_string(), "nack pli".to_string(), "ccm fir".to_string(), "transport-cc".to_string()];
    ServerConfig {
        sfu_id: sfu.epoch.clone(),
        pc_mode: peer.pc_mode,
        ice: ServerConfigIce {
            ip: sfu.ip.to_string(),
            port: sfu.udp_port,
            publish_ufrag: pu,
            publish_pwd: pp,
            subscribe_ufrag: su,
            subscribe_pwd: sp,
            tcp_port: sfu.tcp_port,
        },
        dtls: ServerConfigDtls { fingerprint: sfu.fingerprint.clone(), setup: ServerConfigDtlsSetup::Passive },
        codecs: vec![
            CodecPolicy { kind: Kind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] },
            CodecPolicy { kind: Kind::Video, name: "VP8".into(), rtcp_fb: video_fb.clone() },
            CodecPolicy { kind: Kind::Video, name: "H264".into(), rtcp_fb: video_fb },
        ],
        codecs_sub: None,
        extmap: crate::peer::server_extmap(),
        max_bitrate_bps: sfu.policy.media.max_bitrate_bps as u64,
    }
}

pub fn join(sfu: &Arc<Sfu>, c: &Caller, body: &str) -> Result<String, Fail> {
    let req: RoomJoinReq = serde_json::from_str(body).map_err(|e| fail(Code::INVALID_PAYLOAD, e.to_string()))?;
    let room = sfu.room(&req.room_id).filter(|r| !r.closed.load(Ordering::SeqCst)).ok_or_else(|| fail(Code::ROOM_NOT_FOUND, "room not found"))?;
    for p in sfu.peer_of_user(&c.user_id) {
        if p.session_id != c.session_id {
            end_peer(sfu, &p, EndPath::Evict);
        }
    }
    let mut peer = sfu.peer(&c.session_id);
    if let Some(p) = &peer
        && p.in_room(&room.id)
    {
        end_peer(sfu, p, EndPath::Evict);
        peer = None;
    }
    {
        let g = room.lock();
        if !c.hidden && room.user_count(&g) >= room.capacity {
            return Err((Code::ROOM_FULL, "room full".into(), Some(json!({ "capacity": room.capacity }))));
        }
    }
    if let Some(p) = &peer {
        let n = p.rooms.lock().map(|r| r.len()).unwrap_or(0);
        if n >= MAX_ROOMS {
            return Err((Code::LISTEN_LIMIT, "room limit".into(), Some(json!({ "limit": MAX_ROOMS, "current": n }))));
        }
        let (limit, used) = crate::table::slot_budget(sfu, p);
        if used + 2 > limit {
            return Err((Code::MID_LIMIT, "slot budget".into(), Some(json!({ "limit": limit, "current": used }))));
        }
    }
    let peer = match peer {
        Some(p) => p,
        None => {
            let tags = (c.pc_mode == PcMode::V1pc).then(|| JoinTags { extmap: req.extmap.clone().unwrap_or_default(), codecs: req.codecs.clone().unwrap_or_default() });
            make_peer(sfu, c, tags)
        }
    };
    let select = req.select.unwrap_or(true);
    let me = {
        let mut g = room.lock();
        let permission = g.perm.get(&c.user_id).copied().unwrap_or(c.permission);
        let m = Member {
            user_id: c.user_id.clone(),
            session_id: c.session_id.clone(),
            role: req.role.unwrap_or(255),
            select,
            participant_type: c.participant_type,
            metadata: c.metadata.clone(),
            permission,
            hidden: c.hidden,
        };
        let info = m.info();
        g.members.insert(c.user_id.clone(), m);
        room.ever_joined.store(true, Ordering::SeqCst);
        room.empty_since.store(0, Ordering::SeqCst);
        lock(&peer.rooms).insert(room.id.clone());
        if !c.hidden {
            let rid = room.id.clone();
            let i = info.clone();
            room.emit(sfu, &g, Op::ParticipantEvent, Some(&c.user_id), |version| ParticipantEventBody {
                type_: ParticipantEventBodyType::Joined,
                room_id: rid,
                user_id: i.user_id.clone(),
                role: i.role,
                select: Some(i.select),
                participant_type: Some(i.participant_type),
                metadata: i.metadata.clone(),
                permission: i.permission,
                version,
            });
        }
        info
    };
    crate::floor::join(sfu, &room, &c.user_id);
    lock(&peer.ready).insert(room.id.clone(), crate::peer::Ready::default());
    if select {
        crate::publish::move_pub(sfu, &peer, &room);
    }
    let filled = crate::table::fill(sfu, &peer, false);
    let (participants, tracks, version) = {
        let g = room.lock();
        (room.visible(&g), room.tracks(&g), room.version(sfu))
    };
    tracing::info!(kind = "ev", spec = "IMPL-ROOM-JOIN", user = c.user_id.as_str(), room = room.id.as_str(), session = c.session_id.as_str(), "joined");
    let res = RoomJoinRes {
        room_id: room.id.clone(),
        participants,
        me,
        affiliation: peer.affiliation(),
        server_config: server_config(sfu, &peer),
        tracks,
        version,
        media_codecs: room.codecs.clone(),
        mline_table: filled.table,
    };
    Ok(serde_json::to_string(&res).unwrap_or_default())
}

pub fn leave_room(sfu: &Arc<Sfu>, peer: &Arc<Peer>, room: &Arc<Room>, path: EndPath) -> bool {
    crate::floor::leave(sfu, room, &peer.user_id);
    let hidden = {
        let mut g = room.lock();
        match g.members.get(&peer.user_id) {
            Some(m) if m.session_id == peer.session_id => {
                let h = m.hidden;
                g.members.remove(&peer.user_id);
                Some(h)
            }
            _ => None,
        }
    };
    let Some(hidden) = hidden else {
        tracing::warn!(kind = "diag", spec = "RULE-GC-EDGES", user = peer.user_id.as_str(), room = room.id.as_str(), "leave: not a member");
        lock(&peer.rooms).remove(&room.id);
        return false;
    };
    lock(&peer.rooms).remove(&room.id);
    lock(&peer.ready).remove(&room.id);
    if peer.pub_room_id().as_deref() == Some(room.id.as_str()) {
        peer.pub_room.store(None);
    }
    let last = lock(&peer.rooms).is_empty();
    let full: Vec<Arc<crate::media::PubStream>> = lock(&peer.streams).iter().filter(|s| s.listed.load(Ordering::Relaxed) && s.room() == room.id).cloned().collect();
    let mut touched = crate::publish::unlist(sfu, room, &full);
    for s in &full {
        crate::publish::drop_stream(sfu, peer, s);
    }
    if last {
        reap(sfu, peer, path);
    } else {
        crate::table::fill(sfu, peer, true);
    }
    if !hidden {
        let g = room.lock();
        let rid = room.id.clone();
        let uid = peer.user_id.clone();
        room.emit(sfu, &g, Op::ParticipantEvent, Some(&peer.user_id), |version| ParticipantEventBody {
            type_: ParticipantEventBodyType::Left,
            room_id: rid,
            user_id: uid,
            role: None,
            select: None,
            participant_type: None,
            metadata: None,
            permission: None,
            version,
        });
    }
    touched.retain(|p| !Arc::ptr_eq(p, peer));
    crate::publish::refill_peers(sfu, touched);
    tracing::info!(
        kind = "ev",
        spec = "RULE-GC-LEAVE-SEVEN",
        user = peer.user_id.as_str(),
        room = room.id.as_str(),
        path = format!("{path:?}").as_str(),
        link = peer.pub_conn.state_name(),
        last = last,
        "left"
    );
    true
}

fn reap(sfu: &Arc<Sfu>, peer: &Arc<Peer>, path: EndPath) {
    if !peer.alive.swap(false, Ordering::SeqCst) {
        return;
    }
    {
        let mut m = lock(&sfu.peers);
        if m.get(&peer.session_id).is_some_and(|p| Arc::ptr_eq(p, peer)) {
            m.remove(&peer.session_id);
        }
    }
    let streams: Vec<Arc<crate::media::PubStream>> = lock(&peer.streams).clone();
    for s in &streams {
        if s.listed.load(Ordering::Relaxed)
            && let Some(r) = sfu.room(&s.room())
        {
            let t = crate::publish::unlist(sfu, &r, std::slice::from_ref(s));
            crate::publish::refill_peers(sfu, t);
        }
        crate::publish::drop_stream(sfu, peer, s);
    }
    for c in peer.conns() {
        crate::table::clear(&c);
        sfu.unindex_conn(&c);
        crate::net::close_links_of(&c);
        c.cancel.cancel();
        lock(&c.dtls_in).take();
        lock(&c.dc.tx).take();
    }
    tracing::info!(kind = "ev", spec = "IMPL-GC-PEER-REAP", user = peer.user_id.as_str(), session = peer.session_id.as_str(), path = format!("{path:?}").as_str(), "peer reaped");
}

pub fn end_peer(sfu: &Arc<Sfu>, peer: &Arc<Peer>, path: EndPath) {
    for r in peer.room_ids() {
        if let Some(room) = sfu.room(&r) {
            leave_room(sfu, peer, &room, path);
        }
    }
    reap(sfu, peer, path);
}

pub fn leave(sfu: &Arc<Sfu>, c: &Caller, body: &str) -> Result<String, Fail> {
    let req: RoomLeaveReq = serde_json::from_str(body).map_err(|e| fail(Code::INVALID_PAYLOAD, e.to_string()))?;
    let peer = sfu.peer(&c.session_id).filter(|p| p.in_room(&req.room_id)).ok_or_else(|| fail(Code::NOT_IN_ROOM, "not in room"))?;
    let room = sfu.room(&req.room_id).ok_or_else(|| fail(Code::NOT_IN_ROOM, "not in room"))?;
    leave_room(sfu, &peer, &room, EndPath::Leave);
    let res = RoomLeaveRes { room_id: req.room_id, affiliation: peer.affiliation() };
    Ok(serde_json::to_string(&res).unwrap_or_default())
}

pub fn session_end(sfu: &Arc<Sfu>, _user: &str, sid: &str, path: EndPath) {
    if let Some(p) = sfu.peer(sid) {
        end_peer(sfu, &p, path);
    }
    lock(&sfu.arevs).remove(sid);
}

pub fn resume(sfu: &Arc<Sfu>, user: &str, sid: &str) -> ResumePart {
    let mut part = ResumePart { snapshot: Default::default(), publications: vec![], mline_tables: vec![] };
    let Some(p) = sfu.peer(sid).filter(|p| p.user_id == user) else { return part };
    for r in p.room_ids() {
        if let Some(room) = sfu.room(&r)
            && let Some(s) = room.snapshot(sfu, user)
        {
            part.snapshot.insert(r, s);
        }
    }
    part.publications = crate::publish::publications(&p);
    part.mline_tables.push(crate::table::table_now(sfu, &p));
    part
}

pub fn ice_restart(sfu: &Arc<Sfu>, c: &Caller, _body: &str) -> Result<String, Fail> {
    let peer = sfu.peer(&c.session_id).ok_or_else(|| fail(Code::PEER_NOT_FOUND, "peer not found"))?;
    let (pu, pp) = peer.pub_conn.restart(sfu, &peer.pub_conn);
    let subscribe = if Arc::ptr_eq(&peer.pub_conn, &peer.sub_conn) {
        None
    } else {
        let (su, sp) = peer.sub_conn.restart(sfu, &peer.sub_conn);
        Some(IceRestartResSubscribe { ufrag: su, pwd: sp })
    };
    tracing::info!(kind = "ev", spec = "IMPL-LINK-ICE-RESTART", user = c.user_id.as_str(), ufrag = pu.as_str(), "ice restart");
    let res = IceRestartRes { publish: IceRestartResPublish { ufrag: pu, pwd: pp }, subscribe };
    Ok(serde_json::to_string(&res).unwrap_or_default())
}

pub fn message(sfu: &Arc<Sfu>, c: &Caller, body: &str) -> Result<String, Fail> {
    let req: MessageReq = serde_json::from_str(body).map_err(|e| {
        if e.to_string().contains("missing field") { fail(Code::MISSING_FIELD, e.to_string()) } else { fail(Code::INVALID_PAYLOAD, e.to_string()) }
    })?;
    let room = sfu.room(&req.room_id).ok_or_else(|| fail(Code::ROOM_NOT_FOUND, "room not found"))?;
    if !sfu.peer(&c.session_id).is_some_and(|p| p.in_room(&req.room_id)) {
        return Err(fail(Code::NOT_IN_ROOM, "not in room"));
    }
    {
        let g = room.lock();
        let b = MessageBody { room_id: req.room_id.clone(), user_id: c.user_id.clone(), content: req.content };
        room.bare(sfu, &g, Op::Message, Some(&c.user_id), &b);
    }
    Ok(serde_json::to_string(&MessageRes { msg_id: uuid::Uuid::new_v4().to_string() }).unwrap_or_default())
}

pub fn affiliation(sfu: &Arc<Sfu>, c: &Caller, body: &str) -> Result<String, Fail> {
    match sfu.peer(&c.session_id) {
        Some(p) => crate::publish::affiliation(sfu, &p, body),
        None => {
            let req: oxsig::wire::AffiliationReq = serde_json::from_str(body).map_err(|e| fail(Code::INVALID_PAYLOAD, e.to_string()))?;
            let res = oxsig::wire::AffiliationRes {
                sub_rooms: vec![],
                pub_room: None,
                cause: oxsig::wire::AffiliationResCause::User,
                change_id: req.change_id,
            };
            Ok(serde_json::to_string(&res).unwrap_or_default())
        }
    }
}

fn affiliation_event(sfu: &Sfu, peer: &Peer, room: &str, cause: RoomEventBodyAffiliationCause) {
    let body = RoomEventBody::Affiliation { room_id: room.to_string(), affiliation: peer.affiliation(), cause };
    sfu.emit_user(&peer.user_id, Op::RoomEvent, &body);
}

pub fn kick(sfu: &Arc<Sfu>, room_id: &str, user_id: &str) -> Result<ReapRes, (Code, String)> {
    let room = sfu.room(room_id).ok_or((Code::ROOM_NOT_FOUND, "room not found".to_string()))?;
    let sid = room.lock().members.get(user_id).map(|m| m.session_id.clone()).ok_or((Code::NOT_IN_ROOM, "not in room".to_string()))?;
    let peer = sfu.peer(&sid).ok_or((Code::NOT_IN_ROOM, "not in room".to_string()))?;
    let left = leave_room(sfu, &peer, &room, EndPath::Cut);
    affiliation_event(sfu, &peer, room_id, RoomEventBodyAffiliationCause::Kick);
    Ok(ReapRes { room_id: room_id.to_string(), user_id: user_id.to_string(), left, version: room.version(sfu) })
}

pub fn destroy(sfu: &Arc<Sfu>, room_id: &str, if_version: Option<&str>) -> Result<DestroyRes, (Code, String, Option<Value>)> {
    let room = sfu.room(room_id).ok_or((Code::ROOM_NOT_FOUND, "room not found".to_string(), None))?;
    let Some(want) = if_version else {
        return Err((Code::MISSING_FIELD, "if_version".into(), None));
    };
    let v = room.version(sfu);
    let actual = format!("{}:{}", v.epoch, v.seq);
    if want != actual {
        return Err((Code::PRECONDITION_FAILED, "version mismatch".into(), Some(json!({ "expected": want, "actual": actual }))));
    }
    let sids: Vec<String> = room.lock().members.values().map(|m| m.session_id.clone()).collect();
    let mut notified = 0;
    for sid in sids {
        if let Some(p) = sfu.peer(&sid) {
            leave_room(sfu, &p, &room, EndPath::Cut);
            affiliation_event(sfu, &p, room_id, RoomEventBodyAffiliationCause::RoomClosed);
            notified += 1;
        }
    }
    crate::room::remove(sfu, &room);
    Ok(DestroyRes { room_id: room_id.to_string(), destroyed: true, notified })
}

pub fn home_lost(sfu: &Arc<Sfu>, node: &str, inst: &str) {
    let peers: Vec<Arc<Peer>> = lock(&sfu.peers).values().filter(|p| p.home.node == node && p.home.inst == inst).cloned().collect();
    for p in peers {
        tracing::info!(kind = "ev", spec = "IMPL-BUS-HOME-LOST", user = p.user_id.as_str(), node = node, "home lost — reap");
        end_peer(sfu, &p, EndPath::Window);
    }
}
