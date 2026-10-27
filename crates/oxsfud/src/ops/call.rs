// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-REQUEST · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-SERVER-CONFIG · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-EMIT · 정:IMPL-ROOM-RESYNC · 정:IMPL-ROOM-VERSION-ISSUE · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · 정:IMPL-PUB-ADD · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-TRACK-SET · 정:IMPL-PUB-RID-SPATIAL · 정:IMPL-TRK-FROM-PUB · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-READY-GATE · 정:IMPL-LINK-ICE-RESTART · 정:IMPL-FLR-FIRST-NOTICE · 정:IMPL-FLR-MEMBER-JOIN · 정:IMPL-AUX-MESSAGE · 정:IMPL-AUX-TASK · 정:IMPL-SES-RESUME · 정:IMPL-ERR-ISSUERS · 연:ROOM.S · 연:AFF.S · 연:PUB.S · 연:TRK.S · 연:SUB.S · 연:LINK.S · 연:RULE-ROOM-JOIN-PROC · 연:ROOM.op.ROOM_JOIN · 연:RULE-ROOM-CREATE-PROC · 연:AUX.op.MESSAGE · model: claude-opus-5-5
//! 봉투 하나 = 판정 하나. 방 · 소속 · 발행 · 트랙 · 구독 · 연결의 서버 걸음을 차례대로 · 판정은 상태 락 안에서 · 통지는 그 안에서 버스 한 줄에 · 송신 · 미디어는 락 밖(`floor::Exec`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::bus::env::{Call, Reply, SessionCtx};
use oxsig::code::{Code, Failure};
use oxsig::msg::*;
use oxsig::op::Op;
use oxsig::types::*;
use serde_json::{Value, json};

use super::floor::{self, Exec};
use super::{Peer, Sfu, State, now_ms};
use crate::fwd::hot::{Carry, RoomHot, Slot};
use crate::r#pub::{IngressExt, PublishState, Stream, Uplink};
use crate::trk::Room;
use crate::sub::table::{Change, Class, Tuple, Want};

pub type Res = Result<Value, Failure>;

fn fail(c: Code) -> Failure {
    Failure::of(c)
}

fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, Failure> {
    serde_json::from_value(v.clone()).map_err(|e| Failure::of(Code::INVALID_PAYLOAD).msg(e.to_string()))
}

fn to_val<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

pub async fn handle(sfu: &Arc<Sfu>, c: Call) -> Reply {
    let mut exec = Exec::default();
    let r = match c {
        Call::Op { ctx, op, body } => {
            let res = op_call(sfu, &ctx, op, &body, &mut exec);
            match res {
                Ok(v) => Reply::Ok { body: v },
                Err(f) => Reply::Fail { failure: f },
            }
        }
        Call::SessionGone { user_id, session, code } => {
            crate::gc::session_gone(sfu, &user_id, &session, code, &mut exec);
            Reply::ok(&json!({}))
        }
        Call::Resume { ctx } => match resume(sfu, &ctx) {
            Ok(v) => Reply::Ok { body: v },
            Err(f) => Reply::Fail { failure: f },
        },
        Call::RoomCreate { room_id, name, capacity, unused_ttl_secs, departure_ttl_secs, media_codecs } => match create(sfu, &room_id, &name, capacity, unused_ttl_secs, departure_ttl_secs, media_codecs).await {
            Ok(v) => Reply::Ok { body: v },
            Err(f) => Reply::Fail { failure: f },
        },
        Call::RoomGet { room_id, tracks, me } => match get(sfu, &room_id, tracks, me.as_deref()) {
            Ok(v) => Reply::Ok { body: v },
            Err(f) => Reply::Fail { failure: f },
        },
        Call::RoomList => list(sfu),
        Call::Admin { method, path, query, body } => super::admin::handle(sfu, &method, &path, &query, body.as_ref(), &mut exec),
        Call::LogLevel { level, ttl_secs } => match common::policy::LoggingLevel::parse(&level) {
            Some(l) => {
                common::log::level::set_override(l, ttl_secs, common::time::wall_ms());
                Reply::ok(&json!({"level": level, "ttl_secs": ttl_secs}))
            }
            None => Reply::fail(Code::INVALID_PAYLOAD),
        },
    };
    exec.run(sfu).await;
    r
}

fn op_call(sfu: &Arc<Sfu>, ctx: &SessionCtx, op: u16, body: &Value, exec: &mut Exec) -> Res {
    let Some(o) = Op::from_u16(op) else { return Err(fail(Code::UNKNOWN_OP)) };
    let r = match o {
        Op::RoomJoin => join(sfu, ctx, body, exec),
        Op::RoomLeave => leave(sfu, ctx, body, exec),
        Op::PublishTracks => publish(sfu, ctx, body, exec),
        Op::TrackSet => track_set(sfu, ctx, body, exec),
        Op::Ready => ready(sfu, ctx, body, exec),
        Op::SubscribeLayer => subscribe_layer(sfu, ctx, body, exec),
        Op::IceRestart => ice_restart(sfu, ctx, body),
        Op::Affiliation => affiliation(sfu, ctx, body, exec),
        Op::Message => message(sfu, ctx, body),
        Op::Task => Ok(json!({})),
        _ => Err(fail(Code::UNKNOWN_OP)),
    };
    if let Err(f) = &r {
        common::ev!(spec = "CP-CODE", area = "OPS", user = %ctx.user_id, op = o.name(), code = f.code, "실패 응답");
    }
    r
}

pub fn server_config(sfu: &Sfu, p: &Peer) -> ServerConfig {
    let pc = p.publish.creds().unwrap_or_else(crate::link::Creds::generate);
    let sc = p.sub_transport().creds().unwrap_or_else(|| pc.clone());
    let video_fb = vec!["nack".to_string(), "nack pli".to_string(), "ccm fir".to_string(), "transport-cc".to_string()];
    ServerConfig {
        sfu_id: sfu.epoch.clone(),
        pc_mode: p.pc,
        ice: ServerConfigIce { ip: sfu.cfg.public_ip.to_string(), port: sfu.cfg.udp_port, publish_ufrag: pc.ufrag, publish_pwd: pc.pwd, subscribe_ufrag: sc.ufrag, subscribe_pwd: sc.pwd, tcp_port: sfu.cfg.tcp_port },
        dtls: ServerConfigDtls { fingerprint: sfu.io.fingerprint.clone(), setup: ServerConfigDtlsSetup::Passive },
        codecs: vec![
            CodecPolicy { kind: Kind::Audio, name: "opus".into(), rtcp_fb: vec!["transport-cc".into()] },
            CodecPolicy { kind: Kind::Video, name: "VP8".into(), rtcp_fb: video_fb.clone() },
            CodecPolicy { kind: Kind::Video, name: "H264".into(), rtcp_fb: video_fb },
        ],
        codecs_sub: None,
        extmap: super::peer::server_extmap(),
        max_bitrate_bps: u64::from(sfu.cfg.policy.media.max_bitrate_bps),
    }
}

pub fn stream_of(st: &State, track_id: &str) -> Option<Arc<Stream>> {
    st.peers.values().find_map(|p| p.streams.get(track_id).cloned())
}

