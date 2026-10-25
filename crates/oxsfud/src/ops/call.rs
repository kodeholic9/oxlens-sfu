// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-REQUEST 정:CAND-BUS-HANDLE-SHAPE 정:IMPL-ERR-ISSUERS 정:IMPL-SES-HANDOVER 정:RULE-OPS-LOGGER 운영:RULE-CTL-LOG-LEVEL · model: claude-opus-5-5
//! 봉투 갈래 — op 하나는 그 Peer 의 단일 큐로 · 봉투 홈을 따른다 · 세션 폐기 · RESUME · 방 생성 · 상세 · 목록 · 운영 · 로그 수준.

use std::sync::Arc;

use oxsig::envelope::{Call, Ctx, Reply};
use oxsig::{Code, Op};
use serde_json::Value;

use super::Sfu;

pub fn body_json(body: &str) -> Result<Value, Reply> {
    if body.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(body).map_err(|_| Reply::fail(Code::InvalidPayload))
}

pub fn need_str(v: &Value, k: &str) -> Result<String, Reply> {
    match v.get(k) {
        None | Some(Value::Null) => Err(Reply::failure(Code::MissingField.failure_msg(k.to_string()))),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(Reply::failure(Code::InvalidPayload.failure_msg(k.to_string()))),
    }
}

async fn op(sfu: &Arc<Sfu>, ctx: Ctx, op: u16, body: String) -> Reply {
    let Some(o) = Op::from_u16(op) else { return Reply::fail(Code::UnknownOp) };
    if let Some(p) = sfu.peer(&ctx.user_id)
        && p.session() == ctx.session
    {
        p.follow_home(&ctx.home);
    }
    let r = match o {
        Op::RoomJoin => super::join::room_join(sfu, &ctx, &body).await,
        Op::RoomLeave => super::join::room_leave(sfu, &ctx, &body).await,
        Op::IceRestart => super::join::ice_restart(sfu, &ctx, &body).await,
        Op::PublishTracks => super::publish::publish_tracks(sfu, &ctx, &body).await,
        Op::TrackSet => super::publish::track_set(sfu, &ctx, &body).await,
        Op::Ready => super::sub::ready(sfu, &ctx, &body).await,
        Op::SubscribeLayer => super::sub::subscribe_layer(sfu, &ctx, &body).await,
        Op::Affiliation => super::aff::affiliation(sfu, &ctx, &body).await,
        Op::Message => crate::aux::message(sfu, &ctx, &body),
        _ => Reply::fail(Code::UnknownOp),
    };
    let code = match &r {
        Reply::Fail { failure } => failure.code.to_string(),
        _ => "ok".into(),
    };
    common::ev!(spec = o.catalog_id().as_str(), user = ctx.user_id.as_str(), session = ctx.session.as_str(), "op {} result={code}", o.name());
    r
}

pub async fn dispatch(sfu: &Arc<Sfu>, c: Call) -> Reply {
    match c {
        Call::Op { ctx, op: n, body } => op(sfu, ctx, n, body).await,
        Call::SessionGone { user_id, session } => super::join::session_gone(sfu, &user_id, &session).await,
        Call::Resume { ctx } => super::join::resume(sfu, &ctx).await,
        Call::CreateRoom { room_id, req } => super::join::create_room(sfu, &room_id, req).await,
        Call::GetRoom { room_id, tracks, viewer } => super::join::get_room(sfu, &room_id, tracks, viewer.as_deref()),
        Call::ListRooms => super::join::list_rooms(sfu),
        Call::Admin { req } => super::admin::handle(sfu, req).await,
        Call::LogLevel { level, ttl_secs } => match common::log::parse_level(&level) {
            Some(l) => {
                let ch = sfu.log.override_level(l, ttl_secs);
                common::ev!(spec = "SV-STATE-UNIT-STATE", "log level {} -> {} until={:?}", common::log::level_name(ch.from), common::log::level_name(ch.to), ch.until);
                Reply::ok(&serde_json::json!({ "from": common::log::level_name(ch.from), "to": common::log::level_name(ch.to), "until": ch.until }))
            }
            None => Reply::fail(Code::InvalidPayload),
        },
    }
}
