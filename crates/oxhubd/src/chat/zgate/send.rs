// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CHAT-UNDELIVERED · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! zenoh 입구의 보내기 — 넘기기를 받아 연결의 보낼 줄에 넣는다(성공 · 연결 없음 · 밀림 · 줄 넘침) · 창에 빈 칸이 있으면 줄 앞을 질의로 꺼낸다 · 끝(ACK · ACK 아님 · 시간 넘김)을 처리한다(ACK 는 3rd ACK 줄로 · 아니면 시도 수 +1 · 상한에서 지운 것을 센다 · 시간 넘김은 밀림과 잇단 수) · 계정마다 drain 주인(목록 첫 연결)이 제 것이면 drain 한다(막 주인이 됨 · 밀림이 풀림 · 훑기) — 한 번에 하나 · 묶음 하나씩 끝을 기다린다.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use oxsig::Code;
use oxsig::msg::ChatMessage;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::plan::{After, Due, Item, Knobs, Offer, Outcome, Row, drain_due, drain_pick};
use super::{Cmd, Gate, QueryError};
use crate::chat::acks::{AckLanes, Push};
use crate::chat::store::{Cursor, Lane, Store};

pub const DRAIN_TICK: Duration = Duration::from_secs(1);
pub const WHAT: &str = "ext";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateKnobs {
    pub window: usize,
    pub reply_timeout: Duration,
    pub close_at: u32,
    pub max_tries: u32,
    pub sweep: Duration,
    pub live_max: usize,
    pub cap: usize,
    pub bundle_msgs: usize,
    pub bundle_bytes: usize,
}

impl GateKnobs {
    pub fn of(hook: &common::policy::Hook, live_max: usize, bundle_msgs: usize, bundle_bytes: usize) -> GateKnobs {
        GateKnobs {
            window: usize::from(hook.ext_window),
            reply_timeout: Duration::from_millis(u64::from(hook.ext_reply_timeout_ms)),
            close_at: u32::from(hook.ext_timeout_close),
            max_tries: u32::from(hook.ext_max_tries),
            sweep: Duration::from_secs(u64::from(hook.ext_sweep_secs)),
            live_max,
            cap: bundle_bytes.saturating_mul(4),
            bundle_msgs,
            bundle_bytes,
        }
    }

    pub fn outbox(&self) -> Knobs {
        Knobs { window: self.window, cap: self.cap, live_max: self.live_max, close_at: self.close_at }
    }
}

pub struct GateDeps {
    pub accounts: super::judge::Accounts,
    pub bus: Arc<crate::bus::Bus>,
    pub store: Arc<Store>,
    pub acks: Arc<AckLanes>,
    pub keys: Vec<String>,
    pub knobs: GateKnobs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtDeliver {
    pub key: String,
    pub cid: String,
    pub messages: Vec<ChatMessage>,
}

pub type Waiter = oneshot::Sender<Outcome>;

pub struct Flight {
    pub ids: Vec<String>,
    pub deadline: Instant,
    pub waiter: Option<Waiter>,
}

#[derive(Debug, Default)]
pub struct DrainState {
    owner: Option<String>,
    last_sweep: u64,
    running: bool,
}

fn json_len(m: &ChatMessage) -> usize {
    serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0)
}

pub fn refusal_code(o: Offer) -> Code {
    match o {
        Offer::RefusedBacklog | Offer::RefusedFull => Code::FlowOverflow,
        Offer::Merged | Offer::Queued => Code::InternalError,
    }
}

impl Gate {
    pub fn deliver(&self, cid: &str, live: bool, msgs: Vec<ChatMessage>, waiter: Option<Waiter>) -> Result<Offer, QueryError> {
        let ids: Vec<String> = msgs.iter().map(|m| m.msg_id.clone()).collect();
        let bytes: usize = msgs.iter().map(json_len).sum();
        let body: Vec<serde_json::Value> = msgs.iter().filter_map(|m| serde_json::to_value(m).ok()).collect();
        let mut g = self.conns.lock().map_err(|_| QueryError::NoConn)?;
        let h = g.get_mut(cid).ok_or(QueryError::NoConn)?;
        if !h.open {
            return Err(QueryError::NotOpen);
        }
        let o = h.outbox.offer(Item { live, ids, bytes, msgs: body, waiter });
        if o.accepted() {
            let _ = h.cmd.send(Cmd::Kick);
        } else {
            self.drops.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        Ok(o)
    }

    pub(super) fn pull(&self, cid: &str) -> Vec<Item<serde_json::Value, Waiter>> {
        let mut out = Vec::new();
        if let Ok(mut g) = self.conns.lock()
            && let Some(h) = g.get_mut(cid)
        {
            while let Some(it) = h.outbox.take() {
                out.push(it);
            }
        }
        out
    }

    pub(super) fn finish(&self, cid: &str, key: &str, f: Flight, o: Outcome) -> After {
        let after = match self.conns.lock() {
            Ok(mut g) => match g.get_mut(cid) {
                Some(h) => {
                    let a = h.outbox.done(o);
                    if a == After::Released {
                        h.released = true;
                    }
                    a
                }
                None => After::None,
            },
            Err(_) => After::None,
        };
        match o {
            Outcome::Ack => {
                if self.acks.push(Lane::Ext, (key.to_string(), f.ids)) != Push::Queued {
                    self.sparse("ext ack queue full", key);
                }
            }
            Outcome::Fail | Outcome::Timeout => {
                if o == Outcome::Timeout {
                    self.drops.reply_timeout.fetch_add(1, Ordering::Relaxed);
                }
                let (st, k, ids, max) = (self.store.clone(), key.to_string(), f.ids, self.knobs.max_tries);
                let given_up = self.given_up_sink();
                tokio::task::spawn_blocking(move || match st.ext_failed(&k, &ids, max) {
                    Ok(gone) => given_up.record(&k, &gone, max),
                    Err(c) => tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %k, code = c.num(), "ext tries not counted — store failed"),
                });
            }
        }
        if let Some(w) = f.waiter {
            let _ = w.send(o);
        }
        after
    }

