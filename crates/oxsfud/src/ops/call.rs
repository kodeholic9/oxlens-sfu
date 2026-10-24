// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-REQUEST · 정:CAND-BUS-HANDLE-SHAPE · 정:IMPL-SES-RESUME · 정:IMPL-SES-HANDOVER · 정:IMPL-ERR-ISSUERS · 연:RULE-TRN-FLOW-CONTROL · model: claude-opus-5-5
//! 봉투 갈래 — op(방 귀속 · 세션 귀속) · 세션 폐기 · RESUME 조각 · 방 생성 · 방 상세 · 방 목록 · 운영 · 로그 수준. `user_id` 와 홈은 봉투가 실은 값만 믿는다.

use std::sync::Arc;

use common::bus::envelope::{Call, Reply};
use oxsig::{Code, Op};

use super::Sfu;

pub struct Who {
    pub user: String,
    pub session: String,
    pub pc_mode: oxsig::PcMode,
    pub home: common::bus::envelope::Home,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Option<oxsig::Permission>,
}

pub async fn dispatch(sfu: &Arc<Sfu>, c: Call) -> Reply {
    match c {
        Call::Op { op, body, user_id, session, pc_mode, home, participant_type, hidden, metadata, permission } => {
            let who = Who { user: user_id, session, pc_mode, home, participant_type, hidden, metadata, permission };
            let Some(o) = Op::from_u16(op) else { return sfu.fail(Code::UnknownOp.failure()) };
            if let Some(p) = sfu.peer(&who.user)
                && p.session == who.session
            {
                p.set_home(&who.home);
            }
            match o {
                Op::RoomJoin => super::rooms::join(sfu, &who, body).await,
                Op::RoomLeave => super::rooms::leave(sfu, &who, body).await,
                Op::PublishTracks => super::pubs::publish(sfu, &who, body).await,
                Op::TrackSet => super::pubs::track_set(sfu, &who, body).await,
                Op::Affiliation => super::pubs::affiliation(sfu, &who, body).await,
                Op::Ready => super::subs::ready(sfu, &who, body).await,
                Op::SubscribeLayer => super::subs::subscribe_layer(sfu, &who, body).await,
                Op::IceRestart => super::pubs::ice_restart(sfu, &who, body).await,
                Op::Message => crate::aux::message(sfu, &who, body).await,
                _ => sfu.fail(Code::UnknownOp.failure()),
            }
        }
        Call::SessionGone { user_id, session, cause } => super::rooms::session_gone(sfu, &user_id, &session, cause.as_str()).await,
        Call::Resume { user_id, session, home } => super::rooms::resume(sfu, &user_id, &session, &home).await,
        Call::CreateRoom { req, room_id, unused_ttl_secs, departure_ttl_secs, capacity } => super::rooms::create(sfu, req, room_id, unused_ttl_secs, departure_ttl_secs, capacity).await,
        Call::GetRoom { room_id, tracks, user_id } => super::rooms::get(sfu, &room_id, tracks, user_id.as_deref()).await,
        Call::ListRooms => super::rooms::list(sfu).await,
        Call::Admin { what } => super::admin::admin(sfu, what).await,
        Call::LogLevel { level, ttl_secs } => match common::policy::LoggingLevel::parse(&level) {
            Some(l) => {
                let (from, until) = common::log::override_level(l, ttl_secs);
                tracing::info!(kind = "ev", spec = "OP-PATH-ADMIN-LOG-LEVEL", from = from.as_str(), to = l.as_str(), "log level overridden");
                Reply::Ok(serde_json::json!({"from": from.as_str(), "to": l.as_str(), "until": until}))
            }
            None => sfu.fail(Code::InvalidPayload.failure_with("level", None)),
        },
    }
}
