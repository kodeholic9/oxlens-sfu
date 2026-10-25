// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-ROOM-JOIN 정:IMPL-ROOM-LIFE 정:IMPL-ROOM-DERIVED 정:IMPL-ROOM-SERVER-CONFIG 정:IMPL-ROOM-RESYNC 정:IMPL-ROOM-VERSION-ISSUE 정:IMPL-SES-RESUME 정:IMPL-LINK-ICE-RESTART 정:IMPL-SUB-PIPELINE 정:IMPL-AFF-PUB-MOVE 정:RULE-GC-EDGES 연:ROOM.S.join 연:ROOM.S.join_again 연:ROOM.S.join_hidden 연:ROOM.S.join_3001 연:ROOM.S.join_4001 연:ROOM.S.leave 연:ROOM.S.leave_3002 연:ROOM.S.leave_hidden 연:LINK.S.restart 연:LINK.S.restart_3010 연:RULE-ROOM-JOIN-PROC 연:RULE-ROOM-MAX-100 연:RULE-ROOM-BUDGET-SPLIT 연:DEV-ROOM-ME-APART 연:RULE-SES-RESUME-PROC · model: claude-opus-5-5
//! 입장 ②재입장 축출 ③정원 ④방 수 ⑤배정표 줄(원자) ⑥명단 · 퇴장 · 세션 폐기 · RESUME 스냅샷 · ICE 재시작 · 방 생성 · 상세 · 목록.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::envelope::{Ctx, Reply};
use oxsig::msg::{
    CodecPolicy, CreateRoomReq, CreateRoomRes, GetRoomRes, IceRestartRes, IceRestartResPublish, IceRestartResSubscribe, Kind, ListRoomsRes, MediaCodecs,
    MemberInfo, ParticipantEventBody, ParticipantEventBodyType, PcMode, ResumeRes, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes, RoomSnapshot,
    ServerConfig, ServerConfigDtls, ServerConfigDtlsSetup, ServerConfigIce,
};
use oxsig::{Code, Op};
use serde_json::json;

use super::Sfu;
use super::call::body_json;
use super::peer::{Peer, PeerSt};
use crate::fwd::shell::now_ms;
use crate::fwd::{Conn, RoomMedia};
use crate::link::transport::{Role, Transport};
use crate::r#pub::Ctx as PubCtx;
use crate::room::{JoinCheck, MAX_ROOMS, Member, Room, Spec};
use crate::sub::Gate;
use crate::sub::table::{SubExt, Table};
use crate::trk::codec;

pub fn server_config(sfu: &Sfu, peer: &Peer) -> ServerConfig {
    let p = peer.pub_conn().link.cred();
    let s = peer.sub_conn().link.cred();
    let video_fb = vec!["nack".to_string(), "nack pli".to_string(), "ccm fir".to_string(), "transport-cc".to_string()];
    ServerConfig {
        sfu_id: sfu.epoch.clone(),
        pc_mode: peer.pc_mode,
        ice: ServerConfigIce {
            ip: sfu.cfg.public_ip.to_string(),
            port: sfu.cfg.udp_port,
            publish_ufrag: p.ufrag,
            publish_pwd: p.pwd,
            subscribe_ufrag: s.ufrag,
            subscribe_pwd: s.pwd,
            tcp_port: sfu.cfg.tcp_port,
        },
        dtls: ServerConfigDtls { fingerprint: sfu.cert.fingerprint.clone(), setup: ServerConfigDtlsSetup::Passive },
        codecs: vec![
            CodecPolicy { kind: Kind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] },
            CodecPolicy { kind: Kind::Video, name: "VP8".into(), rtcp_fb: video_fb.clone() },
            CodecPolicy { kind: Kind::Video, name: "H264".into(), rtcp_fb: video_fb },
        ],
        codecs_sub: None,
        extmap: crate::sub::table::server_extmap(),
        max_bitrate_bps: u64::from(sfu.cfg.policy.media.max_bitrate_bps),
    }
}

