// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-SERVER-CONFIG · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-DERIVED · 정:IMPL-SES-RESUME · 정:IMPL-FLR-MEMBER-JOIN · 정:CAND-ROOM-JOIN-AGAIN-EDGE · 정:CAND-ROOM-TTL-PERMANENT-MARK · 연:RULE-ROOM-LEAVE-PROC · 연:RULE-ROOM-LEAVE-FAIL · 연:RULE-ROOM-SELECT · 연:RULE-ROOM-JOIN-NO-TYPE-CHECK · 연:RULE-ROOM-DETAIL-PROC · 연:RULE-ROOM-DETAIL-TWO-USES · 연:RULE-ROOM-DETAIL-FAIL · 연:RULE-ROOM-LIST-PROC · 연:RULE-ROOM-LIST-FORM · 연:RULE-ROOM-CREATE-FAIL · model: claude-opus-5-5
//! 방 op 조율 — 입장 차례 ②재입장 축출 ③정원 ④방 수 ⑤배정표(원자 — 모자라면 되돌림) ⑥명단 · `pub_room` · 통지 · 퇴장 · 재개 스냅샷 · 방 생성(멱등) · 상세 · 목록 · 세션 폐기.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::envelope::Envelope;
use oxsig::msg::*;
use oxsig::{Code, Failure};
use serde_json::{Value, json};

use super::handle::Ctx;
use crate::gc::Why;
use crate::link::{Peer, Role, Transport};
use crate::room::{Member, PERMANENT, Room, RoomInit};
use crate::sub::Table;

pub const MAX_ROOMS: usize = 100;

fn sfu() -> Result<&'static Arc<super::Sfu>, Failure> {
    super::sfu().ok_or_else(|| Code::InternalError.failure())
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Failure> {
    oxsig::code::body(v)
}

fn ok<T: serde::Serialize>(v: &T) -> Result<Value, Failure> {
    serde_json::to_value(v).map_err(|_| Code::InternalError.failure())
}

fn codecs_policy() -> Vec<CodecPolicy> {
    let v = |n: &str| CodecPolicy { kind: Kind::Video, name: n.to_string(), rtcp_fb: ["nack", "nack pli", "ccm fir", "transport-cc"].iter().map(|x| x.to_string()).collect() };
    let mut out = vec![CodecPolicy { kind: Kind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] }];
    out.extend(crate::trk::SUPPORTED_VIDEO.iter().map(|n| v(n)));
    out
}

pub fn server_config(s: &super::Sfu, p: &Peer) -> ServerConfig {
    let pc = p.publish.cred();
    let sc = p.subscribe.cred();
    ServerConfig {
        sfu_id: s.epoch.clone(),
        pc_mode: p.pc_mode,
        ice: ServerConfigIce {
            ip: s.ip.to_string(),
            port: s.udp_port,
            publish_ufrag: pc.ufrag,
            publish_pwd: pc.pwd,
            subscribe_ufrag: sc.ufrag,
            subscribe_pwd: sc.pwd,
            tcp_port: s.tcp_port,
        },
        dtls: ServerConfigDtls { fingerprint: s.fingerprint.clone(), setup: DtlsSetup::Passive },
        codecs: codecs_policy(),
        codecs_sub: None,
        extmap: crate::sub::server_extmap(),
        max_bitrate_bps: s.policy.media.max_bitrate_bps as u64,
    }
}

fn new_transport(s: &super::Sfu, user: &str, role: Role) -> Result<Arc<Transport>, Failure> {
    let m = crate::fwd::rt::media().ok_or_else(|| Code::InternalError.failure())?;
    let w = m.pick_worker(role != Role::Subscribe);
    let tr = Transport::new(s.next_id(), user, role, w);
    m.register(tr.cred().ufrag.as_str(), &tr);
    if let Some(wk) = m.workers.get(w)
        && let Ok(mut g) = wk.transports.lock()
    {
        g.push(Arc::downgrade(&tr));
    }
    let local = SocketAddr::new(s.ip, s.udp_port);
    let cert = s.cert.clone();
    let t2 = tr.clone();
    super::spawn(async move { crate::link::dtls::run(t2, cert, local).await });
    Ok(tr)
}