fn half_kinds(p: &Peer) -> Vec<Kind> {
    p.streams.values().filter(|s| s.duplex() == Duplex::Half).map(|s| s.kind).collect()
}

pub fn floor_perm(perm: &Permission, half: &[Kind]) -> bool {
    perm.floor_request && half.iter().all(|k| if *k == Kind::Audio { perm.publish_audio } else { perm.publish_video })
}

fn wants_for(st: &State, user: &str, room: &str) -> Vec<Want> {
    let Some(r) = st.rooms.get(room) else { return Vec::new() };
    let mut v = Vec::new();
    for kind in [Kind::Audio, Kind::Video] {
        let codec = if kind == Kind::Audio { r.audio_codec.clone() } else { r.video_codec.clone() };
        v.push(Want { track_id: crate::trk::slot_track_id(room, kind), room: room.to_string(), kind, class: Class::Slot, tuple: Tuple { kind, codec }, rank: u64::MAX, waiting_since: 0 });
    }
    let prefs = st.peers.get(user).map(|p| &p.prefs);
    for t in r.tracks.values() {
        if t.user_id == user || !t.active {
            continue;
        }
        if prefs.and_then(|m| m.get(&t.track_id)).is_some_and(|x| x.paused) {
            continue;
        }
        v.push(want_of(st, user, room, t));
    }
    v
}

fn want_of(st: &State, user: &str, room: &str, t: &crate::trk::Item) -> Want {
    let codec = if t.kind == Kind::Audio { crate::trk::AUDIO_CODEC.to_string() } else { t.codec.clone() };
    Want { track_id: t.track_id.clone(), room: room.to_string(), kind: t.kind, class: Class::Full, tuple: Tuple { kind: t.kind, codec }, rank: rank_of(st, user, &t.track_id, t.kind), waiting_since: 0 }
}

fn rank_of(st: &State, user: &str, track_id: &str, kind: Kind) -> u64 {
    match kind {
        Kind::Audio => stream_of(st, track_id).map(|s| s.last_voice_ms.load(Ordering::Relaxed)).unwrap_or(0),
        Kind::Video => {
            let pr = st.peers.get(user).and_then(|p| p.prefs.get(track_id)).map(|x| u64::from(x.priority)).unwrap_or(0);
            pr << 40
        }
    }
}

pub fn apply_changes(st: &mut State, user: &str, changes: &[Change]) -> bool {
    let mut changed = false;
    for ch in changes {
        match ch {
            Change::On { mid, track_id, room, label } => {
                changed = true;
                let stream = stream_of(st, track_id);
                let hot = st.rooms.get(room).map(|r| r.hot.clone());
                let Some(p) = st.peers.get_mut(user) else { continue };
                let gate = p.gate(room);
                let Some(dl) = p.table.downlink_of(*mid) else { continue };
                dl.carry.store(Some(Arc::new(Carry { track_id: Arc::from(track_id.as_str()), room: Arc::from(room.as_str()), gate })));
                dl.carry_at_ms.store(now_ms(), Ordering::Release);
                if dl.slot {
                    if let Some(h) = &hot {
                        h.slot(dl.kind).attach(&dl);
                    }
                } else if let Some(s) = &stream {
                    s.attach(&dl);
                    if dl.kind == Kind::Video {
                        dl.awaiting_key.store(true, Ordering::Release);
                    }
                }
                common::ev!(spec = *label, area = "LINK", key = %user, from = "-", to = %mid, cause = "row", user = %user, room = %room, track = %track_id, arev = p.table.arev);
            }
            Change::Off { mid, track_id, room, label } => {
                changed = true;
                let stream = stream_of(st, track_id);
                let hot = st.rooms.get(room).map(|r| r.hot.clone());
                let Some(p) = st.peers.get_mut(user) else { continue };
                let Some(dl) = p.table.downlink_of(*mid) else { continue };
                if dl.carries(track_id) {
                    dl.carry.store(None);
                    dl.carry_at_ms.store(now_ms(), Ordering::Release);
                }
                if dl.slot {
                    if let Some(h) = &hot {
                        h.slot(dl.kind).detach(&dl);
                    }
                } else if let Some(s) = &stream {
                    s.detach(&dl);
                }
                common::ev!(spec = *label, area = "LINK", key = %user, from = %mid, to = "-", cause = "row", user = %user, room = %room, track = %track_id, arev = p.table.arev);
            }
            Change::Wait { track_id, room } => {
                common::ev!(spec = "LINK.S.track_on_full", area = "LINK", key = %user, from = "-", to = "wait", cause = "row", user = %user, room = %room, track = %track_id);
            }
        }
    }
    if let Some(p) = st.peers.get(user) {
        p.refresh_downlinks();
    }
    changed
}

pub fn rows_on(st: &mut State, user: &str, wants: Vec<Want>) -> Result<bool, Code> {
    let mut all = Vec::new();
    for w in wants {
        let Some(p) = st.peers.get_mut(user) else { return Ok(false) };
        let ch = p.table.on(w, &make_dl(p))?;
        all.extend(ch);
    }
    Ok(apply_changes(st, user, &all))
}

pub fn rows_off(st: &mut State, user: &str, track_id: &str, room: &str) -> bool {
    let Some(p) = st.peers.get_mut(user) else { return false };
    let ch = p.table.off(track_id, room);
    apply_changes(st, user, &ch)
}

pub fn make_dl(p: &Peer) -> impl Fn(u8, Kind, bool, u32, u32, u8, u8) -> Arc<crate::fwd::hot::Downlink> + use<> {
    let user: Arc<str> = Arc::from(p.user.as_str());
    let t = Arc::downgrade(p.sub_transport());
    let ext = p.sub_ext;
    move |mid, kind, slot, ssrc, rtx, pt, rpt| Arc::new(crate::fwd::hot::Downlink::new(mid, kind, slot, ssrc, rtx, pt, rpt, user.clone(), t.clone(), ext))
}

fn track_on_room(sfu: &Sfu, st: &mut State, room: &str, track_id: &str) {
    let Some(r) = st.rooms.get(room) else { return };
    let Some(item) = r.tracks.get(track_id).cloned() else { return };
    let members: Vec<String> = r.members.keys().filter(|u| **u != item.user_id).cloned().collect();
    for u in members {
        if st.peers.get(&u).and_then(|p| p.prefs.get(track_id)).is_some_and(|x| x.paused) {
            continue;
        }
        let w = want_of(st, &u, room, &item);
        if rows_on(st, &u, vec![w]).unwrap_or(false)
            && let Some(p) = st.peers.get(&u)
        {
            sfu.mline_event(p);
        }
    }
}

fn track_off_room(sfu: &Sfu, st: &mut State, room: &str, track_id: &str) {
    let members: Vec<String> = st.rooms.get(room).map(|r| r.members.keys().cloned().collect()).unwrap_or_default();
    for u in members {
        if rows_off(st, &u, track_id, room)
            && let Some(p) = st.peers.get(&u)
        {
            sfu.mline_event(p);
        }
    }
}