fn new_conn(sfu: &Arc<Sfu>, user: &str, role: Role, ext: SubExt, peer: std::sync::Weak<Peer>) -> Arc<Conn> {
    let id = sfu.next_id();
    let link = Arc::new(Transport::new(id, role, sfu.io.clone(), now_ms()));
    Arc::new(Conn {
        id,
        link,
        user: user.to_string(),
        peer,
        role,
        ingress: Default::default(),
        rtx_map: Default::default(),
        egress: Default::default(),
        acked: Default::default(),
        twcc_out: Default::default(),
        ext: arc_swap::ArcSwap::from_pointee(ext),
        twcc_in: Default::default(),
        twcc_id: Default::default(),
        media_ssrc: Default::default(),
        unknown_drop: Default::default(),
        bwe: Default::default(),
    })
}

pub fn new_peer(sfu: &Arc<Sfu>, ctx: &Ctx, req: &RoomJoinReq) -> Arc<Peer> {
    let one = ctx.pc_mode == PcMode::V1pc;
    let ext = if one { SubExt::build(req.extmap.as_deref()) } else { SubExt::build(None) };
    let mut table = Table::new(&sfu.epoch, one, sfu.cfg.policy.media.slot_mid_reserve, ext);
    if one && let Some(c) = &req.codecs {
        table.seed(c);
    }
    let st = PeerSt { pubs: PubCtx::default(), table, gates: Default::default(), prefs: Default::default() };
    let now = now_ms();
    Arc::new_cyclic(|w| {
        let conns = if one {
            vec![new_conn(sfu, &ctx.user_id, Role::Both, ext, w.clone())]
        } else {
            vec![new_conn(sfu, &ctx.user_id, Role::Publish, ext, w.clone()), new_conn(sfu, &ctx.user_id, Role::Subscribe, ext, w.clone())]
        };
        Peer::new(ctx, st, conns, now)
    })
}

fn register_peer(sfu: &Arc<Sfu>, p: &Arc<Peer>) {
    for c in &p.conns {
        sfu.plane.bind_ufrag(&c.link.ufrag(), c);
        tokio::spawn(crate::rtcp::shell::conn_loop(sfu.clone(), c.clone()));
    }
    tokio::spawn(super::floor::dc_loop(sfu.clone(), p.clone()));
    sfu.peers.insert(p.user.clone(), p.clone());
}

pub fn me_info(room: &Room, peer: &Peer) -> Option<MemberInfo> {
    room.lock().members.get(&peer.user).map(Member::info)
}

pub fn snapshot(room: &Room, peer: &Peer) -> Option<RoomSnapshot> {
    let st = room.lock();
    let me = st.members.get(&peer.user)?.info();
    Some(RoomSnapshot { participants: st.participants(), me, tracks: crate::trk::all(room, &st), version: room.version(st.seq) })
}

