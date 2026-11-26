// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-SCOPE · 정:RULE-HOOK-DIRECT · 정:RULE-HOOK-TRANSPORT · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-OPS-DROPS · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! 대외 연동(HOOK) — 가로지르는 것: 방을 맡은 hub 의 사실 짓기(도장 주인 · 사실 봉투) · 3rd 계정마다 계정 큐(`facts` 고르기 · 상한 · 버림 표지 · 계수 `fact_queue_full` · `fact_given_up`) · 현황 조회 · 제어 답(`query`) · 운반 — webhook(`webhook`) · zenoh 사실 구독(`carry` — webhook 이 붙은 계정은 뺀다) · zenoh 입구(`zgate` — 채팅 구독 · 사실 구독 · 현황 조회 · 제어가 같은 입구 하나를 쓴다). 개념(채팅)은 이것을 부르고, 이것은 채팅 넘기기 판단(`crate::chat::ext::plan`)을 직접 부른다 — 콜백 주입 없음.

pub mod carry;
pub mod facts;
pub mod query;
pub mod queue;
pub mod webhook;
pub mod zgate;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use common::envelope::Material;
use common::system::System;

use facts::{Fact, Stamp};
use queue::{Put, Queue};
use zgate::send::FactsBundle;

pub struct Hook {
    inst: String,
    accounts: Vec<(String, Vec<String>)>,
    queue_max: usize,
    stamp: Mutex<Stamp>,
    queues: Mutex<HashMap<String, Queue<Fact>>>,
    pub queue_full: AtomicU64,
    pub given_up: AtomicU64,
    pub wake: tokio::sync::Notify,
    pub zgate: std::sync::OnceLock<std::sync::Arc<zgate::Gate>>,
    pub carry_turns: AtomicU64,
}

impl Hook {
    pub fn new(inst: &str, accounts: Vec<(String, Vec<String>)>, queue_max: u32) -> Hook {
        Hook {
            inst: inst.to_string(),
            accounts,
            queue_max: usize::try_from(queue_max).unwrap_or(usize::MAX),
            stamp: Mutex::new(Stamp::default()),
            queues: Mutex::new(HashMap::new()),
            queue_full: AtomicU64::new(0),
            given_up: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
            zgate: std::sync::OnceLock::new(),
            carry_turns: AtomicU64::new(0),
        }
    }

    pub fn zenoh_accounts(&self, sys: &System) -> Vec<String> {
        self.accounts.iter().map(|(k, _)| k.clone()).filter(|k| sys.hook.as_ref().is_none_or(|h| h.webhook_of(k).is_none())).collect()
    }

    pub fn mint(&self, now: u64) -> String {
        let (ms, c) = self.stamp.lock().map(|mut s| s.next(now)).unwrap_or((now, 0));
        facts::fact_id(ms, c, &self.inst)
    }

    pub fn accounts_of(sys: &System) -> Vec<(String, Vec<String>)> {
        sys.hub
            .auth
            .api_keys
            .iter()
            .filter(|k| k.hook_allowed && k.wants_facts())
            .map(|k| (k.key.clone(), k.facts.clone().unwrap_or_default()))
            .collect()
    }

    pub fn build(&self, room: &str, kind: &str, body: serde_json::Value, now: u64) -> Fact {
        Fact { fact_id: self.mint(now), kind: kind.to_string(), t: now, room_id: room.to_string(), body }
    }

    pub fn of_material(&self, room: &str, m: &Material, now: u64) -> Fact {
        let v = serde_json::to_value(m).unwrap_or_default();
        self.build(room, m.name(), v.get("body").cloned().unwrap_or_default(), now)
    }

    pub fn offer(&self, f: Fact) -> usize {
        let bytes = serde_json::to_vec(&f).map(|v| v.len()).unwrap_or(0);
        let Ok(mut q) = self.queues.lock() else { return 0 };
        let mut n = 0;
        for (key, want) in &self.accounts {
            if !facts::wants(want, &f.kind) {
                continue;
            }
            let put = q.entry(key.clone()).or_insert_with(|| Queue::new(self.queue_max)).put(f.clone(), bytes);
            if put == Put::DroppedOldest {
                self.queue_full.fetch_add(1, Ordering::Relaxed);
            }
            n += 1;
        }
        drop(q);
        if n > 0 {
            self.wake.notify_waiters();
        }
        n
    }

    pub fn take(&self, key: &str, max_bytes: usize) -> Option<FactsBundle> {
        let mut all = self.queues.lock().ok()?;
        let b = all.get_mut(key)?.take(max_bytes)?;
        Some(FactsBundle { facts: b.items, dropped: (b.dropped > 0).then_some(b.dropped) })
    }

    pub fn lost(&self, key: &str, b: &FactsBundle) {
        let facts = b.facts.len() as u64;
        self.given_up.fetch_add(facts, Ordering::Relaxed);
        if let Ok(mut q) = self.queues.lock() {
            q.entry(key.to_string()).or_insert_with(|| Queue::new(self.queue_max)).lost(facts, b.dropped.unwrap_or(0));
        }
    }

    pub fn queued(&self, key: &str) -> usize {
        self.queues.lock().ok().and_then(|all| all.get(key).map(Queue::len)).unwrap_or(0)
    }
}