fn new_peer(c: &Ctx) -> Result<Arc<Peer>, Failure> {
    let s = sfu()?;
    let (publish, subscribe) = match c.pc_mode {
        PcMode::One => {
            let t = new_transport(s, &c.user, Role::Both)?;
            (t.clone(), t)
        }
        PcMode::Two => (new_transport(s, &c.user, Role::Publish)?, new_transport(s, &c.user, Role::Subscribe)?),
    };
    let table = Table::new(subscribe.clone(), c.pc_mode, s.policy.media.slot_mid_reserve as usize);
    s.tables.insert(table);
    let p = Arc::new(Peer {
        user: c.user.clone(),
        sid: c.sid.clone(),
        session: c.session.clone(),
        home: Mutex::new(c.home.clone().unwrap_or(common::envelope::Home { node: String::new(), inst: String::new() })),
        pc_mode: c.pc_mode,
        claims: c.claims.clone().unwrap_or_default(),
        publish,
        subscribe,
        rooms: Mutex::new(BTreeSet::new()),
        created_at: common::now_ms(),
        ever_sent_join: AtomicBool::new(false),
    });
    if let Ok(mut g) = s.peers.lock() {
        g.insert(c.user.clone(), p.clone());
    }
    super::transition("LINK", "PeerState", "SV-STATE-PEER-STATE", "-", "Alive", "peer_created", &[("user", &c.user)]);
    Ok(p)
}

pub fn snapshot_of(room: &Room, me: &Member) -> RoomSnapshot {
    RoomSnapshot { participants: room.participants(), me: me.me(), tracks: tracks_of(room), version: room.version() }
}

pub fn tracks_of(room: &Room) -> Vec<TrackEntry> {
    crate::trk::room_entries(&room.id, &room.media_codecs, &super::media::room_streams(&room.id))
}

pub fn join(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: RoomJoinReq = parse(body)?;
    let s = sfu()?;
    let room = s.rooms.get(&req.room_id).ok_or_else(|| Code::RoomNotFound.failure())?;
    let mut rejoin = false;
    if let Some(old) = s.peer(&c.user) {
        if old.in_room(&req.room_id) {
            crate::gc::evict(&old, Some(&req.room_id), Why::Evict);
            rejoin = room.has(&c.user);
        } else if old.sid != c.sid {
            crate::gc::evict(&old, None, Why::Evict);
        }
    }
    let hidden = c.hidden();
    if !rejoin && room.full_for(hidden) {
        return Err(Code::RoomFull.failure().with_details(serde_json::json!({ "capacity": room.capacity })));
    }
    let existing = s.peer(&c.user);
    if let Some(p) = &existing
        && p.rooms().len() >= MAX_ROOMS
    {
        return Err(Code::ListenLimit.failure().with_details(serde_json::json!({ "limit": MAX_ROOMS, "current": p.rooms().len() })));
    }
    let peer = match existing {
        Some(p) => p,
        None => new_peer(c)?,
    };
    let table = s.table(&peer).ok_or_else(|| Code::InternalError.failure())?;
    if peer.pc_mode == PcMode::One {
        let codecs = req.codecs.clone().unwrap_or_default();
        let extmap = req.extmap.clone().unwrap_or_default();
        if !table.seed(&codecs, &extmap) {
            tracing::info!(kind = "diag", user = %c.user, "1pc join number tags ignored (table exists)");
        }
    } else {
        table.mark_seeded();
    }
    if let Ok(mut g) = peer.rooms.lock() {
        g.insert(req.room_id.clone());
    }
    let out = super::media::resync_peer(&peer, false);
    if out.mid_limit {
        if let Ok(mut g) = peer.rooms.lock() {
            g.remove(&req.room_id);
        }
        super::media::resync_peer(&peer, false);
        if peer.rooms().is_empty() {
            crate::gc::reap(&peer, Why::Leave);
        }
        if rejoin && room.remove(&c.user).is_some() {
            room.announce_left(&c.user);
        }
        let (limit, current) = out.budget.unwrap_or((table.limit(), table.limit()));
        return Err(Code::MidLimit.failure().with_details(serde_json::json!({ "limit": limit, "current": current })));
    }
    let claims = c.claims.clone().unwrap_or_default();
    let select = req.select.unwrap_or(true);
    let member = Member {
        user: c.user.clone(),
        role: Some(req.role.unwrap_or(255)),
        select,
        participant_type: claims.participant_type,
        hidden,
        metadata: claims.metadata.clone(),
        permission: Permission::default(),
    };
    room.join(member, rejoin, claims.permission);
    let me = room.member(&c.user).ok_or_else(|| Code::InternalError.failure())?;
    let half = super::media::has_half(&c.user);
    let u = c.user.clone();
    let fr = me.permission.floor_request;
    super::floor::run(&room, |f, _| f.ws_join(&u, half, fr));
    let old_pub = crate::aff::pub_room(&peer);
    if select {
        match super::media::move_pub(&peer, Some(&room), claims.permission) {
            Ok(()) => {
                let label = if old_pub.is_some() && old_pub.as_deref() != Some(room.id.as_str()) { "AFF.S.join_select_moved" } else { "AFF.S.join_select" };
                super::media::aff_line(&c.user, label, old_pub.as_deref(), Some(&room.id), "ROOM_JOIN");
            }
            Err(f) => {
                let label = if f.code == Code::NotAuthorized.num() { "AFF.S.reject_perm" } else { "AFF.S.reject" };
                super::media::aff_line(&c.user, label, old_pub.as_deref(), old_pub.as_deref(), "ROOM_JOIN");
                tracing::info!(kind = "diag", user = %c.user, room = %room.id, "implicit pub_select refused ({})", f.name);
            }
        }
    } else {
        super::media::aff_line(&c.user, "AFF.S.join_listen", old_pub.as_deref(), old_pub.as_deref(), "ROOM_JOIN");
        if old_pub.as_deref() == Some(room.id.as_str()) {
            let u = c.user.clone();
            super::floor::run(&room, |f, _| f.ws_pub_select(&u));
        }
    }
    peer.ever_sent_join.store(true, Ordering::Release);
    let res = RoomJoinRes {
        room_id: room.id.clone(),
        participants: room.participants(),
        me: me.me(),
        affiliation: crate::aff::affiliation(&peer),
        server_config: server_config(s, &peer),
        tracks: tracks_of(&room),
        version: room.version(),
        media_codecs: room.media_codecs.clone(),
        mline_table: table.snapshot(&s.epoch),
    };
    ok(&res)
}