fn track_event(sfu: &Sfu, room: &Room, action: PublishTracksReqAddAction, tracks: Vec<TrackEntry>, label: &str, user: &str) {
    if tracks.is_empty() {
        return;
    }
    let v = room.version();
    common::ev!(spec = label, area = "TRK", key = %room.id, from = "-", to = ?action, cause = "pub", room = %room.id, user = %user, seq = v.seq);
    let body = to_val(&TrackEventBody { action, room_id: room.id.clone(), tracks, version: v });
    sfu.notice_room(&room.id, Op::TrackEvent, body, Vec::new());
}

fn member_info(ctx: &SessionCtx, req_role: Option<u8>, select: bool, perm: Permission) -> MemberInfo {
    MemberInfo {
        user_id: ctx.user_id.clone(),
        role: Some(req_role.unwrap_or(255)),
        select,
        participant_type: ctx.participant_type,
        metadata: ctx.metadata.clone(),
        permission: if perm == Permission::default() { None } else { Some(perm) },
    }
}

fn join(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: RoomJoinReq = parse(body)?;
    let room_id = req.room_id.clone();
    let user = ctx.user_id.clone();
    let now = now_ms();
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    if !st.rooms.contains_key(&room_id) {
        common::ev!(spec = "ROOM.S.join_3001", area = "ROOM", key = %room_id, cause = "ROOM_JOIN", impossible = 1, user = %user);
        return Err(fail(Code::ROOM_NOT_FOUND));
    }
    let mut again = false;
    if let Some(p) = st.peers.get(&user) {
        let in_room = p.rooms.contains(&room_id);
        if in_room || p.fp != ctx.session {
            again = in_room;
            crate::gc::evict(sfu, st, &user, if in_room { Some(room_id.as_str()) } else { None }, exec);
        }
    }
    let hidden = ctx.hidden;
    let full = st.rooms.get(&room_id).is_some_and(|r| r.full_for(hidden));
    if !again && full {
        common::ev!(spec = "ROOM.S.join_4001", area = "ROOM", key = %room_id, cause = "ROOM_JOIN", impossible = 1, user = %user);
        return Err(fail(Code::ROOM_FULL));
    }
    let created = !st.peers.contains_key(&user);
    if created {
        let arev = st.arev.get(&user).copied().unwrap_or(0);
        let p = Peer::new(ctx, &sfu.io, &sfu.epoch, usize::from(sfu.cfg.policy.media.slot_mid_reserve), req.codecs.as_deref(), req.extmap.as_deref(), arev, now);
        common::ev!(spec = "LINK.S.new_conn", area = "LINK", key = %user, from = "-", to = "Alive", cause = "ROOM_JOIN", user = %user);
        st.peers.insert(user.clone(), p);
    }
    let undo = |st: &mut State, sfu: &Sfu| {
        if created && let Some(p) = st.peers.remove(&user) {
            for t in p.transports() {
                sfu.io.unregister(&t);
                t.close();
            }
        }
    };
    let (rooms_n, has) = st.peers.get(&user).map(|p| (p.rooms.len(), p.rooms.contains(&room_id))).unwrap_or((0, false));
    if !has && rooms_n >= crate::room::MAX_ROOMS {
        undo(st, sfu);
        return Err(fail(Code::LISTEN_LIMIT));
    }
    if !st.peers.get(&user).is_some_and(|p| p.table.slot_room_fits(2)) {
        undo(st, sfu);
        return Err(fail(Code::MID_LIMIT));
    }
    if let Some(p) = st.peers.get_mut(&user) {
        p.ctx = ctx.clone();
        p.fp = ctx.session.clone();
        p.home = (ctx.home_node.clone(), ctx.home_inst.clone());
        p.rooms.insert(room_id.clone());
    }
    let select = req.select.unwrap_or(true);
    let token_perm = ctx.permission;
    let half = st.peers.get(&user).map(half_kinds).unwrap_or_default();
    {
        let Some(r) = st.rooms.get_mut(&room_id) else { return Err(fail(Code::ROOM_NOT_FOUND)) };
        let perm = r.permission_of(&user, token_perm);
        let info = member_info(ctx, req.role, select, perm);
        r.members.insert(user.clone(), crate::room::Member { info: info.clone(), hidden, session: ctx.session.clone() });
        r.ever_joined = true;
        r.empty_since = 0;
        let label = if again {
            "ROOM.S.join_again"
        } else if hidden {
            "ROOM.S.join_hidden"
        } else {
            "ROOM.S.join"
        };
        if !again && !hidden {
            let v = r.bump();
            let ev = ParticipantEventBody { type_: ParticipantEventBodyType::Joined, room_id: room_id.clone(), user_id: user.clone(), role: info.role, select: Some(select), participant_type: Some(info.participant_type), metadata: info.metadata.clone(), permission: info.permission, version: v };
            sfu.notice_room(&room_id, Op::ParticipantEvent, to_val(&ev), vec![user.clone()]);
        }
        common::ev!(spec = label, area = "ROOM", key = %room_id, from = "-", to = "member", cause = "ROOM_JOIN", room = %room_id, user = %user, seq = r.seq, in.hidden = hidden, in.pcm = ctx.pc_mode.as_str());
        let fp = floor_perm(&perm, &half);
        let s = r.floor.step(now, |s, e| crate::flr::core::ws_join(s, e, &user, !half.is_empty(), fp));
        exec.floor(&room_id, r.hot.clone(), "ws_join", Some(&user), s);
    }
    let pub_room = st.peers.get(&user).and_then(|p| p.pub_room.clone());
    let step = crate::aff::decide_join(pub_room.as_deref(), &room_id, select);
    common::ev!(spec = step.label(), area = "AFF", key = %user, from = ?pub_room, to = %room_id, cause = "ROOM_JOIN", user = %user, room = %room_id, in.sel = select);
    if matches!(step, crate::aff::JoinStep::Select | crate::aff::JoinStep::SelectMoved) && move_ok(st, &user, &room_id) {
        set_pub_room(sfu, st, &user, Some(&room_id), exec);
    }
    let wants = wants_for(st, &user, &room_id);
    if let Err(c) = rows_on(st, &user, wants) {
        return Err(fail(c));
    }
    let r = st.rooms.get(&room_id).ok_or_else(|| fail(Code::ROOM_NOT_FOUND))?;
    let p = st.peers.get(&user).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
    let me = r.members.get(&user).map(|m| m.info.clone()).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
    let res = RoomJoinRes {
        room_id: room_id.clone(),
        participants: r.participants(),
        me,
        affiliation: crate::aff::affiliation(&p.rooms, p.pub_room.as_deref()),
        server_config: server_config(sfu, p),
        tracks: r.track_entries(),
        version: r.version(),
        media_codecs: r.media_codecs(),
        mline_table: p.table.wire(),
    };
    Ok(to_val(&res))
}

fn move_ok(st: &State, user: &str, room: &str) -> bool {
    let (Some(p), Some(r)) = (st.peers.get(user), st.rooms.get(room)) else { return false };
    let perm = r.permission_of(user, p.ctx.permission);
    let (bad, noperm) = crate::r#pub::move_check(&p.streams, &perm, &r.audio_codec, &r.video_codec);
    !bad && !noperm
}

