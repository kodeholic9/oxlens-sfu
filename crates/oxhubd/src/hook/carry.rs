// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-BUS-REQUEST · 정:RULE-OPS-DROPS · 정책:hook · model: claude-opus-5-5
//! zenoh 사실 구독 운반(판단 `step` · `backoff` · `pick` · `next_wait` 은 원천 — ★연결이 없거나 창이 다 찼으면 시각이 아니라 사건(보내기 하나의 끝 · 연결 목록이 바뀜 · 큐에 새 사실)으로만 깬다 · 보내기 태스크가 비정상으로 끝나면 오류 진단 줄을 남기고 그 묶음을 ACK 아닌 끝처럼 줄에 되돌린다 · 원천 `server/areas/hook/behavior/carry.ts` 의 거울 · 벡터 `hook.json`) — 방을 맡은 hub 가 3rd 계정마다 하나(★webhook 이 붙은 계정은 webhook 이 운반이라 여기 없다): 계정 큐에서 묶음을 꺼내 그 계정의 사실 구독 연결(`int/facts/{key}/{node}/{cid}`) 중 다음 차례(라운드 로빈)로 보낸다 — ★연결마다 창(`hook.ext_window`) 안에서는 끝을 기다리지 않고 다음 묶음을 다음 연결로 내보낸다(연결 사이에 차례가 없다 — 3rd 는 `fact_id` 로 정렬) · 제 node 면 입구 줄에 · 다른 node 면 `q/node/{node}/ext` 사실 갈래(받는 hub 가 그 묶음의 끝까지 기다려 답한다 · 그 넘기기의 시한 넘김도 ACK 아닌 끝) · 끝은 묶음마다 따로: ACK 면 끝 · ACK 아닌 끝(거절 · 시한 · 닫힘)은 시도 수 +1 하고 `hook.retry_backoff_ms` 의 n 번째 간격(넘으면 마지막 값) 뒤 다음 연결 · 시도 수 상한에 닿으면 버리고 되돌려 센다 · 줄에 못 넣으면(연결 없음 · 밀림 · 줄 넘침 · 죽은 node) 세지 않고 다음 연결 — 한 바퀴 모두 못 넣으면 쉰다 · 연결이 없으면 큐에 둔다.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use common::envelope::Reply;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::Hook;
use super::zgate::send::{End, Ended, ExtDeliver, FactsBundle, WHAT, facts_query_timeout};
use crate::bus::Bus;

pub const IDLE_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct Knobs {
    pub max_tries: u32,
    pub bundle_max_bytes: usize,
    pub reply_timeout: Duration,
    pub window: usize,
    pub backoff_ms: Vec<u32>,
}

