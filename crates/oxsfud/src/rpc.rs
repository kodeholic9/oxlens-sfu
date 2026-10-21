// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-REQUEST 정:RULE-OPS-SURFACE 정:IMPL-ERR-ISSUERS · model: claude-opus-5-5
//! B 평면 Unit 서버 — 게이트웨이가 넘긴 봉투(`UnitReq`)를 받아 판정하고 `UnitRes` 로 답한다. 실패 wire 는 게이트웨이가 그대로 통과한다.

use std::sync::Arc;

use common::rpc::{Blob, Unit, UnitServer};
use oxsig::Op;
use oxsig::bplane::{Caller, Hello, UnitReq, UnitRes};
use oxsig::code::Code;

use crate::publish::Fail;
use crate::sfu::Sfu;

struct Svc {
    sfu: Arc<Sfu>,
}

#[tonic::async_trait]
impl Unit for Svc {
    async fn call(&self, req: tonic::Request<Blob>) -> Result<tonic::Response<Blob>, tonic::Status> {
        let r: UnitReq = oxsig::bplane::decode(&req.into_inner().json).map_err(|e| tonic::Status::invalid_argument(e.to_string()))?;
        let sfu = self.sfu.clone();
        let res = tokio::task::spawn_blocking(move || dispatch(&sfu, r)).await.unwrap_or(UnitRes::Fail {
            code: Code::INTERNAL_ERROR.0,
            message: "dispatch panicked".into(),
            details: None,
        });
        Ok(tonic::Response::new(common::rpc::blob(&res)))
    }
}

pub async fn serve(sfu: Arc<Sfu>, addr: &str) -> Result<(), String> {
    let a: std::net::SocketAddr = addr.parse().map_err(|e: std::net::AddrParseError| e.to_string())?;
    tonic::transport::Server::builder()
        .add_service(UnitServer::new(Svc { sfu }))
        .serve(a)
        .await
        .map_err(|e| e.to_string())
}

fn frame(sfu: &Sfu, r: Result<String, Fail>) -> UnitRes {
    match r {
        Ok(body) => UnitRes::Frame { ok: true, body },
        Err((code, message, details)) => {
            sfu.counters.code(code.0);
            UnitRes::Fail { code: code.0, message, details }
        }
    }
}

fn two(r: Result<String, (Code, String)>) -> Result<String, Fail> {
    r.map_err(|(c, m)| (c, m, None))
}

fn room_op(sfu: &Arc<Sfu>, c: &Caller, op: u16, body: &str) -> UnitRes {
    let r = match Op::from_u16(op) {
        Some(Op::RoomJoin) => crate::session::join(sfu, c, body),
        Some(Op::RoomLeave) => crate::session::leave(sfu, c, body),
        Some(Op::IceRestart) => crate::session::ice_restart(sfu, c, body),
        Some(Op::Message) => crate::session::message(sfu, c, body),
        Some(Op::Affiliation) => crate::session::affiliation(sfu, c, body),
        Some(Op::PublishTracks) => match sfu.peer(&c.session_id) {
            Some(p) => crate::publish::publish(sfu, &p, body),
            None => Err((Code::NOT_IN_ROOM, "not in room".into(), None)),
        },
        Some(Op::TrackSet) => match sfu.peer(&c.session_id) {
            Some(p) => crate::publish::track_set(sfu, &p, body),
            None => Err((Code::NOT_IN_ROOM, "not in room".into(), None)),
        },
        Some(Op::Ready) => match sfu.peer(&c.session_id) {
            Some(p) => two(crate::table::ready(sfu, &p, body)),
            None => {
                let room = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v.get("room_id").and_then(|r| r.as_str().map(str::to_string)));
                match room {
                    None => Err((Code::MISSING_FIELD, "room_id".into(), None)),
                    Some(r) if sfu.room(&r).is_none() => Err((Code::ROOM_NOT_FOUND, "room not found".into(), None)),
                    Some(_) => Err((Code::NOT_IN_ROOM, "not in room".into(), None)),
                }
            }
        },
        Some(Op::SubscribeLayer) => match sfu.peer(&c.session_id) {
            Some(p) => two(crate::table::subscribe_layer(sfu, &p, body)),
            None => Err((Code::NOT_IN_ROOM, "not in room".into(), None)),
        },
        _ => Err((Code::UNKNOWN_OP, format!("op {op:#06x}"), None)),
    };
    frame(sfu, r)
}

pub fn dispatch(sfu: &Arc<Sfu>, r: UnitReq) -> UnitRes {
    sfu.beat.store(common::ident::now_ms(), std::sync::atomic::Ordering::Relaxed);
    match r {
        UnitReq::Hello => UnitRes::Hello(Hello { epoch: sfu.epoch.clone(), build: sfu.build.clone(), node: sfu.node.clone() }),
        UnitReq::RoomOp { caller, op, body } => room_op(sfu, &caller, op, &body),
        UnitReq::SessionEnd { user_id, session_id, path } => {
            crate::session::session_end(sfu, &user_id, &session_id, path);
            UnitRes::Done
        }
        UnitReq::Resume { user_id, session_id } => UnitRes::Resume(crate::session::resume(sfu, &user_id, &session_id)),
        UnitReq::CreateRoom { req, capacity, unused_ttl_secs, departure_ttl_secs } => {
            match crate::room::create(sfu, &req, capacity, unused_ttl_secs, departure_ttl_secs) {
                Ok(c) => UnitRes::Created(c),
                Err((code, message)) => UnitRes::Fail { code: code.0, message, details: None },
            }
        }
        UnitReq::GetRoom { room_id, viewer, tracks } => match crate::room::get(sfu, &room_id, viewer.as_deref(), tracks) {
            Ok(r) => UnitRes::Room(Box::new(r)),
            Err((code, message)) => UnitRes::Fail { code: code.0, message, details: None },
        },
        UnitReq::ListRooms => UnitRes::Rooms(crate::room::list(sfu)),
        UnitReq::Admin(a) => crate::admin::handle(sfu, a),
    }
}