fn full_items_of(st: &State, user: &str, room: &str) -> Vec<crate::trk::Item> {
    st.rooms.get(room).map(|r| r.tracks.values().filter(|t| t.user_id == user).cloned().collect()).unwrap_or_default()
}

pub fn set_pub_room(sfu: &Sfu, st: &mut State, user: &str, to: Option<&str>, exec: &mut Exec) {
    let now = now_ms();
    let from = st.peers.get(user).and_then(|p| p.pub_room.clone());
    if from.as_deref() == to {
        return;
    }
    if let Some(f) = &from
        && let Some(r) = st.rooms.get_mut(f)
    {
        let s = r.floor.step(now, |s, e| crate::flr::core::ws_pub_away(s, e, user));
        exec.floor(f, r.hot.clone(), "pub_away", Some(user), s);
    }
    let hot = to.and_then(|t| st.rooms.get(t).map(|r| r.hot.clone()));
    if let Some(p) = st.peers.get_mut(user) {
        p.pub_room = to.map(str::to_string);
        p.aff.pub_room.store(hot);
    }
    if let Some(t) = to
        && let Some(r) = st.rooms.get_mut(t)
    {
        let s = r.floor.step(now, |s, e| crate::flr::core::ws_pub_select(s, e, user));
        exec.floor(t, r.hot.clone(), "pub_select", Some(user), s);
        common::ev!(spec = "PUB.S.at_pub", area = "PUB", key = %user, from = "-", to = %t, cause = "pub_select", user = %user);
    } else {
        common::ev!(spec = "PUB.S.off_pub", area = "PUB", key = %user, from = ?from, to = "-", cause = "pub_deselect", user = %user);
    }
    pub_move(sfu, st, user, from.as_deref(), to);
}

fn pub_move(sfu: &Sfu, st: &mut State, user: &str, from: Option<&str>, to: Option<&str>) {
    let items: Vec<crate::trk::Item> = match from {
        Some(f) => full_items_of(st, user, f),
        None => st.peers.get(user).map(|p| p.streams.values().filter(|s| s.duplex() == Duplex::Full && !s.simulcast || s.listed.load(Ordering::Acquire)).map(item_of).collect()).unwrap_or_default(),
    };
    if items.is_empty() {
        return;
    }
    if let Some(f) = from {
        if let Some(r) = st.rooms.get_mut(f) {
            for i in &items {
                r.tracks.remove(&i.track_id);
            }
            r.bump();
        }
        if let Some(r) = st.rooms.get(f) {
            track_event(sfu, r, PublishTracksReqAddAction::Remove, items.iter().map(|i| i.removed(f)).collect(), "TRK.S.unpub_full", user);
        }
    }
    if let Some(t) = to {
        if let Some(r) = st.rooms.get_mut(t) {
            for i in &items {
                r.tracks.insert(i.track_id.clone(), i.clone());
            }
            r.bump();
        }
        if let Some(r) = st.rooms.get(t) {
            track_event(sfu, r, PublishTracksReqAddAction::Add, items.iter().map(|i| i.entry(t)).collect(), "TRK.S.pub_full", user);
        }
    }
    let members_from: Vec<String> = from.and_then(|f| st.rooms.get(f).map(|r| r.members.keys().cloned().collect())).unwrap_or_default();
    let members_to: Vec<String> = to.and_then(|t| st.rooms.get(t).map(|r| r.members.keys().cloned().collect())).unwrap_or_default();
    for i in &items {
        for u in members_from.iter().chain(members_to.iter()) {
            if *u == user {
                continue;
            }
            let in_from = members_from.contains(u);
            let in_to = members_to.contains(u);
            let mut changed = false;
            if in_from && in_to {
                if let (Some(f), Some(t)) = (from, to)
                    && let Some(p) = st.peers.get_mut(u)
                    && p.table.rehome(&i.track_id, f, t)
                {
                    if let Some(row) = p.table.find(&i.track_id, t) {
                        let gate = p.gates.get(t).cloned().unwrap_or_default();
                        row.dl.carry.store(Some(Arc::new(Carry { track_id: Arc::from(i.track_id.as_str()), room: Arc::from(t), gate })));
                    }
                    changed = true;
                }
            } else if in_from {
                if let Some(f) = from {
                    changed = rows_off(st, u, &i.track_id, f);
                }
            } else if in_to
                && i.active
                && let Some(t) = to
            {
                let w = want_of(st, u, t, i);
                changed = rows_on(st, u, vec![w]).unwrap_or(false);
            }
            if changed && let Some(p) = st.peers.get(u) {
                sfu.mline_event(p);
            }
        }
    }
}

fn item_of(s: &Arc<Stream>) -> crate::trk::Item {
    crate::trk::Item {
        track_id: s.track_id.to_string(),
        user_id: s.user.to_string(),
        kind: s.kind,
        codec: s.codec.to_string(),
        fmtp: s.fmtp.clone(),
        source: s.source,
        active: s.duplex() == Duplex::Full,
        muted: s.muted.load(Ordering::Acquire),
        simulcast: s.simulcast,
    }
}

fn leave(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: RoomLeaveReq = parse(body)?;
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let member = st.peers.get(&ctx.user_id).is_some_and(|p| p.rooms.contains(&req.room_id));
    if !member {
        common::ev!(spec = "ROOM.S.leave_3002", area = "ROOM", key = %req.room_id, cause = "ROOM_LEAVE", impossible = 1, user = %ctx.user_id);
        return Err(fail(Code::NOT_IN_ROOM));
    }
    let label = if ctx.hidden { "ROOM.S.leave_hidden" } else { "ROOM.S.leave" };
    crate::gc::leave_seven(sfu, st, &req.room_id, &ctx.user_id, crate::gc::Path::Leave, label, exec);
    let aff = st.peers.get(&ctx.user_id).map(|p| crate::aff::affiliation(&p.rooms, p.pub_room.as_deref())).unwrap_or_default();
    Ok(to_val(&RoomLeaveRes { room_id: req.room_id, affiliation: aff }))
}

