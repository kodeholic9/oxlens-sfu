// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-SERVER-CONFIG · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-RESYNC · 정:IMPL-SES-RESUME · 정:IMPL-ROOM-PERMISSION · 연:ROOM.S.join · 연:ROOM.S.join_again · 연:ROOM.S.join_hidden · 연:ROOM.S.leave · 연:RULE-ROOM-JOIN-NUMBER-TAGS · model: claude-opus-5-5
//! 입장(②재입장 축출 ③정원 ④서버별 방 수 ⑤배정표 ⑥명단 · `seq++` · 통지) · 퇴장 · `RESUME` 스냅샷 · 방 생성(멱등 · `room` 토큰) · 방 상세.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::bus::Notice;
use common::handle::{Envelope, Reply};
use oxsig::err::{ErrCode, Failure};
use oxsig::link::{CodecPolicy, ExtMap, ServerConfig, EXT_ABS_SEND_TIME, EXT_AUDIO_LEVEL, EXT_MID, EXT_TWCC, SERVER_EXTMAP};
use oxsig::room::{CreateRoomReq, CreateRoomRes, GetRoomRes, MediaCodecs, MemberInfo, ParticipantEvent, ParticipantEventType, RoomJoinReq, RoomJoinRes, RoomLeaveReq, RoomLeaveRes, RoomSnapshot};
use oxsig::ses::{PcMode, ResumeRes};
use oxsig::trk::MediaKind;
use serde_json::{json, Value};

use super::{fail, wire};
use crate::cfg::Ctx;
use crate::gc::{self, Path};
use crate::link::{lock, Ident, NewPeer, Peer, SubExt};
use crate::room::{Member, Room, Spec};

fn now() -> u64 {
    common::clock::wall_ms()
}

pub fn codecs() -> Vec<CodecPolicy> {
    let v = || vec!["nack".to_string(), "nack pli".to_string(), "ccm fir".to_string(), "transport-cc".to_string()];
    vec![
        CodecPolicy { kind: MediaKind::Audio, name: "opus".to_string(), rtcp_fb: vec!["transport-cc".to_string()] },
        CodecPolicy { kind: MediaKind::Video, name: "VP8".to_string(), rtcp_fb: v() },
        CodecPolicy { kind: MediaKind::Video, name: "H264".to_string(), rtcp_fb: v() },
    ]
}

fn server_ext(uri: &str) -> Option<u8> {
    SERVER_EXTMAP.iter().find(|(_, u)| *u == uri).map(|(i, _)| *i)
}

fn sub_ext(mode: PcMode, tags: Option<&[ExtMap]>) -> SubExt {
    let pick = |uri: &str| match (mode, tags) {
        (PcMode::One, Some(t)) => t.iter().find(|e| e.uri == uri).map(|e| e.id).or_else(|| server_ext(uri)),
        _ => server_ext(uri),
    };
    SubExt { mid: pick(EXT_MID), audio_level: pick(EXT_AUDIO_LEVEL), abs_send_time: pick(EXT_ABS_SEND_TIME), twcc: pick(EXT_TWCC) }
}

pub fn server_config(ctx: &Ctx, p: &Peer) -> ServerConfig {
    ServerConfig {
        sfu_id: ctx.boot.epoch.clone(),
        pc_mode: p.pc_mode.as_str().to_string(),
        ice: p.ice(ctx.links.ip, ctx.links.udp_port, ctx.links.tcp_port),
        dtls: ctx.links.dtls_info(),
        codecs: codecs(),
        codecs_sub: None,
        extmap: SERVER_EXTMAP.iter().map(|(id, uri)| ExtMap { id: *id, uri: uri.to_string() }).collect(),
        max_bitrate_bps: ctx.boot.policy.media.max_bitrate_bps as u64,
    }
}

fn me_of(room: &Room, user: &str) -> Option<MemberInfo> {
    room.member(user).map(|m| m.info())
}

