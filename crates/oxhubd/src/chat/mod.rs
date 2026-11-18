// author: kodeholic (powered by Claude)
// spec: v1.6 · 연:RULE-CHAT-KIND · 연:RULE-CHAT-EXT · 정:RULE-CHAT-GATEWAY-MODULE ·정:IMPL-CHAT-STORE · 정:IMPL-CHAT-FANOUT · 정:IMPL-CHAT-FILTER · 정:IMPL-CHAT-READ · 정:IMPL-CHAT-UNDELIVERED · 정:IMPL-CHAT-CATCHUP · 정:IMPL-CHAT-STORE-FAULTS · 정:IMPL-CHAT-TALK · 연:RULE-CHAT-TALK · 연:CHAT.op.MESSAGE · 연:CHAT.op.SPACE_HISTORY · 연:CHAT.op.SPACE_LIST · 연:CHAT.op.SPACE_VIEW · 연:CHAT.op.CATCHUP · 연:RULE-CHAT-MSG-ID · 연:RULE-CHAT-CONTENT · 연:RULE-CHAT-RECV · 연:RULE-CHAT-DISABLED · 연:RULE-CHAT-LIVE-SOCKET-LOSS · 연:RULE-CHAT-CATCHUP · model: claude-opus-5-5
//! 채팅 — 게이트웨이 안 모듈 · op 다섯 · 발언 기록(node 안 요청 `talk` — 같은 저장 길목 · 보낸 세션 없음) · 저장이 낸 `{멤버 판, 보낸 세션, 메시지}` 를 다른 node 에 뿌린다(실패 수를 돌려주고 다시 안 보낸다) · 공간마다 멤버 캐시로 `명단(t_M)` 을 낸다 · 받은 ACK 는 채널에 넣기만 하고 지우는 스레드 하나가 모아 한 트랜잭션에 지운다 · 보관 기간을 넘긴 미전송 기록을 지운다. 세션은 모른다 — 세션이 이 모듈을 부른다.

pub mod store;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxsig::msg::{CatchupReq, ChatKind, ChatMessage, MessageRes, SpaceHistoryRes, SpaceListRes, SpaceViewRes};
use oxsig::{Code, Failure, Op};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bus::Bus;
use common::envelope::Reply;
use store::{Ack, CONTENT_MAX, Cursor, Dir, HISTORY_DEFAULT, HISTORY_MAX, MSG_ID_MAX, Outgoing, Roster, Sent, Store};

pub const FANOUT_WHAT: &str = "chat";
pub const BODY_MAX: usize = 131_072;
pub const PURGE_EVERY: Duration = Duration::from_secs(600);
const DAY_MS: u64 = 86_400_000;
pub const ACK_QUEUE: usize = 65_536;
pub const ACK_BATCH: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub undelivered_ttl_days: u64,
    pub bundle_msgs: usize,
    pub bundle_bytes: usize,
    pub live_bundle_bytes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { undelivered_ttl_days: 7, bundle_msgs: 100, bundle_bytes: 524_288, live_bundle_bytes: 65_536 }
    }
}

pub struct Chat {
    pub store: Arc<Store>,
    pub limits: Limits,
    cache: Mutex<HashMap<String, Roster>>,
    acks: SyncSender<Ack>,
    ack_dropped: AtomicU64,
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
    pub catchup: Option<CatchupReq>,
}