fn publish(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: PublishTracksReq = parse(body)?;
    let user = ctx.user_id.clone();
    let now = now_ms();
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let room_id = req.room_id().to_string();
    if !st.peers.get(&user).is_some_and(|p| p.rooms.contains(&room_id)) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    match req {
        PublishTracksReq::Remove { track_ids, .. } => {
            let own = st.peers.get(&user).is_some_and(|p| track_ids.iter().all(|t| p.streams.contains_key(t)));
            if !own || track_ids.is_empty() {
                common::ev!(spec = "PUB.S.remove_3005", area = "PUB", key = %user, cause = "PUBLISH_TRACKS", impossible = 1, user = %user);
                return Err(fail(Code::TRACK_NOT_FOUND));
            }
            for t in &track_ids {
                remove_stream(sfu, st, &user, t, exec);
            }
            common::ev!(spec = "PUB.S.remove", area = "PUB", key = %user, from = "registered", to = "-", cause = "PUBLISH_TRACKS", user = %user, room = %room_id);
            Ok(to_val(&PublishTracksRes::Remove {}))
        }
        PublishTracksReq::Add { tracks, twcc_extmap_id, rid_extmap_id, repair_rid_extmap_id, mid_extmap_id, audio_level_extmap_id, .. } => {
            let (perm, pub_room, active, downs, one, ra, rv) = {
                let p = st.peers.get(&user).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
                let r = st.rooms.get(&room_id).ok_or_else(|| fail(Code::ROOM_NOT_FOUND))?;
                (r.permission_of(&user, p.ctx.permission), p.pub_room.clone(), p.active_tracks(), p.table.downlink_ssrcs(), p.pc == PcMode::V1pc, r.audio_codec.clone(), r.video_codec.clone())
            };
            if tracks.iter().any(|t| t.simulcast.unwrap_or(false)) && (rid_extmap_id.is_none() || mid_extmap_id.is_none()) {
                return Err(fail(Code::MISSING_FIELD));
            }
            let actx = crate::r#pub::AddCtx { room: &room_id, pub_room: pub_room.as_deref(), perm, active, downlink_ssrcs: &downs, one_pc: one, room_audio: &ra, room_video: &rv };
            let codecs = match crate::r#pub::check_add(&actx, &tracks) {
                Ok(c) => c,
                Err(f) => {
                    let label = match f.code {
                        2006 => "PUB.S.add_2006",
                        3011 => "PUB.S.add_3011",
                        4002 => "PUB.S.add_4002",
                        _ => "PUB.S.add",
                    };
                    common::ev!(spec = label, area = "PUB", key = %user, cause = "PUBLISH_TRACKS", impossible = 1, user = %user, code = f.code);
                    return Err(f);
                }
            };
            let mut out = Vec::new();
            let mut new_full = Vec::new();
            let mut new_half = false;
            {
                let p = st.peers.get_mut(&user).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
                let t = p.publish.clone();
                let mut ext = **t.ext.load();
                let merge = |a: Option<u8>, b: Option<u8>| b.or(a);
                ext = IngressExt { mid: merge(ext.mid, mid_extmap_id), rid: merge(ext.rid, rid_extmap_id), repaired_rid: merge(ext.repaired_rid, repair_rid_extmap_id), audio_level: merge(ext.audio_level, audio_level_extmap_id), twcc: merge(ext.twcc, twcc_extmap_id) };
                t.ext.store(Arc::new(ext));
                for (tr, codec) in tracks.iter().zip(codecs.iter()) {
                    let track_id = format!("{}-{}-{}", user, tr.kind.as_str(), &uuid::Uuid::new_v4().simple().to_string()[..8]);
                    let s = Arc::new(Stream::new(&track_id, &user, tr, codec, p.aff.clone(), Arc::downgrade(&t)));
                    s.swap_state(PublishState::Intended);
                    common::ev!(spec = common::log::SPEC_PUBLISH_STATE, area = "PUB", key = %track_id, from = "Created", to = "Intended", cause = "PUBLISH_TRACKS", user = %user, track = %track_id);
                    if s.simulcast {
                        let mut v: Vec<Arc<Stream>> = t.pending_sim.load().iter().cloned().collect();
                        v.push(s.clone());
                        t.pending_sim.store(Arc::new(v));
                    } else {
                        let up = Arc::new(Uplink::new(tr.ssrc, false, None, tr.kind, &s));
                        s.add_uplink(up.clone());
                        let rtx = tr.rtx_ssrc.map(|r| Arc::new(Uplink::new(r, true, None, tr.kind, &s)));
                        if let Some(r) = &rtx {
                            s.add_uplink(r.clone());
                        }
                        t.uplinks.rcu(|m| {
                            let mut n = (**m).clone();
                            n.insert(up.ssrc, up.clone());
                            if let Some(r) = &rtx {
                                n.insert(r.ssrc, r.clone());
                            }
                            n
                        });
                    }
                    if s.duplex() == Duplex::Half {
                        new_half = true;
                    } else if !s.simulcast {
                        new_full.push(s.clone());
                    }
                    p.streams.insert(track_id.clone(), s);
                    out.push(PublishTracksResAddTracks { mid: tr.mid.clone(), track_id });
                }
            }
            if new_half {
                let rooms: Vec<String> = st.peers.get(&user).map(|p| p.rooms.iter().cloned().collect()).unwrap_or_default();
                for rid in rooms {
                    if let Some(r) = st.rooms.get_mut(&rid) {
                        let s = r.floor.step(now, |s, e| crate::flr::core::ws_reg_half(s, e, &user));
                        exec.floor(&rid, r.hot.clone(), "reg_half", Some(&user), s);
                    }
                }
                common::ev!(spec = "TRK.S.pub_half", area = "TRK", key = %user, from = "NONE", to = "HALF", cause = "PUBLISH_TRACKS", user = %user);
            }
            if !new_full.is_empty() {
                add_full(sfu, st, &user, &room_id, &new_full);
            }
            common::ev!(spec = "PUB.S.add", area = "PUB", key = %user, from = "-", to = "registered", cause = "PUBLISH_TRACKS", user = %user, room = %room_id);
            Ok(to_val(&PublishTracksRes::Add { tracks: out }))
        }
    }
}

fn add_full(sfu: &Sfu, st: &mut State, user: &str, room_id: &str, streams: &[Arc<Stream>]) {
    let items: Vec<crate::trk::Item> = streams.iter().map(item_of).collect();
    if let Some(r) = st.rooms.get_mut(room_id) {
        for i in &items {
            r.tracks.insert(i.track_id.clone(), i.clone());
        }
        r.bump();
    }
    if let Some(r) = st.rooms.get(room_id) {
        track_event(sfu, r, PublishTracksReqAddAction::Add, items.iter().map(|i| i.entry(room_id)).collect(), "TRK.S.pub_full", user);
    }
    for i in &items {
        track_on_room(sfu, st, room_id, &i.track_id);
    }
}

pub fn on_first_rtp(sfu: &Arc<Sfu>, stream: &Arc<Stream>, up: &Arc<Uplink>) {
    if stream.swap_state(PublishState::Active) != PublishState::Active {
        common::ev!(spec = common::log::SPEC_PUBLISH_STATE, area = "PUB", key = &*stream.track_id, from = "Intended", to = "Active", cause = "first_rtp", user = &*stream.user, track = &*stream.track_id, ssrc = up.ssrc);
    }
    if stream.simulcast && !stream.listed.swap(true, Ordering::AcqRel) {
        let Ok(mut guard) = sfu.state.lock() else { return };
        let st = &mut *guard;
        let room = st.peers.get(&*stream.user).and_then(|p| p.pub_room.clone());
        if let Some(room) = room {
            add_full(sfu, st, &stream.user, &room, std::slice::from_ref(stream));
        }
    }
}