pub async fn room_join(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: RoomJoinReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let room = ctx.rooms.get(&req.room_id).ok_or_else(|| fail(ErrCode::RoomNotFound))?;
    let mut existing = ctx.links.peer(&env.user_id);
    if let Some(p) = existing.clone()
        && (p.in_room(&room.id) || p.session() != env.session_id) {
            tracing::info!(kind = "ev", spec = "ROOM.S.join_again", room = %room.id, user = %env.user_id, session = %env.session_id, epoch = %ctx.boot.epoch, area = "ROOM", key = %room.id, cause = "ROOM_JOIN", "재입장 — 옛 Peer 축출");
            gc::drop_peer(ctx, &p, Path::Evict);
            existing = None;
        }
    {
        let g = lock(&room.inner);
        if !g.members.contains_key(&env.user_id) {
            crate::room::core::capacity(env.hidden, g.user_count(), room.capacity)?;
        }
    }
    if let Some(p) = &existing {
        crate::room::core::listen_limit(p.room_list().len())?;
    }
    let created = existing.is_none();
    let p = match existing {
        Some(p) => p,
        None => {
            let p = ctx.links.create(NewPeer {
                user_id: env.user_id.clone(),
                session_id: env.session_id.clone(),
                pc_mode: env.pc_mode,
                ident: Ident { participant_type: env.participant_type, hidden: env.hidden, metadata: env.metadata.clone(), permission: env.permission },
                home: (env.home_node.clone(), env.home_inst.clone()),
                ext: sub_ext(env.pc_mode, req.extmap.as_deref()),
            });
            if env.pc_mode == PcMode::One
                && let Some(c) = &req.codecs {
                    lock(&p.table).pts.seed_tags(c);
                }
            p
        }
    };
    if created && env.pc_mode == PcMode::Two && (req.extmap.is_some() || req.codecs.is_some()) {
        lock(&p.table).pts.ignored_tags += 1;
    }
    if !lock(&p.table).can_add_slots(1) {
        if created {
            ctx.links.reap(&p);
        }
        return Err(fail(ErrCode::MidLimit).details(json!({"limit": ctx.boot.policy.media.slot_mid_reserve})));
    }
    let perm = {
        let mut g = lock(&room.inner);
        let perm = crate::room::core::initial(env.permission, g.overrides.get(&env.user_id).copied());
        g.members.insert(
            env.user_id.clone(),
            Member {
                user_id: env.user_id.clone(),
                role: req.role.unwrap_or(oxsig::room::ROLE_DEFAULT),
                select: req.select.unwrap_or(false),
                participant_type: env.participant_type,
                metadata: env.metadata.clone(),
                permission: perm,
                hidden: env.hidden,
                session_id: env.session_id.clone(),
                joined_at: now(),
            },
        );
        g.ever_joined = true;
        g.empty_since = 0;
        perm
    };
    lock(&p.rooms).insert(room.id.clone());
    let half = lock(&p.pubs).iter().any(|s| s.half.load(Ordering::Acquire) && !s.gone.load(Ordering::Acquire));
    let u = p.user_id.clone();
    room.floor.apply("ROOM_JOIN", move |s| {
        s.ws_join(&u, perm.floor_request);
        s.ws_reg_half(&u, half);
    });
    crate::sub::gate_close(&p, &room.id, now());
    wire::room_rows(ctx, &p, &room);
    if req.select == Some(true) {
        crate::aff::join_select(ctx, &p, &room.id);
    }
    if !env.hidden {
        let info = room.member(&env.user_id).map(|m| m.info());
        room.change(|_, v| {
            let i = info?;
            let body = ParticipantEvent { kind: ParticipantEventType::Joined, room_id: room.id.clone(), user_id: i.user_id.clone(), role: Some(i.role), select: Some(i.select), participant_type: Some(i.participant_type), metadata: i.metadata.clone(), permission: i.permission, version: v };
            Some(Notice { op: oxsig::op::PARTICIPANT_EVENT, body: serde_json::to_value(body).ok()?, exclude: vec![i.user_id.clone()], room_id: Some(room.id.clone()), sfu_id: None, control: None })
        });
    }
    let label = if env.hidden { "ROOM.S.join_hidden" } else { "ROOM.S.join" };
    tracing::info!(kind = "ev", spec = %label, room = %room.id, user = %p.user_id, session = %p.session(), epoch = %ctx.boot.epoch, area = "ROOM", key = %room.id, cause = "ROOM_JOIN", in.hidden = env.hidden, "입장");
    let (participants, me) = {
        let g = lock(&room.inner);
        (g.participants(), g.members.get(&env.user_id).map(Member::info))
    };
    let (tracks, version) = ctx.tracks.snapshot(&room);
    let res = RoomJoinRes {
        room_id: room.id.clone(),
        participants,
        me: me.ok_or_else(|| fail(ErrCode::InternalError))?,
        affiliation: gc::affiliation(&p),
        server_config: server_config(ctx, &p),
        tracks,
        version,
        media_codecs: room.media.clone(),
        mline_table: p.table_wire(),
    };
    Ok(serde_json::to_value(res).unwrap_or_default())
}

