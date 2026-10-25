// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-AUX-MESSAGE 정:IMPL-AUX-DC-SEND 정:RULE-BUS-REQUEST 연:AUX.op.MESSAGE · model: claude-opus-5-5
//! `MESSAGE` 중계 — `user_id` 는 세션에서 · 그 방 전원(발신자 제외) · 응답 `msg_id` = uuid.

use std::sync::Arc;

use oxsig::envelope::{Ctx, Reply};
use oxsig::msg::{MessageBody, MessageRes};
use oxsig::{Code, Op};

use crate::ops::Sfu;
use crate::ops::call::{body_json, need_str};

pub fn message(sfu: &Arc<Sfu>, ctx: &Ctx, body: &str) -> Reply {
    let v = match body_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let room = match need_str(&v, "room_id") {
        Ok(r) => r,
        Err(r) => return r,
    };
    let content = match need_str(&v, "content") {
        Ok(r) => r,
        Err(r) => return r,
    };
    let member = sfu.room(&room).is_some_and(|r| r.lock().members.contains_key(&ctx.user_id));
    if !member {
        return Reply::fail(Code::NotInRoom);
    }
    let b = MessageBody { room_id: room.clone(), user_id: ctx.user_id.clone(), content };
    sfu.emit_room(&room, Op::Message, &b, vec![ctx.user_id.clone()]);
    Reply::ok(&MessageRes { msg_id: common::id::uuid4() })
}
