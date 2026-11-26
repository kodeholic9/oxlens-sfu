// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CHAT-UNDELIVERED · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 채팅 3rd 구독의 채팅 몫 — 주인은 채팅이다: 넘기기 판단(`plan` — 원천 `chat/behavior/extsend.ts` 의 거울 · 벡터 `chat.json`) · 3rd 몫 펜딩 · 끝처리(ACK 는 3rd ACK 줄로 · ACK 아닌 끝은 시도 수 +1 · 상한에서 지운 것을 센다) · 계정마다 drain 주인(가장 오래 붙어 있는 연결)이 제 것이면 drain 한다(막 주인이 됨 · 밀림이 풀림 · 훑기 — 한 번에 하나 · 묶음 하나씩 끝을 기다린다). 입구 · 연결 · 보내기 수송은 대외 연동(`crate::hook::zgate`)이 맡고 이 모듈은 그것을 직접 부른다.

pub mod plan;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxsig::msg::ChatMessage;
use tokio::sync::oneshot;

use crate::bus::Bus;
use crate::chat::acks::{AckLanes, Push};
use crate::chat::store::{Cursor, Lane, Store};
use crate::hook::zgate::judge::Lane as GateLane;
use crate::hook::zgate::{Drops, Gate};
use plan::{Due, Knobs, Outcome, Row, drain_due, drain_owner, drain_pick};

pub const DRAIN_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatKnobs {
    pub live_max: usize,
    pub cap: usize,
    pub bundle_msgs: usize,
    pub bundle_bytes: usize,
    pub sweep: Duration,
    pub max_tries: u32,
}

impl ChatKnobs {
    pub fn of(hook: &common::policy::Hook, live_max: usize, bundle_msgs: usize, bundle_bytes: usize) -> ChatKnobs {
        ChatKnobs {
            live_max,
            cap: bundle_bytes.saturating_mul(4),
            bundle_msgs,
            bundle_bytes,
            sweep: Duration::from_secs(u64::from(hook.ext_sweep_secs)),
            max_tries: u32::from(hook.ext_max_tries),
        }
    }

    pub fn outbox(&self, window: usize, close_at: u32) -> Knobs {
        Knobs { window, cap: self.cap, live_max: self.live_max, close_at }
    }
}

#[derive(Debug, Default)]
pub struct DrainState {
    owner: Option<String>,
    last_sweep: u64,
    running: bool,
}

pub struct ChatExt {
    pub store: Arc<Store>,
    pub acks: Arc<AckLanes>,
    pub keys: Vec<String>,
    pub knobs: ChatKnobs,
    drain_st: Mutex<HashMap<String, DrainState>>,
}

pub fn json_len(m: &ChatMessage) -> usize {
    serde_json::to_vec(m).map(|v| v.len()).unwrap_or(0)
}

impl ChatExt {
    pub fn new(store: Arc<Store>, acks: Arc<AckLanes>, keys: Vec<String>, knobs: ChatKnobs) -> ChatExt {
        ChatExt { store, acks, keys, knobs, drain_st: Mutex::new(HashMap::new()) }
    }

    pub fn done(&self, drops: &Arc<Drops>, key: &str, ids: Vec<String>, o: Outcome) {
        match o {
            Outcome::Ack => {
                if self.acks.push(Lane::Ext, (key.to_string(), ids)) != Push::Queued {
                    tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %key, "ext ack queue full");
                }
            }
            Outcome::Fail | Outcome::Timeout => {
                let (st, k, max, drops) = (self.store.clone(), key.to_string(), self.knobs.max_tries, drops.clone());
                tokio::task::spawn_blocking(move || match st.ext_failed(&k, &ids, max) {
                    Ok(gone) => {
                        drops.given_up.fetch_add(gone.len() as u64, Ordering::Relaxed);
                        for m in &gone {
                            tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %k, msg_id = %m, "in.max_tries" = max, "ext pending given up — tries reached the limit");
                        }
                    }
                    Err(c) => tracing::warn!(kind = "diag", spec = "SV-RULE-CHAT-EXT-SUB", key = %k, code = c.num(), "ext tries not counted — store failed"),
                });
            }
        }
    }

    fn owner_here(bus: &Bus, gate: &Gate, key: &str) -> Option<String> {
        let list = bus.ext_list(key);
        let (node, cid) = list.get(drain_owner(&list)?)?;
        if *node != bus.node {
            return None;
        }
        gate.lane_open(cid, GateLane::Chat).then(|| cid.clone())
    }

    pub async fn drains(self: Arc<Self>, gate: Arc<Gate>, bus: Arc<Bus>) {
        let mut tick = tokio::time::interval(DRAIN_TICK);
        let sweep_ms = u64::try_from(self.knobs.sweep.as_millis()).unwrap_or(u64::MAX);
        loop {
            tick.tick().await;
            let now = common::now_ms();
            for key in self.keys.clone() {
                let mine = ChatExt::owner_here(&bus, &gate, &key);
                let released = mine.as_deref().is_some_and(|c| gate.take_released(c, GateLane::Chat));
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
                    let (me, g) = (self.clone(), gate.clone());
                    tokio::spawn(async move { me.drain(g, key, cid, due).await });
                }
            }
        }
    }

    async fn drain(self: Arc<Self>, gate: Arc<Gate>, key: String, cid: String, due: Due) {
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
            match gate.deliver(&cid, false, msgs, Some(tx)) {
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