pub async fn create_room(sfu: &Arc<Sfu>, room_id: &str, req: CreateRoomReq) -> Reply {
    let _g = sfu.rooms_op.lock().await;
    if let Some(r) = sfu.room(room_id) {
        return Reply::ok(&CreateRoomRes {
            room_id: r.id.clone(),
            name: r.name.clone(),
            capacity: r.capacity,
            created_at: r.created_at,
            media_codecs: r.codecs.clone(),
        });
    }
    let mc = req.media_codecs.clone().unwrap_or(MediaCodecs { audio: None, video: None });
    let audio = mc.audio.as_deref().and_then(codec::canon).unwrap_or("opus").to_string();
    let video = mc.video.as_deref().and_then(codec::canon).unwrap_or("H264").to_string();
    let codecs = MediaCodecs { audio: Some(audio.clone()), video: Some(video.clone()) };
    let spec = Spec {
        id: room_id.to_string(),
        name: req.name.clone(),
        capacity: req.capacity.unwrap_or(sfu.cfg.policy.hub.room_default_capacity),
        codecs: codecs.clone(),
        unused_ttl: req.unused_ttl_secs.unwrap_or(crate::room::PERMANENT),
        departure_ttl: req.departure_ttl_secs.unwrap_or(crate::room::PERMANENT),
        queue_max: sfu.cfg.policy.floor.queue_max,
        t2_secs: sfu.cfg.policy.floor.t2_stop_talking_secs,
    };
    let room = Arc::new(Room::new(spec, &sfu.epoch, now_ms()));
    sfu.plane.add_room(Arc::new(RoomMedia::new(room_id, &audio, &video)));
    sfu.rooms.insert(room_id.to_string(), room.clone());
    crate::gc::start_floor(sfu, &room);
    if let Err(e) = sfu.bus.declare_room(room_id).await {
        tracing::error!(room = room_id, "room token declare failed {e}");
    }
    common::ev!(
        spec = "SV-STATE-UNIT-STATE",
        room = room_id,
        "room created capacity={} codecs={audio}/{video} unused_ttl={} departure_ttl={}",
        room.capacity,
        room.unused_ttl,
        room.departure_ttl
    );
    Reply::ok(&CreateRoomRes { room_id: room.id.clone(), name: room.name.clone(), capacity: room.capacity, created_at: room.created_at, media_codecs: codecs })
}

pub fn list_rooms(sfu: &Arc<Sfu>) -> Reply {
    let mut rooms: Vec<_> = sfu.rooms.iter().map(|r| r.summary()).collect();
    rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
    let total = rooms.len() as u32;
    Reply::ok(&ListRoomsRes { rooms, total })
}

pub fn get_room(sfu: &Arc<Sfu>, room_id: &str, tracks: bool, viewer: Option<&str>) -> Reply {
    let Some(room) = sfu.room(room_id) else { return Reply::fail(Code::RoomNotFound) };
    let st = room.lock();
    let me = viewer.and_then(|v| st.members.get(v)).map(Member::info);
    Reply::ok(&GetRoomRes {
        room_id: room.id.clone(),
        name: room.name.clone(),
        capacity: room.capacity,
        user_count: st.user_count(),
        participants: st.participants(),
        me,
        created_at: room.created_at,
        media_codecs: room.codecs.clone(),
        version: room.version(st.seq),
        tracks: tracks.then(|| crate::trk::all(&room, &st)),
    })
}

pub fn floor_perm(peer: &Peer, perm: &oxsig::msg::Permission) -> bool {
    let kinds = peer.lock().pubs.half_kinds();
    perm.floor_request
        && kinds.iter().all(|k| match k {
            Kind::Audio => perm.publish_audio,
            Kind::Video => perm.publish_video,
        })
}