pub fn remove_stream(sfu: &Sfu, st: &mut State, user: &str, track_id: &str, exec: &mut Exec) {
    let now = now_ms();
    let Some(s) = st.peers.get_mut(user).and_then(|p| p.streams.remove(track_id)) else { return };
    if s.duplex() == Duplex::Half
        && let Some(pr) = st.peers.get(user).and_then(|p| p.pub_room.clone())
        && let Some(r) = st.rooms.get_mut(&pr)
        && r.floor.srv.holder().map(String::as_str) == Some(user)
    {
        let st2 = r.floor.step(now, |s, e| crate::flr::core::ws_pub_away(s, e, user));
        exec.floor(&pr, r.hot.clone(), "pub_remove", Some(user), st2);
        let s3 = r.floor.step(now, |s, e| crate::flr::core::ws_pub_select(s, e, user));
        exec.floor(&pr, r.hot.clone(), "pub_select", Some(user), s3);
    }
    if let Some(p) = st.peers.get(user) {
        let t = &p.publish;
        let ssrcs: Vec<u32> = s.uplinks.load().iter().map(|u| u.ssrc).collect();
        t.uplinks.rcu(|m| {
            let mut n = (**m).clone();
            for x in &ssrcs {
                n.remove(x);
            }
            n
        });
        let ps: Vec<Arc<Stream>> = t.pending_sim.load().iter().filter(|x| *x.track_id != *track_id).cloned().collect();
        t.pending_sim.store(Arc::new(ps));
    }
    let rooms: Vec<String> = st.rooms.iter().filter(|(_, r)| r.tracks.contains_key(track_id)).map(|(k, _)| k.clone()).collect();
    for rid in rooms {
        let item = st.rooms.get_mut(&rid).and_then(|r| r.tracks.remove(track_id));
        if let Some(r) = st.rooms.get_mut(&rid) {
            r.bump();
        }
        if let (Some(r), Some(i)) = (st.rooms.get(&rid), item) {
            let label = if i.active { "TRK.S.unpub_full" } else { "TRK.S.unpub_resting" };
            track_event(sfu, r, PublishTracksReqAddAction::Remove, vec![i.removed(&rid)], label, user);
        }
        track_off_room(sfu, st, &rid, track_id);
    }
}

fn track_set(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: TrackSetReq = parse(body)?;
    let user = ctx.user_id.clone();
    let now = now_ms();
    if req.muted.is_some() == req.duplex.is_some() {
        return Err(fail(Code::FIELD_CONFLICT));
    }
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let p = st.peers.get(&user).ok_or_else(|| fail(Code::NOT_IN_ROOM))?;
    if !p.rooms.contains(&req.room_id) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    let stream = match (&req.track_id, req.ssrc) {
        (Some(t), _) => p.streams.get(t).cloned(),
        (None, Some(ssrc)) => p.streams.values().find(|s| s.ssrc == ssrc).cloned(),
        _ => None,
    };
    let Some(s) = stream else {
        common::ev!(spec = "PUB.S.set_3005", area = "PUB", key = %user, cause = "TRACK_SET", impossible = 1, user = %user);
        return Err(fail(Code::TRACK_NOT_FOUND));
    };
    let ssrc = s.ssrc;
    if let Some(m) = req.muted {
        if s.duplex() == Duplex::Half {
            common::ev!(spec = "PUB.S.mute_3006", area = "PUB", key = %user, cause = "TRACK_SET", impossible = 1, user = %user);
            return Err(fail(Code::TRACK_OP_UNSUPPORTED));
        }
        if s.muted.load(Ordering::Acquire) == m {
            common::ev!(spec = "PUB.S.mute_noop", area = "PUB", key = %user, from = m, to = m, cause = "TRACK_SET", user = %user);
            return Ok(to_val(&TrackSetRes { ssrc, muted: None, duplex: None, noop: Some(true) }));
        }
        s.muted.store(m, Ordering::Release);
        common::ev!(spec = "PUB.S.mute", area = "PUB", key = %user, from = !m, to = m, cause = "TRACK_SET", user = %user, track = &*s.track_id);
        let rooms: Vec<String> = st.rooms.iter().filter(|(_, r)| r.tracks.contains_key(&*s.track_id)).map(|(k, _)| k.clone()).collect();
        for rid in rooms {
            if let Some(r) = st.rooms.get_mut(&rid) {
                if let Some(i) = r.tracks.get_mut(&*s.track_id) {
                    i.muted = m;
                }
                let v = r.bump();
                common::ev!(spec = "TRK.S.mute", area = "TRK", key = &*s.track_id, from = !m, to = m, cause = "TRACK_SET", room = %rid, seq = v.seq);
                let body = TrackStateBody { type_: TrackStateBodyType::Muted, user_id: user.clone(), track_id: s.track_id.to_string(), kind: s.kind, room_id: rid.clone(), version: v, muted: m };
                sfu.notice_room(&rid, Op::TrackState, to_val(&body), Vec::new());
            }
        }
        return Ok(to_val(&TrackSetRes { ssrc, muted: Some(m), duplex: None, noop: None }));
    }
    let Some(d) = req.duplex else { return Err(fail(Code::FIELD_CONFLICT)) };
    if s.simulcast {
        return Err(fail(Code::TRACK_OP_UNSUPPORTED));
    }
    let pub_room = p.pub_room.clone();
    if d == Duplex::Full && pub_room.as_deref() != Some(req.room_id.as_str()) {
        common::ev!(spec = "PUB.S.set_3011", area = "PUB", key = %user, cause = "TRACK_SET", impossible = 1, user = %user);
        return Err(fail(Code::NOT_PUB_ROOM));
    }
    if s.duplex() == d {
        common::ev!(spec = "PUB.S.set_noop", area = "PUB", key = %user, from = d.as_str(), to = d.as_str(), cause = "TRACK_SET", user = %user);
        return Ok(to_val(&TrackSetRes { ssrc, muted: None, duplex: None, noop: Some(true) }));
    }
    let room = pub_room.unwrap_or_else(|| req.room_id.clone());
    if d == Duplex::Full {
        if let Some(r) = st.rooms.get_mut(&room)
            && r.floor.srv.holder().map(String::as_str) == Some(user.as_str())
        {
            let s1 = r.floor.step(now, |sv, e| crate::flr::core::ws_pub_away(sv, e, &user));
            exec.floor(&room, r.hot.clone(), "duplex_full", Some(&user), s1);
            let s2 = r.floor.step(now, |sv, e| crate::flr::core::ws_pub_select(sv, e, &user));
            exec.floor(&room, r.hot.clone(), "pub_select", Some(&user), s2);
        }
        s.set_duplex(Duplex::Full);
        s.muted.store(false, Ordering::Release);
        let item = item_of(&s);
        if let Some(r) = st.rooms.get_mut(&room) {
            r.tracks.insert(item.track_id.clone(), item.clone());
            r.bump();
        }
        if let Some(r) = st.rooms.get(&room) {
            track_event(sfu, r, PublishTracksReqAddAction::Add, vec![item.entry(&room)], "TRK.S.half_to_full", &user);
        }
        track_on_room(sfu, st, &room, &s.track_id);
        if s.kind == Kind::Video {
            crate::rtcp::keyframe_burst(&s, &[0, 500, 1_500]);
        }
    } else {
        s.set_duplex(Duplex::Half);
        s.muted.store(false, Ordering::Release);
        let item = item_of(&s);
        if let Some(r) = st.rooms.get_mut(&room) {
            r.tracks.insert(item.track_id.clone(), item.clone());
            r.bump();
        }
        if let Some(r) = st.rooms.get(&room) {
            track_event(sfu, r, PublishTracksReqAddAction::Add, vec![item.entry(&room)], "TRK.S.full_to_half", &user);
        }
        track_off_room(sfu, st, &room, &s.track_id);
        let rooms: Vec<String> = st.peers.get(&user).map(|p| p.rooms.iter().cloned().collect()).unwrap_or_default();
        for rid in rooms {
            if let Some(r) = st.rooms.get_mut(&rid) {
                let s1 = r.floor.step(now, |sv, e| crate::flr::core::ws_reg_half(sv, e, &user));
                exec.floor(&rid, r.hot.clone(), "reg_half", Some(&user), s1);
            }
        }
    }
    common::ev!(spec = "PUB.S.set_duplex", area = "PUB", key = %user, from = ?d, to = d.as_str(), cause = "TRACK_SET", user = %user, track = &*s.track_id);
    Ok(to_val(&TrackSetRes { ssrc, muted: None, duplex: Some(d), noop: None }))
}