impl Knobs {
    pub fn of(hook: &common::policy::Hook, bundle_max_bytes: usize) -> Knobs {
        Knobs {
            max_tries: u32::from(hook.ext_max_tries),
            bundle_max_bytes,
            reply_timeout: Duration::from_millis(u64::from(hook.ext_reply_timeout_ms)),
            window: usize::from(hook.ext_window).max(1),
            backoff_ms: hook.retry_backoff_ms.clone(),
        }
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

pub fn backoff(failures: u32, ms: &[u32]) -> Duration {
    let i = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    Duration::from_millis(u64::from(ms.get(i).or(ms.last()).copied().unwrap_or(0)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Event,
    Until(Duration),
}

pub fn next_wait(conns: usize, free: bool, earliest_ready_in_ms: Option<i64>) -> Wait {
    if conns == 0 || !free {
        return Wait::Event;
    }
    match earliest_ready_in_ms {
        Some(ms) => Wait::Until(Duration::from_millis(ms.max(0).unsigned_abs())),
        None => Wait::Event,
    }
}

pub type Conn = (String, String);

pub fn pick(inflight: &[usize], window: usize, rr: u64) -> Option<usize> {
    crate::chat::ext::plan::ring(inflight.len(), rr).into_iter().find(|i| inflight[*i] < window)
}

struct Waiting {
    bundle: FactsBundle,
    tries: u32,
    refused: usize,
    ready: Instant,
}

async fn send_one(hook: Arc<Hook>, bus: Arc<Bus>, conn: Conn, key: String, b: FactsBundle, reply_timeout: Duration) -> Try {
    let (node, cid) = conn;
    if node == bus.node {
        let Some(gate) = hook.zgate.get() else { return Try::Refused };
        let (tx, rx) = oneshot::channel();
        return match gate.deliver_facts(&cid, &b, Some(tx)) {
            Ok(o) if o.accepted() => match tokio::time::timeout(facts_query_timeout(reply_timeout), rx).await {
                Ok(Ok(o)) => Try::Ended(End::of(o)),
                Ok(Err(_)) => Try::Ended(End::Closed),
                Err(_) => Try::Ended(End::Timeout),
            },
            _ => Try::Refused,
        };
    }
    if !bus.view.lock().map(|v| v.node_live(&node)).unwrap_or(false) {
        return Try::Refused;
    }
    let d = ExtDeliver::Facts { key, cid, bundle: b };
    match bus.query_for(&node, WHAT, serde_json::to_vec(&d).unwrap_or_default(), facts_query_timeout(reply_timeout)).await {
        Ok(bytes) => match Reply::decode(&bytes) {
            Ok(Reply::Ok { body }) => serde_json::from_value::<Ended>(body).map_or(Try::Refused, |e| Try::Ended(e.end)),
            _ => Try::Refused,
        },
        Err(_) => Try::Ended(End::Timeout),
    }
}

fn count(inflight: &mut HashMap<Conn, usize>, conn: &Conn, up: bool) {
    let now = inflight.get(conn).copied().unwrap_or(0);
    let next = if up { now + 1 } else { now.saturating_sub(1) };
    if next == 0 {
        inflight.remove(conn);
    } else {
        inflight.insert(conn.clone(), next);
    }
}

struct Sending {
    bundle: FactsBundle,
    tries: u32,
    refused: usize,
    conn: Conn,
}

fn ended(hook: &Hook, bus: &Bus, key: &str, k: &Knobs, waiting: &mut VecDeque<Waiting>, s: Sending, t: Try) {
    let now = Instant::now();
    match step(t, s.tries, k.max_tries) {
        Step::Done => {}
        Step::GiveUp => {
            tracing::warn!(kind = "diag", spec = "SV-RULE-HOOK-FACTS-SUB", key = %key, tries = s.tries + 1, facts = s.bundle.facts.len(), "facts bundle given up — tries reached the limit");
            hook.lost(key, &s.bundle);
        }
        Step::Next(n) if n > s.tries => waiting.push_back(Waiting { bundle: s.bundle, tries: n, refused: 0, ready: now + backoff(n, &k.backoff_ms) }),
        Step::Next(n) => {
            let around = s.refused + 1 >= bus.facts_list(key).len().max(1);
            waiting.push_back(Waiting { bundle: s.bundle, tries: n, refused: if around { 0 } else { s.refused + 1 }, ready: if around { now + IDLE_TICK } else { now } });
        }
    }
}

pub async fn run(hook: Arc<Hook>, bus: Arc<Bus>, key: String, k: Knobs) {
    let mut rr = 0u64;
    let mut inflight: HashMap<Conn, usize> = HashMap::new();
    let mut waiting: VecDeque<Waiting> = VecDeque::new();
    let mut sends: JoinSet<Try> = JoinSet::new();
    let mut sending: HashMap<tokio::task::Id, Sending> = HashMap::new();
    loop {
        hook.carry_turns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let wake = hook.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let moved = bus.facts_changed.notified();
        tokio::pin!(moved);
        moved.as_mut().enable();
        let conns = bus.facts_list(&key);
        loop {
            let counts: Vec<usize> = conns.iter().map(|c| inflight.get(c).copied().unwrap_or(0)).collect();
            let Some(i) = pick(&counts, k.window, rr) else { break };
            let now = Instant::now();
            let next = match waiting.iter().position(|w| w.ready <= now) {
                Some(pos) => waiting.remove(pos),
                None => hook.take(&key, k.bundle_max_bytes).map(|bundle| Waiting { bundle, tries: 0, refused: 0, ready: now }),
            };
            let Some(w) = next else { break };
            rr = rr.wrapping_add(1);
            let conn = conns[i].clone();
            count(&mut inflight, &conn, true);
            let (h, b, kk, t, c, bundle) = (hook.clone(), bus.clone(), key.clone(), k.reply_timeout, conn.clone(), w.bundle.clone());
            let id = sends.spawn(send_one(h, b, c, kk, bundle, t)).id();
            sending.insert(id, Sending { bundle: w.bundle, tries: w.tries, refused: w.refused, conn });
        }
        let counts: Vec<usize> = conns.iter().map(|c| inflight.get(c).copied().unwrap_or(0)).collect();
        let free = pick(&counts, k.window, rr).is_some();
        let now = Instant::now();
        let earliest = waiting.iter().map(|w| w.ready).min().map(|r| if r > now { i64::try_from((r - now).as_millis()).unwrap_or(i64::MAX).max(1) } else { 0 });
        let until = match next_wait(conns.len(), free, earliest) {
            Wait::Until(d) => Some(now + d),
            Wait::Event => None,
        };
        tokio::select! {
            Some(done) = sends.join_next_with_id(), if !sends.is_empty() => {
                let (id, t) = match done {
                    Ok((id, t)) => (id, t),
                    Err(e) => {
                        tracing::error!(kind = "diag", spec = "SV-RULE-HOOK-FACTS-SUB", key = %key, "facts send task ended abnormally: {e} — the bundle goes back to the line as a non-ACK end");
                        (e.id(), Try::Ended(End::Closed))
                    }
                };
                if let Some(s) = sending.remove(&id) {
                    count(&mut inflight, &s.conn, false);
                    ended(&hook, &bus, &key, &k, &mut waiting, s, t);
                }
            }
            _ = &mut wake => {}
            _ = &mut moved => {}
            _ = tokio::time::sleep_until(until.unwrap_or(now)), if until.is_some() => {}
        }
    }
}
