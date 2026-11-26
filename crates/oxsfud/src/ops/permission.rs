// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ROOM-PERMISSION · 정:RULE-HOOK-PERMISSION · 연:RULE-ROOM-PERMISSION-MAP · 연:RULE-PUB-REVOKED · 연:DEV-ROOM-HIDDEN-SELF-PSTATE · 연:RULE-FLR-OWN-RULES · model: claude-opus-5-5
//! 대외 권한 제어의 집행 — ①멱등(같은 값이면 아무것도 안 한다) ②적용(오버라이드 표와 명단을 한 임계 구역에서) ③회수(내려간 발행 비트의 전이중 트랙을 발행 제거 길로 · 발언권은 상태기 입력으로) ④통지(②③ 뒤 · 투명이면 본인에게만 version 없이). 성공은 적용된 permission 넷.

use std::sync::Arc;

use common::envelope::{Envelope, PermissionCall};
use oxsig::msg::{Kind, ParticipantStateBody, ParticipantStateType, Permission};
use oxsig::{Code, Failure, Op};
use serde_json::Value;

use crate::r#pub::PubStream;
use crate::room::Room;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorMove {
    Keep,
    Down,
    Up,
}

impl FloorMove {
    pub fn as_str(self) -> &'static str {
        match self {
            FloorMove::Keep => "keep",
            FloorMove::Down => "down",
            FloorMove::Up => "up",
        }
    }
}

pub fn floor_perm(p: &Permission, half_kinds: &[Kind]) -> (bool, &'static str) {
    if !p.floor_request {
        return (false, "floor_request");
    }
    if half_kinds.contains(&Kind::Audio) && !p.publish_audio {
        return (false, "publish_audio");
    }
    if half_kinds.contains(&Kind::Video) && !p.publish_video {
        return (false, "publish_video");
    }
    (true, "allowed")
}

pub fn half_kinds(room: &str, user: &str) -> Vec<Kind> {
    super::media::half_in(room, user).iter().map(|st| st.kind).collect()
}

pub fn floor_move(before: bool, after: bool) -> FloorMove {
    match (before, after) {
        (true, false) => FloorMove::Down,
        (false, true) => FloorMove::Up,
        _ => FloorMove::Keep,
    }
}

pub fn lowered(old: &Permission, new: &Permission, streams: &[Arc<PubStream>], room: &str) -> Vec<Arc<PubStream>> {
    streams
        .iter()
        .filter(|st| !st.is_half() && crate::trk::listed(st, room))
        .filter(|st| crate::aff::bit_allows(old, st.kind, st.source) && !crate::aff::bit_allows(new, st.kind, st.source))
        .cloned()
        .collect()
}

pub fn apply(env: &Envelope) -> Result<Value, Failure> {
    let s = super::sfu().ok_or_else(|| Code::InternalError.failure())?;
    let call: PermissionCall = oxsig::code::body(&env.body)?;
    let user = env.target_user.as_deref().ok_or_else(|| Code::MissingField.failure().with_message("target_user"))?;
    let room = s.rooms.get(&call.room_id).ok_or_else(|| Code::RoomNotFound.failure())?;
    let (old, new, hidden) = room.set_permission(user, call.permission).ok_or_else(|| Code::NotInRoom.failure())?;
    if old == new {
        record(&room.id, user, &old, &new, "same", None);
        return serde_json::to_value(new).map_err(|_| Code::InternalError.failure());
    }
    let (removed, mv, why) = revoke(&room, user, &old, &new);
    notify(&room, user, new, hidden);
    record(&room.id, user, &old, &new, "applied", Some((mv, why, removed, hidden)));
    serde_json::to_value(new).map_err(|_| Code::InternalError.failure())
}

fn record(room: &str, user: &str, old: &Permission, new: &Permission, what: &str, done: Option<(FloorMove, &str, usize, bool)>) {
    let mut l = common::log::Line::diag("SV-IMPL-ROOM-PERMISSION")
        .key("room", room)
        .key("user", user)
        .key("session", super::session_of(user))
        .rec("old", bits(old))
        .rec("new", bits(new));
    if let Some((mv, why, removed, hidden)) = done {
        l = l.rec("floor", mv.as_str()).rec("floor_why", why).rec("removed", removed).rec("hidden", hidden);
    }
    common::log::emit(l.msg(&format!("permission {what}")));
}

fn revoke(room: &Arc<Room>, user: &str, old: &Permission, new: &Permission) -> (usize, FloorMove, &'static str) {
    let Some(s) = super::sfu() else { return (0, FloorMove::Keep, "no_sfu") };
    let streams = s.pubs.get(user).map(|c| c.streams()).unwrap_or_default();
    let drop = lowered(old, new, &streams, &room.id);
    if let Some(p) = s.peer(user)
        && !drop.is_empty()
    {
        super::media::remove_streams(&p, &drop);
    }
    let half = half_kinds(&room.id, user);
    let (before, _) = floor_perm(old, &half);
    let (after, why) = floor_perm(new, &half);
    let mv = floor_move(before, after);
    let u = user.to_string();
    match mv {
        FloorMove::Down => {
            super::floor::run(room, super::floor::Entry::of("perm_down", &u), |f, _| Some(f.perm_down(&u)));
        }
        FloorMove::Up => {
            super::floor::run(room, super::floor::Entry::of("perm_up", &u), |f, _| Some(f.perm_up(&u)));
        }
        FloorMove::Keep => {}
    }
    (drop.len(), mv, why)
}

fn notify(room: &Arc<Room>, user: &str, p: Permission, hidden: bool) {
    let body = |version| ParticipantStateBody { type_: ParticipantStateType::Permission, user_id: user.to_string(), room_id: room.id.clone(), version, permission: p };
    if hidden {
        super::user_notice(user, Op::ParticipantState, &body(None), Some(&room.id));
        return;
    }
    room.bump_and_send(|v| Some(crate::room::notice(Op::ParticipantState, &body(Some(v)), Vec::new(), &room.id)));
}

fn bits(p: &Permission) -> String {
    [("audio", p.publish_audio), ("video", p.publish_video), ("screen", p.publish_screen), ("floor", p.floor_request)]
        .iter()
        .map(|(n, on)| format!("{n}:{}", if *on { 1 } else { 0 }))
        .collect::<Vec<_>>()
        .join(",")
}