impl Chat {
    pub fn new(store: Store, limits: Limits) -> Chat {
        let store = Arc::new(store);
        let (acks, rx) = std::sync::mpsc::sync_channel(ACK_QUEUE);
        let st = store.clone();
        if let Err(e) = std::thread::Builder::new().name("chat-ack".into()).spawn(move || ack_writer(&st, &rx)) {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", "ack writer did not start: {e} — records stay, catchup sends them again");
        }
        Chat { store, limits, cache: Mutex::new(HashMap::new()), acks, ack_dropped: AtomicU64::new(0) }
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

    pub async fn recipients(&self, d: &Deliver) -> Result<Vec<String>, Code> {
        self.roster_at_least(&d.message.space_id, d.ver).await.map(|r| r.at(d.message.t))
    }

    pub async fn handle(&self, user: &str, sid: &str, op: Op, body: Value, raw: usize) -> Result<Answer, Failure> {
        let user = user.to_string();
        let plain = |body: Value| Answer { body, deliver: None, catchup: None };
        match op {
            Op::Message => {
                if raw > BODY_MAX {
                    return Err(Code::InvalidPayload.failure().with_message("body"));
                }
                let space: String = oxsig::code::need(&body, "space_id")?;
                let msg_id: String = oxsig::code::need(&body, "msg_id")?;
                if !valid_msg_id(&msg_id) {
                    return Err(Code::InvalidPayload.failure().with_message("msg_id"));
                }
                let out = outgoing(&body)?;
                let st = self.store.clone();
                let sent = blocking(move || st.post(&user, &space, &msg_id, &out, None)).await.map_err(Code::failure)?;
                let (msg, deliver) = match sent {
                    Sent::Fresh { msg, ver } => (msg.clone(), Some(Deliver { ver, sid: sid.to_string(), message: msg })),
                    Sent::Again { msg } => (msg, None),
                };
                Ok(Answer { body: json(&MessageRes { t: msg.t })?, deliver, catchup: None })
            }
            Op::SpaceHistory => {
                let space: String = oxsig::code::need(&body, "space_id")?;
                let after: Option<u64> = oxsig::code::field(&body, "after")?;
                let before: Option<u64> = oxsig::code::field(&body, "before")?;
                let limit: Option<u32> = oxsig::code::field(&body, "limit")?;
                let limit = limit.unwrap_or(HISTORY_DEFAULT);
                if limit == 0 || limit > HISTORY_MAX {
                    return Err(Code::InvalidPayload.failure().with_message("limit"));
                }
                let dir = match (after, before) {
                    (Some(_), Some(_)) => return Err(Code::FieldConflict.failure().with_message("after xor before")),
                    (Some(a), None) => Dir::After(a),
                    (None, Some(b)) => Dir::Before(b),
                    (None, None) => Dir::Latest,
                };
                let st = self.store.clone();
                let budget = self.limits.bundle_bytes;
                let page = blocking(move || st.history_within(&user, &space, dir, limit, budget)).await.map_err(Code::failure)?;
                Ok(plain(json(&SpaceHistoryRes { messages: page.messages, relations: page.relations, more: page.more })?))
            }
            Op::SpaceList => {
                let st = self.store.clone();
                let spaces = blocking(move || st.list(&user)).await.map_err(Code::failure)?;
                Ok(plain(json(&SpaceListRes { spaces })?))
            }
            Op::SpaceView => {
                let space: String = oxsig::code::need(&body, "space_id")?;
                let st = self.store.clone();
                let read = blocking(move || st.view(&user, &space)).await.map_err(Code::failure)?;
                Ok(plain(json(&SpaceViewRes { read })?))
            }
            Op::Catchup => {
                let space_id: Option<String> = oxsig::code::field(&body, "space_id")?;
                let t: Option<u64> = oxsig::code::field(&body, "t")?;
                if let Some(sp) = space_id.clone() {
                    let st = self.store.clone();
                    blocking(move || st.catchup_check(&user, &sp)).await.map_err(Code::failure)?;
                }
                Ok(Answer { body: Value::Null, deliver: None, catchup: Some(CatchupReq { space_id, t }) })
            }
            _ => Err(Code::UnknownOp.failure()),
        }
    }

    pub async fn talk(&self, r: common::talk::Report) -> Result<(u64, Option<Deliver>), Code> {
        let st = self.store.clone();
        let sent = blocking(move || st.talk(&r, None)).await?;
        Ok(match sent {
            Sent::Fresh { msg, ver } => (msg.t, Some(Deliver { ver, sid: String::new(), message: msg })),
            Sent::Again { msg } => (msg.t, None),
        })
    }

    pub async fn rec_seen(&self, user: &str, space: &str, talk_id: &str) -> Result<(), Code> {
        let (st, user, space, talk_id) = (self.store.clone(), user.to_string(), space.to_string(), talk_id.to_string());
        blocking(move || st.rec_seen(&user, &space, &talk_id)).await
    }

    pub async fn catchup_page(&self, user: &str, req: &CatchupReq, after: &Cursor) -> Result<Vec<ChatMessage>, Code> {
        let st = self.store.clone();
        let (user, space, since, after, lim) = (user.to_string(), req.space_id.clone(), req.t, after.clone(), self.limits);
        blocking(move || st.catchup(&user, space.as_deref(), since, &after, lim.bundle_msgs, lim.bundle_bytes)).await
    }

    pub fn acked(&self, user: &str, msg_ids: Vec<String>) {
        if msg_ids.is_empty() {
            return;
        }
        let why = match self.acks.try_send((user.to_string(), msg_ids)) {
            Ok(()) => return,
            Err(TrySendError::Full(_)) => "ack queue full",
            Err(TrySendError::Disconnected(_)) => "ack writer gone",
        };
        let n = self.ack_dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_power_of_two() {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", user = %user, dropped = n, "{why} — records stay, catchup sends them again");
        }
    }

    pub fn purge_once(&self, now: u64) -> Result<usize, Code> {
        let before = now.saturating_sub(self.limits.undelivered_ttl_days.saturating_mul(DAY_MS));
        self.store.purge(before).inspect_err(|c| {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", code = c.num(), "undelivered purge failed — retried next cycle");
        })
    }

    pub async fn purge_loop(self: Arc<Chat>) {
        let mut tick = tokio::time::interval(PURGE_EVERY);
        loop {
            tick.tick().await;
            let me = self.clone();
            let _ = blocking(move || me.purge_once(common::now_ms())).await;
        }
    }
}

fn ack_writer(store: &Store, rx: &Receiver<Ack>) {
    while let Ok(first) = rx.recv() {
        if let Err(c) = store.ack_drain(first, rx, ACK_BATCH) {
            tracing::warn!(kind = "diag", spec = "SV-IMPL-CHAT-STORE-FAULTS", code = c.num(), "ack batch not deleted — records stay, catchup sends them again");
        }
    }
}

pub fn is_chat(op: Op) -> bool {
    matches!(op, Op::Message | Op::SpaceHistory | Op::SpaceList | Op::SpaceView | Op::Catchup)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, Code> + Send + 'static) -> Result<T, Code> {
    tokio::task::spawn_blocking(f).await.unwrap_or(Err(Code::InternalError))
}

pub fn valid_msg_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MSG_ID_MAX
}