fn ready(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: ReadyReq = parse(body).map_err(|_| fail(Code::MISSING_FIELD))?;
    let user = ctx.user_id.clone();
    let now = now_ms();
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let p = st.peers.get_mut(&user).ok_or_else(|| fail(Code::NOT_IN_ROOM))?;
    if !p.rooms.contains(&req.room_id) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    p.gate(&req.room_id).store(true, Ordering::Release);
    p.first_ready.entry(req.room_id.clone()).or_insert(common::time::wall_ms());
    let before = p.table.ack;
    let acked = p.table.ack(req.arev);
    common::ev!(spec = "SUB.S.ready", area = "SUB", key = %user, from = "gated", to = "open", cause = "READY", user = %user, room = %req.room_id, arev = req.arev);
    common::ev!(spec = if acked { "LINK.S.ack" } else { "LINK.S.ack_old" }, area = "LINK", key = %user, from = before, to = p.table.ack, cause = "READY", user = %user, arev = req.arev, in.a = req.arev);
    let mut pli: Vec<Arc<crate::fwd::hot::Downlink>> = Vec::new();
    for r in p.table.rows() {
        if r.kind == Kind::Video && r.track.as_ref().is_some_and(|t| t.1 == req.room_id) {
            pli.push(r.dl.clone());
        }
        if r.track.is_some() && r.since <= p.table.ack && !p.sub_active.get(&r.mid).copied().unwrap_or(false) {
            p.sub_active.insert(r.mid, true);
            common::ev!(spec = common::log::SPEC_SUBSCRIBE_STATE, area = "SUB", key = %r.mid, from = "Created", to = "Active", cause = "READY", user = %user, arev = req.arev);
        }
    }
    exec.pli.extend(pli);
    if let Some(r) = st.rooms.get_mut(&req.room_id) {
        let s = r.floor.step(now, |s, e| crate::flr::core::ready(s, e, &user));
        exec.floor(&req.room_id, r.hot.clone(), "ready", Some(&user), s);
    }
    Ok(json!({}))
}

fn subscribe_layer(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: SubscribeLayerReq = parse(body)?;
    let user = ctx.user_id.clone();
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    if !st.peers.get(&user).is_some_and(|p| p.rooms.contains(&req.room_id)) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    let mut changed = false;
    for t in &req.targets {
        let Some(item) = st.rooms.get(&req.room_id).and_then(|r| r.tracks.get(&t.track_id).cloned()) else { continue };
        let (was_paused, spatial) = {
            let Some(p) = st.peers.get_mut(&user) else { continue };
            let e = p.prefs.entry(t.track_id.clone()).or_insert(super::peer::Prefs { spatial: 1, paused: false, priority: 0 });
            let was = e.paused;
            if let Some(s) = t.spatial {
                e.spatial = s.min(1);
            }
            if let Some(pa) = t.paused {
                e.paused = pa;
            }
            if let Some(pr) = t.priority {
                e.priority = pr;
            }
            (was, e.spatial)
        };
        let now_paused = st.peers.get(&user).and_then(|p| p.prefs.get(&t.track_id)).is_some_and(|x| x.paused);
        let rank = rank_of(st, &user, &t.track_id, item.kind);
        if let Some(p) = st.peers.get_mut(&user) {
            p.table.set_rank(&t.track_id, &req.room_id, rank);
            if let Some(row) = p.table.find(&t.track_id, &req.room_id) {
                row.dl.want_layer.store(spatial, Ordering::Release);
                row.dl.paused.store(now_paused, Ordering::Release);
            }
        }
        if now_paused && !was_paused {
            changed |= rows_off(st, &user, &t.track_id, &req.room_id);
        } else if !now_paused && was_paused && item.active {
            let w = want_of(st, &user, &req.room_id, &item);
            changed |= rows_on(st, &user, vec![w]).unwrap_or(false);
            if let Some(s) = stream_of(st, &t.track_id) {
                exec.burst.push(s);
            }
        } else if let Some(p) = st.peers.get(&user)
            && let Some(row) = p.table.find(&t.track_id, &req.room_id)
        {
            exec.layer_pli.push(row.dl.clone());
        }
    }
    if changed && let Some(p) = st.peers.get(&user) {
        sfu.mline_event(p);
    }
    Ok(json!({}))
}

fn ice_restart(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value) -> Res {
    let req: IceRestartReq = parse(body)?;
    if req.sfu_id != sfu.epoch {
        return Err(fail(Code::PEER_NOT_FOUND));
    }
    let guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let Some(p) = guard.peers.get(&ctx.user_id) else {
        common::ev!(spec = "LINK.S.restart_3010", area = "LINK", key = %ctx.user_id, cause = "ICE_RESTART", impossible = 1, user = %ctx.user_id);
        return Err(fail(Code::PEER_NOT_FOUND));
    };
    let mut creds = Vec::new();
    for t in p.transports() {
        let c = crate::link::Creds::generate();
        let dropped = t.ledger.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?.restart(c.clone());
        if let Some(d) = dropped {
            sfu.io.unregister_ufrag(&d.ufrag);
        }
        sfu.io.register_ufrag(&c.ufrag, &t);
        common::ev!(spec = "LINK.S.restart", area = "LINK", key = t.id, from = "use", to = "pend", cause = "ICE_RESTART", user = %ctx.user_id);
        creds.push(c);
    }
    let publish = creds.first().map(|c| IceRestartResPublish { ufrag: c.ufrag.clone(), pwd: c.pwd.clone() }).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
    let subscribe = creds.get(1).map(|c| IceRestartResSubscribe { ufrag: c.ufrag.clone(), pwd: c.pwd.clone() });
    Ok(to_val(&IceRestartRes { publish, subscribe }))
}

