// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-REQUEST · 정:RULE-OPS-SURFACE · 정:RULE-OPS-OBSERVE · 정:RULE-DOC-PLANES · 정:IMPL-LINK-PEER-STATE · 정:IMPL-TRK-FROM-PUB · model: claude-opus-5-5
//! OPS — node 안 gRPC Handle(Hello · Call) · 봉투를 영역 함수로 잇는 길 · 사유별 버림 계수(조용한 drop 금지) · 연결 의심 통지 · 첫 RTP 의 트랙 등록.

pub mod admin;
pub mod join;
pub mod publish;
pub mod wire;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::bus::Notice;
use common::handle::pb::handle_server::{Handle, HandleServer};
use common::handle::pb::{CallReq, CallRes, HelloReq, HelloRes};
use common::handle::{Call, Reply};
use oxsig::err::{ErrCode, Failure};
use oxsig::link::LinkEvent;
use serde_json::{json, Value};

use crate::link::Peer;
use crate::r#pub::PubStream;

#[derive(Default)]
pub struct Drops {
    pub unknown_ssrc: AtomicU64,
    pub probation: AtomicU64,
    pub no_pub_room: AtomicU64,
    pub floor_gate: AtomicU64,
    pub slot_unbound: AtomicU64,
    pub room_moved: AtomicU64,
    pub gate: AtomicU64,
    pub rtx_gate: AtomicU64,
    pub rtx_miss: AtomicU64,
    pub rtx_budget: AtomicU64,
}

impl Drops {
    pub fn view(&self) -> Value {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let Some(ctx) = crate::cfg::get() else { return json!({}) };
        json!({
            "unknown_ssrc": g(&self.unknown_ssrc),
            "probation": g(&self.probation),
            "no_pub_room": g(&self.no_pub_room),
            "floor_gate": g(&self.floor_gate),
            "slot_unbound": g(&self.slot_unbound),
            "room_moved": g(&self.room_moved),
            "ready_gate": g(&self.gate),
            "rtx_gate": g(&self.rtx_gate),
            "rtx_cache_miss": g(&self.rtx_miss),
            "rtx_budget": g(&self.rtx_budget),
            "stun_bad": g(&ctx.links.stun_bad),
            "unknown_addr": g(&ctx.links.unknown_addr),
            "dc_bad": g(&crate::aux::DC_BAD),
            "dc_unsent": g(&crate::aux::DC_UNSENT),
            "bus_dropped": g(&ctx.bus.dropped),
        })
    }
}

pub fn fail(c: ErrCode) -> Failure {
    Failure::new(c)
}

pub fn notice(op: u16, body: Value) -> Notice {
    Notice { op, body, exclude: Vec::new(), room_id: None, sfu_id: None, control: None }
}

pub fn link_suspect(p: &Arc<Peer>, ufrag: &str) {
    let Some(ctx) = crate::cfg::get() else { return };
    let ev = LinkEvent { kind_of: "suspect".to_string(), sfu_id: ctx.boot.epoch.clone(), ufrag: ufrag.to_string() };
    tracing::info!(kind = "ev", spec = "CP-OP-LINK-EVENT", user = %p.user_id, session = %p.session(), epoch = %ctx.boot.epoch, ufrag = %ufrag, label = "LINK.S.suspect", "연결 의심 통지");
    ctx.bus.put_user(&p.user_id, notice(oxsig::op::LINK_EVENT, serde_json::to_value(ev).unwrap_or_default()));
}

pub fn first_rtp(s: &Arc<PubStream>) {
    let Some(ctx) = crate::cfg::get() else { return };
    tracing::info!(kind = "ev", spec = "SV-STATE-PUBLISH-STATE", user = %s.user_id, track = %s.track_id, epoch = %ctx.boot.epoch, from = "intended", to = "active", "첫 RTP");
    if s.half.load(Ordering::Acquire) || !s.simulcast || s.listed.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(room) = ctx.rooms.get(&s.room_id()) else { return };
    let e = s.entry(room.media.video());
    ctx.tracks.put(&room, e.clone());
    wire::track_on(ctx, &room, &e);
}