    fn given_up_sink(&self) -> GivenUp {
        GivenUp(self.drops.clone())
    }

    fn owner_here(&self, key: &str) -> Option<String> {
        let list = self.bus.ext_list(key);
        let (node, cid) = list.first()?;
        if *node != self.bus.node {
            return None;
        }
        let g = self.conns.lock().ok()?;
        g.get(cid).filter(|h| h.open).map(|_| cid.clone())
    }

    fn take_released(&self, cid: &str) -> bool {
        self.conns.lock().ok().and_then(|mut g| g.get_mut(cid).map(|h| std::mem::replace(&mut h.released, false))).unwrap_or(false)
    }

    pub async fn drains(self: Arc<Self>) {
        let mut tick = tokio::time::interval(DRAIN_TICK);
        let sweep_ms = u64::try_from(self.knobs.sweep.as_millis()).unwrap_or(u64::MAX);
        loop {
            tick.tick().await;
            let now = common::now_ms();
            for key in self.keys_sub.clone() {
                let mine = self.owner_here(&key);
                let released = mine.as_deref().is_some_and(|c| self.take_released(c));
                let start = {
                    let Ok(mut g) = self.drain_st.lock() else { continue };
                    let st = g.entry(key.clone()).or_default();
                    let was = st.owner.is_some() && st.owner == mine;
                    let due = drain_due(mine.is_some(), was, released, now, st.last_sweep, sweep_ms);
                    st.owner = mine.clone();
                    match (due, mine, st.running) {
                        (Due::None, _, _) | (_, None, _) | (_, _, true) => None,
                        (d, Some(cid), false) => {
                            st.running = true;
                            st.last_sweep = now;
                            Some((cid, d))
                        }
                    }
                };
                if let Some((cid, due)) = start {
                    let me = self.clone();
                    tokio::spawn(async move { me.drain(key, cid, due).await });
                }
            }
        }
    }

    async fn drain(self: Arc<Self>, key: String, cid: String, due: Due) {
        let before = match due {
            Due::Older(b) => Some(b),
            Due::All | Due::None => None,
        };
        let mut after = Cursor::default();
        let mut sent = 0usize;
        loop {
            let (st, k, a, lim) = (self.store.clone(), key.clone(), after.clone(), self.knobs.bundle_msgs);
            let rows = match tokio::task::spawn_blocking(move || st.ext_page(&k, &a, before, lim)).await {
                Ok(Ok(r)) => r,
                _ => {
                    tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, cid = %cid, "ext drain stopped — store read failed");
                    break;
                }
            };
            let plan: Vec<Row> = rows.iter().map(|(m, tries)| Row { id: m.msg_id.clone(), bytes: json_len(m), tries: *tries }).collect();
            let pick: HashSet<String> = drain_pick(&plan, self.knobs.bundle_msgs, self.knobs.bundle_bytes).into_iter().collect();
            let msgs: Vec<ChatMessage> = rows.into_iter().map(|(m, _)| m).filter(|m| pick.contains(&m.msg_id)).collect();
            let Some(last) = msgs.last() else { break };
            after = Cursor { t: last.t, msg_id: last.msg_id.clone() };
            let n = msgs.len();
            let (tx, rx) = oneshot::channel();
            match self.deliver(&cid, false, msgs, Some(tx)) {
                Ok(o) if o.accepted() => {}
                _ => break,
            }
            if rx.await.is_err() {
                break;
            }
            sent += n;
        }
        if sent > 0 {
            tracing::info!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, cid = %cid, sent = sent, mode = ?due, "ext drain done");
        }
        if let Ok(mut g) = self.drain_st.lock()
            && let Some(st) = g.get_mut(&key)
        {
            st.running = false;
        }
    }
}

#[derive(Clone)]
pub struct GivenUp(pub Arc<super::Drops>);

impl GivenUp {
    fn record(&self, key: &str, gone: &[String], max: u32) {
        self.0.given_up.fetch_add(gone.len() as u64, Ordering::Relaxed);
        for m in gone {
            tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, msg_id = %m, "in.max_tries" = max, "ext pending given up — tries reached the limit");
        }
    }
}

pub type DrainStates = HashMap<String, DrainState>;