pub async fn room_join(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let req: RoomJoinReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Reply::fail(Code::InvalidPayload),
    };
    let Some(room) = sfu.room(&req.room_id) else { return Reply::fail(Code::RoomNotFound) };
    let _g = sfu.rooms_op.lock().await;
    let mut kept = false;
    let existing = sfu.peer(&ctx.user_id);
    let peer = match existing {
        Some(p) if p.session() == ctx.session && !p.aff.has(&room.id) => {
            if req.codecs.is_some() || req.extmap.is_some() {
                sfu.drops.num_tags_ignored.fetch_add(1, Ordering::Relaxed);
            }
            p
        }
        Some(old) => {
            let keep = (old.session() == ctx.session && old.aff.has(&room.id)).then(|| room.id.clone());
            kept = keep.is_some();
            crate::gc::evict(sfu, &old, keep.as_deref()).await;
            let p = new_peer(sfu, ctx, &req);
            register_peer(sfu, &p);
            p
        }
        None => {
            let p = new_peer(sfu, ctx, &req);
            register_peer(sfu, &p);
            p
        }
    };
    let _op = peer.op.lock().await;
    let fresh = peer.aff.count() == 0;
    if !kept && matches!(crate::room::capacity_check(&room.lock(), ctx.hidden, room.capacity), JoinCheck::Full) {
        if fresh {
            crate::gc::reap(sfu, &peer, "join_failed");
        }
        return Reply::fail(Code::RoomFull);
    }
    if peer.aff.count() >= MAX_ROOMS {
        return Reply::failure(Code::ListenLimit.failure().with_details(json!({ "limit": MAX_ROOMS, "current": peer.aff.count() })));
    }
    let now = now_ms();
    let slots = {
        let mut st = peer.lock();
        let r = st.table.add_slots(&room.id, room.codec_of(Kind::Audio), room.codec_of(Kind::Video), now);
        if r.is_err() {
            st.table.discard();
        }
        r
    };
    if let Err(m) = slots {
        if fresh {
            crate::gc::reap(sfu, &peer, "join_failed");
        }
        return Reply::failure(Code::MidLimit.failure().with_details(json!({ "limit": m.limit, "current": m.current })));
    }
    let select = req.select.unwrap_or(true);
    let role = req.role.unwrap_or(crate::room::DEFAULT_ROLE);
    let perm = {
        let mut st = room.lock();
        let perm = st.perm_for(&ctx.user_id, ctx.permission.as_ref());
        let was = st.members.contains_key(&ctx.user_id);
        let m = Member {
            user: ctx.user_id.clone(),
            role,
            select,
            ptype: ctx.participant_type,
            hidden: ctx.hidden,
            metadata: ctx.metadata.clone(),
            perm: perm.clone(),
            joined_at: now,
        };
        st.members.insert(ctx.user_id.clone(), m.clone());
        if !was && !ctx.hidden {
            let seq = st.bump();
            let ev = ParticipantEventBody {
                type_: ParticipantEventBodyType::Joined,
                room_id: room.id.clone(),
                user_id: m.user.clone(),
                role: Some(m.role),
                select: Some(m.select),
                participant_type: Some(m.ptype),
                metadata: m.metadata.clone(),
                permission: m.info().permission,
                version: room.version(seq),
            };
            sfu.emit_room(&room.id, Op::ParticipantEvent, &ev, vec![ctx.user_id.clone()]);
        }
        perm
    };
    room.joined();
    peer.aff.add(&room.id);
    peer.lock().gates.entry(room.id.clone()).or_insert_with(|| Arc::new(Gate::new(now)));
    let half = peer.lock().pubs.has_half();
    let fperm = floor_perm(&peer, &perm);
    room.floor.join(&peer.user, false, half, fperm);
    let label = if kept {
        "ROOM.S.join_again"
    } else if ctx.hidden {
        "ROOM.S.join_hidden"
    } else {
        "ROOM.S.join"
    };
    common::log::transition(&common::log::Step {
        area: "ROOM",
        key: &room.id,
        from: "NONE",
        to: Some("JOINED"),
        cause: "ROOM_JOIN",
        label: Some(label),
        ins: &[
            ("hidden", ctx.hidden.to_string()),
            ("pcm", serde_json::to_value(ctx.pc_mode).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()),
        ],
        room: Some(&room.id),
        user: Some(&ctx.user_id),
        arev: None,
    });
    if select && let Err(f) = super::aff::move_pub(sfu, &peer, Some(&room.id)).await {
        tracing::info!(user = ctx.user_id.as_str(), room = room.id.as_str(), "join select not moved code={}", f.code);
    }
    let wants = super::publish::receivable(sfu, &room, &peer);
    {
        let mut st = peer.lock();
        for w in wants {
            st.table.want(w, now);
        }
        let _ = st.table.finish();
    }
    super::sub::sync(sfu, &peer);
    let (participants, tracks, version) = {
        let st = room.lock();
        (st.participants(), crate::trk::all(&room, &st), room.version(st.seq))
    };
    let Some(me) = me_info(&room, &peer) else { return Reply::fail(Code::InternalError) };
    let res = RoomJoinRes {
        room_id: room.id.clone(),
        participants,
        me,
        affiliation: peer.aff.snapshot(),
        server_config: server_config(sfu, &peer),
        tracks,
        version,
        media_codecs: room.codecs.clone(),
        mline_table: peer.lock().table.wire(),
    };
    Reply::ok(&res)
}