pub fn room_leave(ctx: &Ctx, env: &Envelope, body: &Value) -> Result<Value, Failure> {
    let req: RoomLeaveReq = serde_json::from_value(body.clone()).map_err(|e| fail(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let room = ctx.rooms.get(&req.room_id).ok_or_else(|| fail(ErrCode::NotInRoom))?;
    let p = ctx.links.peer(&env.user_id).filter(|p| p.in_room(&room.id)).ok_or_else(|| fail(ErrCode::NotInRoom))?;
    if !gc::leave_seven(ctx, &room, &p, Path::Leave) {
        return Err(fail(ErrCode::NotInRoom));
    }
    let res = RoomLeaveRes { room_id: room.id.clone(), affiliation: gc::affiliation(&p) };
    Ok(serde_json::to_value(res).unwrap_or_default())
}

pub fn snapshot(ctx: &Ctx, room: &Room, user: &str) -> Option<RoomSnapshot> {
    let (participants, me) = {
        let g = lock(&room.inner);
        (g.participants(), g.members.get(user).map(Member::info))
    };
    let (tracks, version) = ctx.tracks.snapshot(room);
    Some(RoomSnapshot { participants, me: me?, tracks, version })
}

pub fn resume(ctx: &Ctx, env: &Envelope) -> Value {
    let mut out = ResumeRes::default();
    if let Some(p) = ctx.links.peer(&env.user_id).filter(|p| p.session() == env.session_id) {
        for r in p.room_list() {
            if let Some(room) = ctx.rooms.get(&r)
                && let Some(s) = snapshot(ctx, &room, &p.user_id) {
                    out.snapshot.insert(r, s);
                }
        }
        out.publications = lock(&p.pubs).iter().filter(|s| !s.gone.load(Ordering::Acquire)).map(|s| s.publication()).collect();
        out.mline_tables = vec![p.table_wire()];
    }
    serde_json::to_value(out).unwrap_or_default()
}

pub async fn create_room(ctx: &Ctx, req: CreateRoomReq, unused: Option<u32>, departure: Option<u32>, capacity: u32) -> Reply {
    let Some(id) = req.room_id.clone() else { return Reply::fail(fail(ErrCode::MissingField)) };
    let media = req.media_codecs.clone().unwrap_or_default().filled();
    if !crate::r#pub::core::SUPPORTED_VIDEO.iter().any(|s| s.eq_ignore_ascii_case(media.video())) {
        return Reply::fail(fail(ErrCode::CodecRequired).details(json!({"supported": crate::r#pub::core::SUPPORTED_VIDEO})));
    }
    if !media.audio().eq_ignore_ascii_case("opus") {
        return Reply::fail(fail(ErrCode::CodecRequired).details(json!({"supported": ["opus"]})));
    }
    let (room, fresh) = ctx.rooms.create(Spec { id: id.clone(), name: req.name.clone().unwrap_or_else(|| id.clone()), capacity, media, unused_ttl: unused, departure_ttl: departure });
    if fresh {
        ctx.tracks.create_slots(&room);
        room.floor.spawn();
        tracing::info!(kind = "ev", spec = "SV-STATE-ROOM-LIFE", room = %id, epoch = %ctx.boot.epoch, capacity, unused_ttl = ?unused, departure_ttl = ?departure, "방 생성");
    }
    if !ctx.bus.declare_room(&id).await {
        return Reply::fail(fail(ErrCode::InternalError).msg("room 토큰"));
    }
    let res = CreateRoomRes { room_id: room.id.clone(), name: room.name.clone(), capacity: room.capacity, created_at: room.created_at, media_codecs: room.media.clone() };
    Reply::ok(serde_json::to_value(res).unwrap_or_default())
}

pub fn get_room(ctx: &Ctx, room_id: &str, tracks: bool, user: Option<&str>) -> Reply {
    let Some(room) = ctx.rooms.get(room_id) else { return Reply::fail(fail(ErrCode::RoomNotFound)) };
    let (participants, user_count) = {
        let g = lock(&room.inner);
        (g.participants(), g.user_count())
    };
    let me = user.and_then(|u| me_of(&room, u));
    let (list, version) = ctx.tracks.snapshot(&room);
    let res = GetRoomRes {
        room_id: room.id.clone(),
        name: room.name.clone(),
        capacity: room.capacity,
        user_count,
        participants,
        me,
        created_at: room.created_at,
        media_codecs: room.media.clone(),
        version,
        tracks: tracks.then_some(list),
    };
    Reply::ok(serde_json::to_value(res).unwrap_or_default())
}

pub fn media_codecs_of(r: &Arc<Room>) -> MediaCodecs {
    r.media.clone()
}
