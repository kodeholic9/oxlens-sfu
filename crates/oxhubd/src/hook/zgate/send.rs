// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:RULE-HOOK-FACTS-SUB · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-REQUEST · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! zenoh 입구의 보내기(수송만) — 연결의 키 갈래(채팅 · 사실)마다 보낼 줄(창 · 줄 · 밀림 · 잇단 시간 넘김 — 갈래마다 따로)에 넣는다(성공 · 연결 없음 · 그 갈래가 안 열림 · 밀림 · 줄 넘침) · 창에 빈 칸이 있으면 줄 앞을 질의로 꺼낸다 · 끝(ACK · ACK 아님 · 시간 넘김)을 줄에 알리고 채팅 갈래면 채팅 몫(`crate::chat::ext::ChatExt`)을 직접 부른다 · 사실 갈래는 기다리는 쪽(사실 운반)에 끝을 돌려준다 · node 사이 넘기기 몸(`q/node/{node}/ext` — 갈래 `lane`)과 그 답: 채팅은 줄에 넣은 것만 답하고 · ★사실은 그 묶음의 끝까지 기다려 답한다(방을 맡은 hub 가 시도 수 · 버림 표지를 센다).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::envelope::Reply;
use oxsig::Code;
use oxsig::msg::ChatMessage;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::judge::Lane;
use super::{Cmd, Gate, QueryError};
use crate::chat::ext::plan::{After, Item, Knobs, Offer, Outcome};
use crate::hook::facts::Fact;

pub const WHAT: &str = "ext";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateKnobs {
    pub window: usize,
    pub reply_timeout: Duration,
    pub close_at: u32,
    pub facts_cap: usize,
}

impl GateKnobs {
    pub fn of(hook: &common::policy::Hook, facts_bundle_max_bytes: usize) -> GateKnobs {
        GateKnobs {
            window: usize::from(hook.ext_window),
            reply_timeout: Duration::from_millis(u64::from(hook.ext_reply_timeout_ms)),
            close_at: u32::from(hook.ext_timeout_close),
            facts_cap: facts_bundle_max_bytes.saturating_mul(4),
        }
    }

    pub fn facts_outbox(&self) -> Knobs {
        Knobs { window: self.window, cap: self.facts_cap, live_max: 0, close_at: self.close_at }
    }
}

pub struct GateDeps {
    pub accounts: super::judge::Accounts,
    pub bus: Arc<crate::bus::Bus>,
    pub chat: Option<Arc<crate::chat::ext::ChatExt>>,
    pub knobs: GateKnobs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "lane", rename_all = "snake_case", try_from = "ExtWire")]
pub enum ExtDeliver {
    Chat { key: String, cid: String, messages: Vec<ChatMessage> },
    Facts { key: String, cid: String, bundle: FactsBundle },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtLane {
    #[default]
    Chat,
    Facts,
}

#[derive(Deserialize)]
pub struct ExtWire {
    #[serde(default)]
    lane: ExtLane,
    key: String,
    cid: String,
    #[serde(default)]
    messages: Option<Vec<ChatMessage>>,
    #[serde(default)]
    bundle: Option<FactsBundle>,
}

impl TryFrom<ExtWire> for ExtDeliver {
    type Error = &'static str;

    fn try_from(w: ExtWire) -> Result<ExtDeliver, &'static str> {
        match w.lane {
            ExtLane::Chat => Ok(ExtDeliver::Chat { key: w.key, cid: w.cid, messages: w.messages.ok_or("chat lane without messages")? }),
            ExtLane::Facts => Ok(ExtDeliver::Facts { key: w.key, cid: w.cid, bundle: w.bundle.ok_or("facts lane without bundle")? }),
        }
    }
}

impl ExtDeliver {
    pub fn key(&self) -> &str {
        match self {
            ExtDeliver::Chat { key, .. } | ExtDeliver::Facts { key, .. } => key,
        }
    }

