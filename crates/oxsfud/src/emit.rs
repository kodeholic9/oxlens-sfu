// author: kodeholic (powered by Claude)
//! 통지 버스 — sfud→hub `Subscribe` 스트림의 원천(정§15-2 통지 배달). 방 broadcast 는 `room_id`+`exclude`, unicast 는 `target`.

use common::bplane::{Envelope, iop};
use oxsig::frame::{Header, encode_json};
use oxsig::op::Op;
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Envelope>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self { tx: broadcast::channel(4096).0 }
    }
}

impl EventBus {
    pub fn subscribe(&self) -> broadcast::Receiver<Envelope> {
        self.tx.subscribe()
    }

    fn send(&self, env: Envelope) {
        // 구독자(hub) 없음 = 버림. 통지는 재전송 자산이 아니다(연§3-4 — 재동기는 version 이 맡는다).
        let _ = self.tx.send(env);
    }

    /// 방 broadcast — hub 가 명단 사본으로 배달한다.
    pub fn room<T: Serialize>(&self, room_id: &str, op: Op, body: &T, exclude: &[String]) {
        let wire = encode_json(&Header::msg(op.code(), 0), &serde_json::to_value(body).unwrap_or_default());
        self.send(Envelope { session_id: String::new(), user_id: String::new(), room_id: room_id.to_owned(), target: String::new(), exclude: exclude.to_vec(), wire, pc_mode: String::new() });
    }

    /// 당사자 unicast — `ROOM_EVENT{affiliation}`·`sync_required`(정§5-3·§14-3).
    pub fn user<T: Serialize>(&self, room_id: &str, user_id: &str, op: Op, body: &T) {
        let wire = encode_json(&Header::msg(op.code(), 0), &serde_json::to_value(body).unwrap_or_default());
        self.send(Envelope { session_id: String::new(), user_id: String::new(), room_id: room_id.to_owned(), target: user_id.to_owned(), exclude: Vec::new(), wire, pc_mode: String::new() });
    }

    /// 정§4-1 생성 완료 통보 / 폭파 통보 — hub 가 배치를 배우고 푼다.
    pub fn lifecycle(&self, room_id: &str, created: bool) {
        let body = serde_json::json!({ "type": if created { "created" } else { "destroyed" }, "room_id": room_id });
        let wire = encode_json(&Header::msg(iop::ROOM_LIFECYCLE, 0), &body);
        self.send(Envelope { session_id: String::new(), user_id: String::new(), room_id: room_id.to_owned(), target: String::new(), exclude: Vec::new(), wire, pc_mode: String::new() });
    }
}
