// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-AUX-MESSAGE · 정:IMPL-AUX-DC-SEND · 연:AUX.op.MESSAGE · 연:RULE-AUX-MESSAGE-FORM · 연:RULE-AUX-MESSAGE-PROC · model: claude-opus-5-5
//! 부가 통신 — `MESSAGE` 중계(방 방송 · 발신자 제외 · 입장한 방만 · `user_id` 는 봉투에서 · `msg_id` uuid).

use std::sync::Arc;

use common::bus::envelope::Reply;
use oxsig::{parse, Code, MessageBody, MessageReq, MessageRes, Op};

use crate::ops::call::Who;
use crate::ops::Sfu;

pub async fn message(sfu: &Arc<Sfu>, who: &Who, body: serde_json::Value) -> Reply {
    let req: MessageReq = match parse::typed(body) {
        Ok(r) => r,
        Err(e) => return sfu.fail(e.code().failure_with(e.message(), None)),
    };
    let w = sfu.world.lock().await;
    if !w.rooms.contains_key(&req.room_id) {
        return sfu.fail(Code::RoomNotFound.failure());
    }
    if !w.rooms.get(&req.room_id).map(|r| r.has(&who.user)).unwrap_or(false) {
        return sfu.fail(Code::NotInRoom.failure());
    }
    let msg_id = uuid::Uuid::new_v4().to_string();
    let out = MessageBody { room_id: req.room_id.clone(), user_id: who.user.clone(), content: req.content };
    sfu.to_room(&req.room_id, Op::Message, serde_json::to_value(out).unwrap_or_default(), vec![who.user.clone()]);
    tracing::info!(kind = "ev", spec = "CP-OP-MESSAGE", user = %who.user, room = %req.room_id, "message relayed");
    Reply::ok(&MessageRes { msg_id })
}