pub fn leave(c: &Ctx, body: &Value) -> Result<Value, Failure> {
    let req: RoomLeaveReq = parse(body)?;
    let (peer, _room) = super::media::joined(c, &req.room_id)?;
    if !crate::gc::leave(&peer, &req.room_id, Why::Leave, false) {
        return Err(Code::NotInRoom.failure());
    }
    ok(&RoomLeaveRes { room_id: req.room_id, affiliation: crate::aff::affiliation(&peer) })
}

pub fn session_gone(env: &Envelope) -> Result<Value, Failure> {
    let s = sfu()?;
    let user = env.user_id.clone().unwrap_or_default();
    let sid = env.sid.clone().unwrap_or_default();
    if let Some(p) = s.peer(&user)
        && (sid.is_empty() || p.sid == sid)
    {
        crate::gc::evict(&p, None, Why::SessionGone);
    }
    Ok(json!({}))
}

pub fn resume(env: &Envelope) -> Result<Value, Failure> {
    let s = sfu()?;
    let user = env.user_id.clone().unwrap_or_default();
    let Some(p) = s.peer(&user) else {
        return Ok(json!({ "snapshot": {}, "publications": [] }));
    };
    let mut snap = BTreeMap::new();
    for rid in p.rooms() {
        if let Some(room) = s.rooms.get(&rid)
            && let Some(m) = room.member(&user)
        {
            snap.insert(rid.clone(), snapshot_of(&room, &m));
        }
    }
    let pubs: Vec<Publication> = s
        .pubs
        .get(&user)
        .map(|c| c.streams())
        .unwrap_or_default()
        .iter()
        .map(|st| Publication {
            track_id: st.track_id.clone(),
            room_id: st.room.load().as_str().to_string(),
            kind: st.kind,
            duplex: st.duplex(),
            codec: Some(st.codec_name()),
            fmtp: st.fmtp.clone(),
            simulcast: st.simulcast,
        })
        .collect();
    let table = s.table(&p).map(|t| t.snapshot(&s.epoch));
    Ok(json!({ "snapshot": snap, "publications": pubs, "mline_table": table }))
}