    pub fn cid(&self) -> &str {
        match self {
            ExtDeliver::Chat { cid, .. } | ExtDeliver::Facts { cid, .. } => cid,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FactsBundle {
    pub facts: Vec<Fact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Ack,
    Fail,
    Timeout,
    Closed,
}

impl End {
    pub fn of(o: Outcome) -> End {
        match o {
            Outcome::Ack => End::Ack,
            Outcome::Fail => End::Fail,
            Outcome::Timeout => End::Timeout,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    pub end: End,
}

pub type Waiter = oneshot::Sender<Outcome>;

pub struct Flight {
    pub lane: Lane,
    pub ids: Vec<String>,
    pub deadline: Instant,
    pub waiter: Option<Waiter>,
}

pub fn refusal_code(o: Offer) -> Code {
    match o {
        Offer::RefusedBacklog | Offer::RefusedFull => Code::FlowOverflow,
        Offer::Merged | Offer::Queued => Code::InternalError,
    }
}

impl Gate {
    fn offer(&self, cid: &str, lane: Lane, item: Item<serde_json::Value, Waiter>) -> Result<Offer, QueryError> {
        let mut g = self.conns.lock().map_err(|_| QueryError::NoConn)?;
        let h = g.get_mut(cid).ok_or(QueryError::NoConn)?;
        let q = h.lanes.get_mut(&lane).filter(|q| q.open).ok_or(QueryError::NotOpen)?;
        let o = q.outbox.offer(item);
        if o.accepted() {
            let _ = h.cmd.send(Cmd::Kick);
        } else {
            self.drops.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        Ok(o)
    }

    pub fn deliver(&self, cid: &str, live: bool, msgs: Vec<ChatMessage>, waiter: Option<Waiter>) -> Result<Offer, QueryError> {
        let ids: Vec<String> = msgs.iter().map(|m| m.msg_id.clone()).collect();
        let bytes: usize = msgs.iter().map(crate::chat::ext::json_len).sum();
        let body: Vec<serde_json::Value> = msgs.iter().filter_map(|m| serde_json::to_value(m).ok()).collect();
        self.offer(cid, Lane::Chat, Item { live, ids, bytes, msgs: body, waiter })
    }

    pub fn deliver_facts(&self, cid: &str, bundle: &FactsBundle, waiter: Option<Waiter>) -> Result<Offer, QueryError> {
        let ids: Vec<String> = bundle.facts.iter().map(|f| f.fact_id.clone()).collect();
        let body = serde_json::to_value(bundle).unwrap_or_default();
        let bytes = serde_json::to_vec(&body).map(|v| v.len()).unwrap_or(0);
        self.offer(cid, Lane::Facts, Item { live: true, ids, bytes, msgs: vec![body], waiter })
    }

    pub fn lane_open(&self, cid: &str, lane: Lane) -> bool {
        self.conns.lock().ok().and_then(|g| g.get(cid).and_then(|h| h.lanes.get(&lane).map(|q| q.open))).unwrap_or(false)
    }

    pub fn take_released(&self, cid: &str, lane: Lane) -> bool {
        self.conns
            .lock()
            .ok()
            .and_then(|mut g| g.get_mut(cid).and_then(|h| h.lanes.get_mut(&lane).map(|q| std::mem::replace(&mut q.released, false))))
            .unwrap_or(false)
    }

    pub(super) fn pull(&self, cid: &str) -> Vec<(Lane, Item<serde_json::Value, Waiter>)> {
        let mut out = Vec::new();
        if let Ok(mut g) = self.conns.lock()
            && let Some(h) = g.get_mut(cid)
        {
            for (lane, q) in h.lanes.iter_mut() {
                while let Some(it) = q.outbox.take() {
                    out.push((*lane, it));
                }
            }
        }
        out
    }

    pub(super) fn finish(&self, cid: &str, key: &str, f: Flight, o: Outcome) -> After {
        let after = match self.conns.lock() {
            Ok(mut g) => match g.get_mut(cid).and_then(|h| h.lanes.get_mut(&f.lane)) {
                Some(q) => {
                    let a = q.outbox.done(o);
                    if a == After::Released {
                        q.released = true;
                    }
                    a
                }
                None => After::None,
            },
            Err(_) => After::None,
        };
        if o == Outcome::Timeout {
            self.drops.reply_timeout.fetch_add(1, Ordering::Relaxed);
        }
        if f.lane == Lane::Chat
            && let Some(c) = &self.chat
        {
            c.done(&self.drops, key, f.ids, o);
        }
        if let Some(w) = f.waiter {
            let _ = w.send(o);
        }
        after
    }
}

pub fn facts_query_timeout(reply_timeout: Duration) -> Duration {
    reply_timeout + crate::bus::NODE_CALL_TIMEOUT
}

pub async fn serve_ext(gate: Option<&Arc<Gate>>, payload: &[u8]) -> Vec<u8> {
    let Some(gate) = gate else { return Reply::fail(Code::SessionNotFound).encode() };
    let Ok(d) = serde_json::from_slice::<ExtDeliver>(payload) else { return Reply::fail(Code::InvalidPayload).encode() };
    match d {
        ExtDeliver::Chat { cid, messages, .. } => match gate.deliver(&cid, true, messages, None) {
            Ok(o) if o.accepted() => Reply::ok(serde_json::json!({})).encode(),
            Ok(o) => Reply::fail(refusal_code(o)).encode(),
            Err(_) => Reply::fail(Code::SessionNotFound).encode(),
        },
        ExtDeliver::Facts { cid, bundle, .. } => {
            let (tx, rx) = oneshot::channel();
            match gate.deliver_facts(&cid, &bundle, Some(tx)) {
                Ok(o) if o.accepted() => {
                    let end = rx.await.map(End::of).unwrap_or(End::Closed);
                    Reply::ok(serde_json::to_value(Ended { end }).unwrap_or_default()).encode()
                }
                Ok(o) => Reply::fail(refusal_code(o)).encode(),
                Err(_) => Reply::fail(Code::SessionNotFound).encode(),
            }
        }
    }
}