pub async fn room_leave(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let req: RoomLeaveReq = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Reply::fail(Code::InvalidPayload),
    };
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.aff.has(&req.room_id)) else { return Reply::fail(Code::NotInRoom) };
    let Some(room) = sfu.room(&req.room_id) else { return Reply::fail(Code::NotInRoom) };
    let _op = peer.op.lock().await;
    crate::gc::leave(sfu, &peer, &room, crate::gc::Why::Leave, true).await;
    let aff = peer.aff.snapshot();
    if peer.aff.count() == 0 {
        crate::gc::reap(sfu, &peer, "last_room");
    }
    Reply::ok(&RoomLeaveRes { room_id: req.room_id, affiliation: aff })
}

pub async fn session_gone(sfu: &Arc<Sfu>, user: &str, session: &str) -> Reply {
    if let Some(p) = sfu.peer(user)
        && p.session() == session
    {
        let _g = sfu.rooms_op.lock().await;
        crate::gc::drop_peer(sfu, &p, crate::gc::Why::SessionGone).await;
    }
    Reply::empty()
}

pub async fn resume(sfu: &Arc<Sfu>, ctx: &Ctx) -> Reply {
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.session() == ctx.session) else { return Reply::fail(Code::PeerNotFound) };
    peer.follow_home(&ctx.home);
    let _op = peer.op.lock().await;
    let mut snap = BTreeMap::new();
    for r in peer.aff.rooms() {
        if let Some(room) = sfu.room(&r)
            && let Some(s) = snapshot(&room, &peer)
        {
            snap.insert(r, s);
        }
    }
    if snap.is_empty() {
        return Reply::fail(Code::PeerNotFound);
    }
    let publications = peer.lock().pubs.streams.iter().map(|s| s.publication()).collect();
    let mline_tables = vec![peer.lock().table.wire()];
    Reply::ok(&ResumeRes { snapshot: snap, publications, mline_tables })
}

pub async fn ice_restart(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let v = match body_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if v.get("sfu_id").and_then(|x| x.as_str()).is_some_and(|s| s != sfu.epoch) {
        return Reply::fail(Code::PeerNotFound);
    }
    let Some(peer) = sfu.peer(&ctx.user_id).filter(|p| p.session() == ctx.session) else { return Reply::fail(Code::PeerNotFound) };
    let _op = peer.op.lock().await;
    let mut creds = Vec::new();
    for c in &peer.conns {
        let (cred, replaced) = c.link.restart();
        if let Some(old) = replaced {
            sfu.plane.unbind_ufrag(&old);
        }
        sfu.plane.bind_ufrag(&cred.ufrag, c);
        creds.push(cred);
    }
    common::log::transition(&common::log::Step {
        area: "LINK",
        key: &ctx.user_id,
        from: "current",
        to: Some("restart"),
        cause: "ICE_RESTART",
        label: Some("LINK.S.restart"),
        ins: &[],
        room: None,
        user: Some(&ctx.user_id),
        arev: None,
    });
    let publish = IceRestartResPublish { ufrag: creds[0].ufrag.clone(), pwd: creds[0].pwd.clone() };
    let subscribe = creds.get(1).map(|c| IceRestartResSubscribe { ufrag: c.ufrag.clone(), pwd: c.pwd.clone() });
    Reply::ok(&IceRestartRes { publish, subscribe })
}