pub async fn create(env: &Envelope) -> Result<Value, Failure> {
    let s = sfu()?;
    let req: CreateRoomReq = parse(&env.body)?;
    let id = req.room_id.clone().filter(|x| !x.is_empty()).ok_or_else(|| Code::MissingField.failure().with_message("room_id"))?;
    if let Some(r) = s.rooms.get(&id) {
        return ok(&CreateRoomRes { room_id: r.id.clone(), name: r.name.clone(), capacity: r.capacity, created_at: r.created_at, media_codecs: r.media_codecs.clone() });
    }
    let codecs = crate::trk::resolve_room_codecs(req.media_codecs.as_ref())
        .ok_or_else(|| Code::CodecRequired.failure().with_details(json!({ "supported": crate::trk::supported_list() })))?;
    let cap = req.capacity.unwrap_or(s.policy.hub.room_default_capacity);
    let pol_ttl = |v: u32| if v == 0 { PERMANENT } else { v };
    let room = Room::new(RoomInit {
        id: id.clone(),
        name: req.name.clone(),
        capacity: cap,
        unused_ttl: req.unused_ttl_secs.unwrap_or(pol_ttl(s.policy.hub.room_unused_ttl_secs)),
        departure_ttl: req.departure_ttl_secs.unwrap_or(pol_ttl(s.policy.hub.room_departure_ttl_secs)),
        media_codecs: codecs,
        epoch: s.epoch.clone(),
        floor: crate::flr::core::Floor::new(s.policy.floor.queue_max as usize, s.policy.floor.t2_stop_talking_secs as u32),
    });
    let room = s.rooms.insert(room);
    s.publish_slots();
    if super::floor::of(&room.id).is_none() {
        super::floor::open(&room);
    }
    if let Some(b) = crate::bus::get() {
        b.declare_room(&room.id).await;
    }
    tracing::info!(kind = "diag", room = %room.id, "room created (unused {} departure {})", room.unused_ttl, room.departure_ttl);
    ok(&CreateRoomRes { room_id: room.id.clone(), name: room.name.clone(), capacity: room.capacity, created_at: room.created_at, media_codecs: room.media_codecs.clone() })
}

pub fn get(env: &Envelope) -> Result<Value, Failure> {
    let s = sfu()?;
    let rid = env.body.get("room_id").and_then(|v| v.as_str()).ok_or_else(|| Code::MissingField.failure().with_message("room_id"))?;
    let tracks = env.body.get("tracks").and_then(|v| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0))).unwrap_or(false);
    let me_ok = env.body.get("me").and_then(|v| v.as_bool()).unwrap_or(false);
    let room = s.rooms.get(rid).ok_or_else(|| Code::RoomNotFound.failure())?;
    let me = if me_ok { env.user_id.as_deref().and_then(|u| room.member(u)).map(|m| m.me()) } else { None };
    let res = GetRoomRes {
        room_id: room.id.clone(),
        name: room.name.clone(),
        capacity: room.capacity,
        user_count: room.user_count(),
        participants: room.participants(),
        me,
        created_at: room.created_at,
        media_codecs: room.media_codecs.clone(),
        version: room.version(),
        tracks: if tracks { Some(tracks_of(&room)) } else { None },
    };
    ok(&res)
}

pub fn list() -> Result<Value, Failure> {
    let s = sfu()?;
    let mut rooms: Vec<RoomSummary> = s.rooms.all().iter().map(|r| r.summary()).collect();
    rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
    let total = rooms.len() as u32;
    ok(&ListRoomsRes { rooms, total })
}

pub async fn destroy_room(room: &Arc<Room>) {
    let Some(s) = super::sfu() else { return };
    room.close();
    s.rooms.remove(&room.id);
    s.publish_slots();
    super::floor::close(&room.id);
    for t in s.tables.map.lock().map(|g| g.values().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        t.forget_room(&room.id);
    }
    if let Some(b) = crate::bus::get() {
        b.undeclare_room(&room.id);
    }
    tracing::info!(kind = "diag", room = %room.id, "room removed");
}