pub async fn dispatch(call: Call) -> Reply {
    let Some(ctx) = crate::cfg::get() else { return Reply::fail(fail(ErrCode::InternalError)) };
    match call {
        Call::Op { env, op, body } => {
            let r = match op {
                oxsig::op::ROOM_JOIN => join::room_join(ctx, &env, &body).await,
                oxsig::op::ROOM_LEAVE => join::room_leave(ctx, &env, &body),
                oxsig::op::PUBLISH_TRACKS => publish::publish(ctx, &env, &body),
                oxsig::op::READY => publish::ready(ctx, &env, &body),
                oxsig::op::SUBSCRIBE_LAYER => publish::layer(ctx, &env, &body),
                oxsig::op::TRACK_SET => publish::track_set(ctx, &env, &body),
                oxsig::op::ICE_RESTART => publish::ice_restart(ctx, &env, &body),
                oxsig::op::AFFILIATION => publish::affiliation(ctx, &env, &body),
                oxsig::op::MESSAGE => match ctx.links.peer(&env.user_id) {
                    Some(p) => crate::aux::message(&p, &body),
                    None => Err(fail(ErrCode::NotInRoom)),
                },
                _ => Err(fail(ErrCode::UnknownOp)),
            };
            match r {
                Ok(v) => Reply::ok(v),
                Err(f) => {
                    tracing::info!(kind = "ev", spec = %format!("CP-CODE-{}", f.name.replace('_', "-")), user = %env.user_id, session = %env.session_id, epoch = %ctx.boot.epoch, op = %oxsig::op::name(op), code = f.code, "op 실패");
                    Reply::fail(f)
                }
            }
        }
        Call::SessionGone { session_id, user_id, path } => {
            let done = crate::gc::session_gone(ctx, &session_id, &user_id, &path);
            Reply::ok(json!({"reaped": done}))
        }
        Call::Resume { env } => Reply::ok(join::resume(ctx, &env)),
        Call::CreateRoom { req, unused_ttl_secs, departure_ttl_secs, capacity } => join::create_room(ctx, req, unused_ttl_secs, departure_ttl_secs, capacity).await,
        Call::GetRoom { room_id, tracks, session_id: _, user_id } => join::get_room(ctx, &room_id, tracks, user_id.as_deref()),
        Call::ListRooms => {
            let rooms: Vec<oxsig::room::RoomSummary> = ctx.rooms.all().iter().map(|r| r.summary()).collect();
            let total = rooms.len() as u32;
            Reply::ok(serde_json::to_value(oxsig::room::ListRoomsRes { rooms, total }).unwrap_or_default())
        }
        Call::Admin { method, path, query, body } => admin::admin(ctx, &method, &path, &query, &body),
        Call::LogLevel { level, ttl_secs } => match common::log::override_level(&level, ttl_secs) {
            Some((from, to, until)) => Reply::ok(json!({"from": from, "to": to, "until": until})),
            None => Reply::fail(fail(ErrCode::InvalidPayload)),
        },
    }
}

pub struct Svc {
    pub epoch: String,
    pub build: String,
}

#[tonic::async_trait]
impl Handle for Svc {
    async fn hello(&self, r: tonic::Request<HelloReq>) -> Result<tonic::Response<HelloRes>, tonic::Status> {
        let h = r.into_inner();
        tracing::info!(kind = "diag", hub = %h.node, "Hello");
        Ok(tonic::Response::new(HelloRes { epoch: self.epoch.clone(), build: self.build.clone() }))
    }

    async fn call(&self, r: tonic::Request<CallReq>) -> Result<tonic::Response<CallRes>, tonic::Status> {
        let reply = match Call::decode(&r.into_inner().envelope) {
            Ok(c) => dispatch(c).await,
            Err(e) => Reply::fail(fail(ErrCode::InvalidPayload).msg(e)),
        };
        Ok(tonic::Response::new(CallRes { reply: reply.encode() }))
    }
}

pub async fn serve(addr: std::net::SocketAddr, svc: Svc) -> Result<(), String> {
    tonic::transport::Server::builder().add_service(HandleServer::new(svc)).serve(addr).await.map_err(|e| e.to_string())
}