fn bad(what: &str) -> Failure {
    Code::InvalidPayload.failure().with_message(what)
}

pub fn outgoing(body: &Value) -> Result<Outgoing, Failure> {
    let kind = match body.get("kind") {
        None => None,
        Some(Value::String(s)) => Some(ChatKind::parse(s).filter(|k| k.is_relation()).ok_or_else(|| bad("kind"))?),
        Some(_) => return Err(bad("kind")),
    };
    let r#ref = match body.get("ref") {
        None => None,
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => return Err(bad("ref")),
    };
    let content = match body.get("content") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(bad("content")),
    };
    let keys = match body.get("keys") {
        None => None,
        Some(v) => Some(serde_json::from_value::<Vec<String>>(v.clone()).map_err(|_| bad("keys"))?),
    };
    let ext = match body.get("ext") {
        None => None,
        Some(v) if v.is_object() => Some(v.clone()),
        Some(_) => return Err(bad("ext")),
    };
    if body.get("target").is_some() {
        return Err(bad("target"));
    }
    if body.get("talk").is_some() {
        return Err(bad("talk"));
    }
    if content.as_ref().is_some_and(|c| c.len() > CONTENT_MAX) {
        return Err(bad("content"));
    }
    let empty = content.as_deref().is_none_or(str::is_empty);
    let (need, extra) = match kind {
        None => (content.is_some(), keys.is_some()),
        Some(ChatKind::Edit) => (r#ref.is_some() && content.is_some(), keys.is_some()),
        Some(ChatKind::React) => (r#ref.is_some() && keys.is_some(), !empty || ext.is_some()),
        Some(ChatKind::Delete) => (r#ref.is_some(), keys.is_some() || !empty || ext.is_some()),
        Some(_) => (true, true),
    };
    if extra {
        return Err(bad("kind"));
    }
    if !need {
        return Err(Code::MissingField.failure().with_message("kind"));
    }
    Ok(Outgoing { kind, r#ref, content: content.unwrap_or_default(), keys, ext })
}

fn json<T: Serialize>(v: &T) -> Result<Value, Failure> {
    serde_json::to_value(v).map_err(|_| Code::InternalError.failure())
}

pub async fn fanout_others(bus: &Bus, d: &Deliver, only: Option<&[String]>) -> Vec<String> {
    let payload = serde_json::to_vec(d).unwrap_or_default();
    let known: Vec<String> = bus.known_nodes().into_iter().map(|(n, _)| n).collect();
    let mut failed: Vec<String> = only.unwrap_or_default().iter().filter(|n| **n != bus.node && !known.contains(n)).cloned().collect();
    let mut live = Vec::new();
    for (n, up) in bus.known_nodes() {
        if n == bus.node || only.is_some_and(|o| !o.contains(&n)) {
            continue;
        }
        if up {
            live.push(n);
        } else {
            failed.push(n);
        }
    }
    let asks = live.iter().map(|n| bus.query(n, FANOUT_WHAT, payload.clone()));
    for (node, r) in live.iter().zip(futures_util::future::join_all(asks).await) {
        let ok = r.ok().and_then(|b| Reply::decode(&b).ok()).is_some_and(|x| matches!(x, Reply::Ok { .. }));
        if !ok {
            failed.push(node.clone());
        }
    }
    failed
}
