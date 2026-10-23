// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-AUX-DC-SEND · 정:IMPL-AUX-MESSAGE · 정:IMPL-FLR-LOCK · 연:TRN.frame.DC · 연:AUX.op.MESSAGE · 연:RULE-FLR-TRANSPORT-DC · model: claude-opus-5-5
//! AUX — DC 프레임 송수신(준비 전 버퍼 64 · 넘치면 오래된 것부터 버리고 센다 · 잘린 프레임은 버리고 센다) · MBCP 를 그 방 판정 로직으로 · `MESSAGE` 중계(신원은 봉투에서).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::bus::Notice;
use oxsig::aux::{MessageBody, MessageReq, MessageRes};
use oxsig::err::{ErrCode, Failure};
use serde_json::Value;

use crate::link::{Peer, Transport};

pub const DC_BUFFER: usize = 64;

pub static DC_BAD: AtomicU64 = AtomicU64::new(0);
pub static DC_UNSENT: AtomicU64 = AtomicU64::new(0);

pub fn on_dc(t: &Arc<Transport>, data: &[u8]) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some((svc, payload)) = oxsig::trn::decode_dc(data) else {
        DC_BAD.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if svc != oxsig::trn::SVC_MBCP {
        return;
    }
    let Some(m) = oxsig::flr::decode(payload) else {
        DC_BAD.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let Some(room_id) = m.room() else {
        DC_BAD.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let user = t.user_id.clone();
    tracing::info!(kind = "ev", spec = %format!("CP-MSG-{}", oxsig::flr::name(m.msg_type).replace('_', "-")), room = %room_id, user = %user, epoch = %ctx.boot.epoch, mbcp_seq = m.seq().unwrap_or(0), "mbcp in");
    let Some(core) = crate::flr::shell::wire_in(&m) else { return };
    let Some(room) = ctx.rooms.get(&room_id) else {
        if core.t == crate::flr::core::T::Request {
            let deny = crate::flr::core::Msg::new(crate::flr::core::T::Deny, None, oxsig::flr::deny::OTHER_REASON as u32, 0);
            if let Some(f) = crate::flr::shell::wire_out(&room_id, &deny) {
                t.dc_send(f);
            }
        }
        return;
    };
    room.floor.apply(oxsig::flr::name(m.msg_type), move |s| s.server_rx(&user, &core));
}

pub fn floor_send(room: &str, user: &str, frame: Vec<u8>) {
    let Some(ctx) = crate::cfg::get() else { return };
    let Some(p) = ctx.links.peer(user) else {
        DC_UNSENT.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !p.pub_t.dc_send(frame) {
        tracing::debug!(kind = "diag", room = %room, user = %user, "DC 미준비 — 버퍼");
    }
}

pub fn message(p: &Arc<Peer>, body: &Value) -> Result<Value, Failure> {
    let Some(ctx) = crate::cfg::get() else { return Err(Failure::new(ErrCode::InternalError)) };
    let req: MessageReq = serde_json::from_value(body.clone()).map_err(|e| Failure::new(ErrCode::InvalidPayload).msg(e.to_string()))?;
    let room = ctx.rooms.get(&req.room_id).ok_or_else(|| Failure::new(ErrCode::NotInRoom))?;
    if !room.has(&p.user_id) {
        return Err(Failure::new(ErrCode::NotInRoom));
    }
    let b = MessageBody { room_id: req.room_id.clone(), user_id: p.user_id.clone(), content: req.content };
    let n = Notice { op: oxsig::op::MESSAGE, body: serde_json::to_value(b).unwrap_or(Value::Null), exclude: vec![p.user_id.clone()], room_id: Some(req.room_id), sfu_id: None, control: None };
    room.emit(n);
    Ok(serde_json::to_value(MessageRes { msg_id: common::ident::uuid4() }).unwrap_or(Value::Null))
}
