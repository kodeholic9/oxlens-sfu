// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · 정:RULE-HOOK-WEBHOOK · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 계정 큐 — 방을 맡은 hub 가 3rd 계정마다 하나(원천 `server/areas/hook/behavior/facts.ts` `Queue` 의 거울 · 벡터 `hook.json`): 상한에 닿으면 가장 오래된 것을 버리고 버림 수에 더한다 · 묶음은 앞에서부터 바이트 상한까지(첫 것은 커도 혼자) · 줄이 비어도 버림 수가 있으면 빈 묶음 · 싣고 나면 버림 수 0 · 끝내 못 보낸 묶음은 그 사실 수와 실었던 버림 수를 되돌려 더한다.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Put {
    Queued,
    DroppedOldest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bundle<T> {
    pub items: Vec<T>,
    pub dropped: u64,
}

#[derive(Debug, Clone)]
pub struct Queue<T> {
    items: VecDeque<(T, usize)>,
    dropped: u64,
    max: usize,
}

impl<T> Queue<T> {
    pub fn new(max: usize) -> Queue<T> {
        Queue { items: VecDeque::new(), dropped: 0, max: max.max(1) }
    }

    pub fn put(&mut self, item: T, bytes: usize) -> Put {
        let mut r = Put::Queued;
        if self.items.len() >= self.max {
            self.items.pop_front();
            self.dropped += 1;
            r = Put::DroppedOldest;
        }
        self.items.push_back((item, bytes));
        r
    }

    pub fn take(&mut self, max_bytes: usize) -> Option<Bundle<T>> {
        if self.items.is_empty() && self.dropped == 0 {
            return None;
        }
        let mut items = Vec::new();
        let mut bytes = 0usize;
        while let Some((_, b)) = self.items.front() {
            if !items.is_empty() && bytes + b > max_bytes {
                break;
            }
            bytes += b;
            if let Some((it, _)) = self.items.pop_front() {
                items.push(it);
            }
        }
        Some(Bundle { items, dropped: std::mem::take(&mut self.dropped) })
    }

    pub fn lost(&mut self, facts: u64, carried: u64) {
        self.dropped += facts + carried;
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn ids<'a>(&'a self, f: impl Fn(&'a T) -> &'a str) -> Vec<&'a str> {
        self.items.iter().map(|(t, _)| f(t)).collect()
    }
}
