// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-BUS-REQUEST · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! zenoh 사실 구독 운반 — 방을 맡은 hub 가 3rd 계정마다 하나(★webhook 이 붙은 계정은 webhook 이 운반이라 여기 없다): 계정 큐에서 묶음을 꺼내 그 계정의 사실 구독 연결(`int/facts/{key}/{node}/{cid}`) 중 다음 차례(라운드 로빈)로 보낸다 — 제 node 면 입구 줄에 · 다른 node 면 `q/node/{node}/ext` 사실 갈래(받는 hub 가 그 묶음의 끝까지 기다려 답한다) · 줄에 못 넣으면(연결 없음 · 밀림 · 줄 넘침 · 못 닿음) 다음 연결 · ACK 면 끝 · ACK 아닌 끝(거절 · 시한 · 닫힘)은 시도 수 +1 하고 다음 연결 · 시도 수 상한에 닿으면 버리고 되돌려 센다 · 한 바퀴 모두 못 넣으면 묶음을 쥔 채 쉰다 · 연결이 없으면 큐에 둔다.

use std::sync::Arc;
use std::time::Duration;

use common::envelope::Reply;
use tokio::sync::oneshot;

use super::Hook;
use super::zgate::send::{End, Ended, ExtDeliver, FactsBundle, WHAT, facts_query_timeout};
use crate::bus::Bus;

pub const IDLE_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
pub struct Knobs {
    pub max_tries: u32,
    pub bundle_max_bytes: usize,
    pub reply_timeout: Duration,
}

impl Knobs {
    pub fn of(hook: &common::policy::Hook, bundle_max_bytes: usize) -> Knobs {
        Knobs { max_tries: u32::from(hook.ext_max_tries), bundle_max_bytes, reply_timeout: Duration::from_millis(u64::from(hook.ext_reply_timeout_ms)) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Try {
    Ended(End),
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Done,
    Next(u32),
    GiveUp,
}

pub fn step(t: Try, tries: u32, max: u32) -> Step {
    match t {
        Try::Ended(End::Ack) => Step::Done,
        Try::Refused => Step::Next(tries),
        Try::Ended(_) if tries + 1 >= max => Step::GiveUp,
        Try::Ended(_) => Step::Next(tries + 1),
    }
}

async fn send_one(hook: &Hook, bus: &Bus, node: &str, cid: &str, key: &str, b: &FactsBundle, k: &Knobs) -> Try {
    if node == bus.node {
        let Some(gate) = hook.zgate.get() else { return Try::Refused };
        let (tx, rx) = oneshot::channel();
        return match gate.deliver_facts(cid, b, Some(tx)) {
            Ok(o) if o.accepted() => match tokio::time::timeout(facts_query_timeout(k.reply_timeout), rx).await {
                Ok(Ok(o)) => Try::Ended(End::of(o)),
                _ => Try::Ended(End::Closed),
            },
            _ => Try::Refused,
        };
    }
    let d = ExtDeliver::Facts { key: key.to_string(), cid: cid.to_string(), bundle: b.clone() };
    match bus.query_for(node, WHAT, serde_json::to_vec(&d).unwrap_or_default(), facts_query_timeout(k.reply_timeout)).await {
        Ok(bytes) => match Reply::decode(&bytes) {
            Ok(Reply::Ok { body }) => serde_json::from_value::<Ended>(body).map_or(Try::Refused, |e| Try::Ended(e.end)),
            _ => Try::Refused,
        },
        Err(_) => Try::Refused,
    }
}

pub async fn run(hook: Arc<Hook>, bus: Arc<Bus>, key: String, k: Knobs) {
    let mut rr = 0u64;
    let mut held: Option<(FactsBundle, u32)> = None;
    loop {
        let wake = hook.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        if held.is_none() {
            held = hook.take(&key, k.bundle_max_bytes).map(|b| (b, 0));
        }
        let Some((b, mut tries)) = held.take() else {
            let _ = tokio::time::timeout(IDLE_TICK, wake).await;
            continue;
        };
        let conns = bus.facts_list(&key);
        rr = rr.wrapping_add(1);
        let mut outcome = None;
        let mut reached = false;
        for i in crate::chat::ext::plan::ring(conns.len(), rr) {
            let (node, cid) = &conns[i];
            let t = send_one(&hook, &bus, node, cid, &key, &b, &k).await;
            reached |= t != Try::Refused;
            match step(t, tries, k.max_tries) {
                Step::Done => {
                    outcome = Some(Step::Done);
                    break;
                }
                Step::GiveUp => {
                    tracing::warn!(kind = "diag", spec = "SV-RULE-HOOK-FACTS-SUB", key = %key, tries = tries + 1, facts = b.facts.len(), "facts bundle given up — tries reached the limit");
                    hook.lost(&key, &b);
                    outcome = Some(Step::GiveUp);
                    break;
                }
                Step::Next(n) => tries = n,
            }
        }
        if outcome.is_none() {
            held = Some((b, tries));
            if !reached {
                let _ = tokio::time::timeout(IDLE_TICK, wake).await;
            }
        }
    }
}
