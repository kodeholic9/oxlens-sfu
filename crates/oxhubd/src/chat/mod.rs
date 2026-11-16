// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-GATEWAY-MODULE · 정:IMPL-CHAT-STORE · 정:IMPL-CHAT-FANOUT · 정:IMPL-CHAT-FILTER · 정:IMPL-CHAT-READ · 연:CHAT.op.MESSAGE · 연:CHAT.op.SPACE_HISTORY · 연:CHAT.op.SPACE_LIST · 연:CHAT.op.SPACE_VIEW · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-RECV · 연:RULE-CHAT-DISABLED · 연:RULE-CHAT-LIVE-SOCKET-LOSS · model: claude-opus-5-5
//! 채팅 — 게이트웨이 안 모듈 · op 넷 · 저장이 낸 `{멤버 판, 보낸 세션, 메시지}` 를 다른 node 에 뿌린다(실패 수를 돌려주고 다시 안 보낸다) · 공간마다 멤버 캐시로 `명단(t_M)` 을 낸다. 세션은 모른다 — 세션이 이 모듈을 부른다.

pub mod store;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oxsig::msg::{ChatMessage, MessageRes, SpaceHistoryRes, SpaceListRes, SpaceViewRes};
use oxsig::{Code, Failure, Op};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bus::Bus;
use store::{Dir, HISTORY_DEFAULT, HISTORY_MAX, Roster, Sent, Store};

pub const FANOUT_WHAT: &str = "chat";

pub struct Chat {
    pub store: Arc<Store>,
    cache: Mutex<HashMap<String, Roster>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deliver {
    pub ver: u64,
    pub sid: String,
    pub message: ChatMessage,
}

pub struct Answer {
    pub body: Value,
    pub deliver: Option<Deliver>,
}

impl Chat {
    pub fn new(store: Store) -> Chat {
        Chat { store: Arc::new(store), cache: Mutex::new(HashMap::new()) }
    }

    pub async fn roster_at_least(&self, space: &str, ver: u64) -> Result<Roster, Code> {
        let hit = self.cache.lock().ok().and_then(|g| g.get(space).cloned());
        if let Some(r) = hit
            && r.ver >= ver
        {
            return Ok(r);
        }
        let st = self.store.clone();
        let sp = space.to_string();
        let r = blocking(move || st.roster(&sp)).await?;
        if let Ok(mut g) = self.cache.lock() {
            let keep = g.get(space).is_some_and(|old| old.ver > r.ver);
            if !keep {
                g.insert(space.to_string(), r.clone());
            }
        }
        Ok(r)
    }

    pub async fn recipients(&self, d: &Deliver) -> Vec<String> {
        match self.roster_at_least(&d.message.space_id, d.ver).await {
            Ok(r) => r.at(d.message.t),
            Err(_) => Vec::new(),
        }
    }

    pub async fn handle(&self, user: &str, sid: &str, op: Op, body: Value) -> Result<Answer, Failure> {
        let user = user.to_string();
        match op {
            Op::Message => {
                let space: String = oxsig::code::need(&body, "space_id")?;
                let msg_id: String = oxsig::code::need(&body, "msg_id")?;
                let content: String = oxsig::code::need(&body, "content")?;
                if !valid_msg_id(&msg_id, &user) {
                    return Err(Code::InvalidPayload.failure().with_message("msg_id"));
                }
                let st = self.store.clone();
                let now = common::now_ms();
                let sent = blocking(move || st.send(&user, &space, &msg_id, &content, now)).await.map_err(Code::failure)?;
                let (msg, deliver) = match sent {
                    Sent::Fresh { msg, ver } => (msg.clone(), Some(Deliver { ver, sid: sid.to_string(), message: msg })),
                    Sent::Again { msg } => (msg, None),
                };
                Ok(Answer { body: json(&MessageRes { t: msg.t })?, deliver })
            }
            Op::SpaceHistory => {
                let space: String = oxsig::code::need(&body, "space_id")?;
                let after: Option<u64> = oxsig::code::field(&body, "after")?;
                let before: Option<u64> = oxsig::code::field(&body, "before")?;
                let limit: Option<u32> = oxsig::code::field(&body, "limit")?;
                let limit = limit.unwrap_or(HISTORY_DEFAULT);
                if limit > HISTORY_MAX {
                    return Err(Code::InvalidPayload.failure().with_message("limit"));
                }
                let dir = match (after, before) {
                    (Some(_), Some(_)) => return Err(Code::FieldConflict.failure().with_message("after xor before")),
                    (Some(a), None) => Dir::After(a),
                    (None, Some(b)) => Dir::Before(b),
                    (None, None) => Dir::Latest,
                };
                let st = self.store.clone();
                let page = blocking(move || st.history(&user, &space, dir, limit)).await.map_err(Code::failure)?;
                Ok(Answer { body: json(&SpaceHistoryRes { messages: page.messages, more: page.more })?, deliver: None })
            }
            Op::SpaceList => {
                let st = self.store.clone();
                let spaces = blocking(move || st.list(&user)).await.map_err(Code::failure)?;
                Ok(Answer { body: json(&SpaceListRes { spaces })?, deliver: None })
            }
            Op::SpaceView => {
                let space: String = oxsig::code::need(&body, "space_id")?;
                let st = self.store.clone();
                let read = blocking(move || st.view(&user, &space)).await.map_err(Code::failure)?;
                Ok(Answer { body: json(&SpaceViewRes { read })?, deliver: None })
            }
            _ => Err(Code::UnknownOp.failure()),
        }
    }
}

pub fn is_chat(op: Op) -> bool {
    matches!(op, Op::Message | Op::SpaceHistory | Op::SpaceList | Op::SpaceView)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, Code> + Send + 'static) -> Result<T, Code> {
    tokio::task::spawn_blocking(f).await.unwrap_or(Err(Code::InternalError))
}

pub fn valid_msg_id(id: &str, user: &str) -> bool {
    let Some((who, rest)) = id.split_once('/') else { return false };
    who == user && rest.len() == 36 && uuid::Uuid::parse_str(rest).is_ok_and(|u| u.get_version_num() == 4)
}

fn json<T: Serialize>(v: &T) -> Result<Value, Failure> {
    serde_json::to_value(v).map_err(|_| Code::InternalError.failure())
}

pub async fn fanout_others(bus: &Bus, d: &Deliver) -> Vec<String> {
    let payload = serde_json::to_vec(d).unwrap_or_default();
    let others: Vec<String> = bus.live_nodes().into_iter().map(|(n, _)| n).filter(|n| n != &bus.node).collect();
    let asks = others.iter().map(|n| bus.query(n, FANOUT_WHAT, payload.clone()));
    let mut failed = Vec::new();
    for (node, r) in others.iter().zip(futures_util::future::join_all(asks).await) {
        if r.is_err() {
            failed.push(node.clone());
        }
    }
    failed
}