fn affiliation(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value, exec: &mut Exec) -> Res {
    let req: AffiliationReq = parse(body)?;
    let user = ctx.user_id.clone();
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let p = st.peers.get(&user).ok_or_else(|| fail(Code::NOT_IN_ROOM))?;
    let sel = req.pub_select.clone().filter(|r| p.rooms.contains(r));
    if req.pub_select.is_some() && sel.is_none() && st.rooms.contains_key(req.pub_select.as_deref().unwrap_or("")) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    let desel = req.pub_deselect.clone().filter(|r| p.rooms.contains(r));
    let (mut bad, mut noperm) = (false, false);
    if let Some(s) = &sel
        && p.pub_room.as_deref() != Some(s.as_str())
        && let Some(r) = st.rooms.get(s)
    {
        let perm = r.permission_of(&user, p.ctx.permission);
        (bad, noperm) = crate::r#pub::move_check(&p.streams, &perm, &r.audio_codec, &r.video_codec);
    }
    let cur = p.pub_room.clone();
    let step = crate::aff::decide(cur.as_deref(), sel.as_deref(), desel.as_deref(), bad, noperm);
    common::ev!(spec = step.label(), area = "AFF", key = %user, from = ?cur, to = ?sel, cause = "AFFILIATION", user = %user, in.bad = bad, in.noperm = noperm);
    match step {
        crate::aff::Step::Reject => return Err(fail(Code::CODEC_MISMATCH)),
        crate::aff::Step::RejectPerm => return Err(fail(Code::NOT_AUTHORIZED)),
        crate::aff::Step::Select | crate::aff::Step::SelectMoved => set_pub_room(sfu, st, &user, sel.as_deref(), exec),
        crate::aff::Step::Away => set_pub_room(sfu, st, &user, None, exec),
        crate::aff::Step::Same => {}
    }
    let p = st.peers.get(&user).ok_or_else(|| fail(Code::INTERNAL_ERROR))?;
    Ok(to_val(&AffiliationRes { sub_rooms: p.rooms.iter().cloned().collect(), pub_room: p.pub_room.clone(), cause: AffiliationResCause::User, change_id: req.change_id }))
}

fn message(sfu: &Arc<Sfu>, ctx: &SessionCtx, body: &Value) -> Res {
    let req: MessageReq = parse(body)?;
    let guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    if !guard.peers.get(&ctx.user_id).is_some_and(|p| p.rooms.contains(&req.room_id)) {
        return Err(fail(Code::NOT_IN_ROOM));
    }
    let b = MessageBody { room_id: req.room_id.clone(), user_id: ctx.user_id.clone(), content: req.content };
    sfu.notice_room(&req.room_id, Op::Message, to_val(&b), vec![ctx.user_id.clone()]);
    Ok(to_val(&MessageRes { msg_id: uuid::Uuid::new_v4().to_string() }))
}

fn resume(sfu: &Arc<Sfu>, ctx: &SessionCtx) -> Res {
    let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let st = &mut *guard;
    let Some(p) = st.peers.get_mut(&ctx.user_id) else {
        return Ok(json!({"snapshot": {}, "publications": [], "mline_table": null}));
    };
    p.home = (ctx.home_node.clone(), ctx.home_inst.clone());
    p.ctx = ctx.clone();
    let rooms: Vec<String> = p.rooms.iter().cloned().collect();
    let pubs: Vec<Publication> = p.streams.values().map(|s| s.publication(p.pub_room.as_deref().unwrap_or(""))).collect();
    let table = p.table.wire();
    let mut snap = BTreeMap::new();
    for r in rooms {
        if let Some(room) = st.rooms.get(&r)
            && let Some(s) = room.snapshot(&ctx.user_id)
        {
            snap.insert(r, s);
        }
    }
    Ok(json!({"snapshot": snap, "publications": pubs, "mline_table": table}))
}

async fn create(sfu: &Arc<Sfu>, room_id: &str, name: &str, capacity: u32, unused: u32, departure: u32, codecs: Option<MediaCodecs>) -> Res {
    let c = codecs.unwrap_or_default();
    let audio = match &c.audio {
        Some(a) => Some(crate::trk::canonical_audio(a).ok_or_else(|| fail(Code::CODEC_REQUIRED).details(json!({"supported": crate::trk::SUPPORTED_AUDIO})))?.to_string()),
        None => None,
    };
    let video = match &c.video {
        Some(v) => Some(crate::trk::canonical_video(v).ok_or_else(|| fail(Code::CODEC_REQUIRED).details(json!({"supported": crate::trk::SUPPORTED_VIDEO})))?.to_string()),
        None => None,
    };
    let codecs = MediaCodecs { audio, video };
    let (res, new) = {
        let mut guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
        let st = &mut *guard;
        if let Some(r) = st.rooms.get(room_id) {
            (CreateRoomRes { room_id: r.id.clone(), name: r.name.clone(), capacity: r.capacity, created_at: r.created_at, media_codecs: r.media_codecs() }, false)
        } else {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let resolved_v = codecs.video.clone().unwrap_or_else(|| crate::trk::DEFAULT_VIDEO.to_string());
            let resolved_a = codecs.audio.clone().unwrap_or_else(|| crate::trk::AUDIO_CODEC.to_string());
            let hot = Arc::new(RoomHot { id: Arc::from(room_id), audio: Slot::new(Kind::Audio, room_id), video: Slot::new(Kind::Video, room_id), audio_codec: Arc::from(resolved_a.as_str()), video_codec: Arc::from(resolved_v.as_str()), floor_tx: tx });
            let fl = crate::flr::Floor::new(usize::from(sfu.cfg.policy.floor.queue_max), u32::from(sfu.cfg.policy.floor.t2_stop_talking_secs));
            let r = Room::new(room_id, name, capacity, &codecs, unused, departure, &sfu.epoch, hot, fl, now_ms());
            let res = CreateRoomRes { room_id: r.id.clone(), name: r.name.clone(), capacity: r.capacity, created_at: r.created_at, media_codecs: r.media_codecs() };
            st.rooms.insert(room_id.to_string(), r);
            floor::spawn_driver(sfu, room_id, rx);
            common::ev!(spec = "ROOM.S.reopen", area = "ROOM", key = %room_id, from = "-", to = "open", cause = "CREATE_ROOM", room = %room_id);
            (res, true)
        }
    };
    if new && let Err(e) = sfu.bus.declare_room(room_id).await {
        tracing::warn!(room = %room_id, error = %e, "room 토큰 선언 실패");
    }
    Ok(to_val(&res))
}

fn get(sfu: &Arc<Sfu>, room_id: &str, tracks: bool, me: Option<&str>) -> Res {
    let guard = sfu.state.lock().map_err(|_| fail(Code::INTERNAL_ERROR))?;
    let r = guard.rooms.get(room_id).ok_or_else(|| fail(Code::ROOM_NOT_FOUND))?;
    let res = GetRoomRes {
        room_id: r.id.clone(),
        name: r.name.clone(),
        capacity: r.capacity,
        user_count: r.user_count(),
        participants: r.participants(),
        me: me.and_then(|u| r.members.get(u).map(|m| m.info.clone())),
        created_at: r.created_at,
        media_codecs: r.media_codecs(),
        version: r.version(),
        tracks: tracks.then(|| r.track_entries()),
    };
    Ok(to_val(&res))
}

fn list(sfu: &Arc<Sfu>) -> Reply {
    let Ok(guard) = sfu.state.lock() else { return Reply::fail(Code::INTERNAL_ERROR) };
    let rooms: Vec<RoomSummary> = guard.rooms.values().map(Room::summary).collect();
    let total = rooms.len() as u32;
    Reply::ok(&ListRoomsRes { rooms, total })
}
