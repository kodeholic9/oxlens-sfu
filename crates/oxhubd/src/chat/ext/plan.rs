// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! 채팅 3rd 구독 보내기 판단 — 원천 `server/areas/chat/behavior/extsend.ts` 의 거울(벡터 `chat.json` 이 맞댄다): 넘길 연결 차례(라운드 로빈 한 바퀴) · 연결마다 보낼 줄(창 · 줄 끝 실시간 묶음 합치기 · 줄 상한 = 밀림 · 시간 넘김 = 밀림 + 잇단 수 · 닫는 수 · 풀림) · drain 묶음 고르기(시도된 행은 한 건씩) · drain 때(주인 · 막 주인 · 풀림 · 훑기) · drain 주인(가장 오래 붙어 있는 연결 — `cid` 앞 12 자리 열린 시각).

use std::collections::VecDeque;

pub fn ring(n: usize, c: u64) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let s = (c % n as u64) as usize;
    (0..n).map(|i| (s + i) % n).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Merged,
    Queued,
    RefusedBacklog,
    RefusedFull,
}

impl Offer {
    pub fn accepted(self) -> bool {
        matches!(self, Offer::Merged | Offer::Queued)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Offer::Merged => "merged",
            Offer::Queued => "queued",
            Offer::RefusedBacklog => "refused_backlog",
            Offer::RefusedFull => "refused_full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ack,
    Fail,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    None,
    Released,
    Close,
}

impl After {
    pub fn as_str(self) -> &'static str {
        match self {
            After::None => "none",
            After::Released => "released",
            After::Close => "close",
        }
    }
}

#[derive(Debug)]
pub struct Item<P, W> {
    pub live: bool,
    pub ids: Vec<String>,
    pub bytes: usize,
    pub msgs: Vec<P>,
    pub waiter: Option<W>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Knobs {
    pub window: usize,
    pub cap: usize,
    pub live_max: usize,
    pub close_at: u32,
}

#[derive(Debug)]
pub struct Outbox<P, W> {
    pub knobs: Knobs,
    pub inflight: usize,
    queue: VecDeque<Item<P, W>>,
    pub backlog: bool,
    pub streak: u32,
}

impl<P, W> Outbox<P, W> {
    pub fn new(knobs: Knobs) -> Outbox<P, W> {
        Outbox { knobs, inflight: 0, queue: VecDeque::new(), backlog: false, streak: 0 }
    }

    pub fn queued(&self) -> usize {
        self.queue.iter().map(|q| q.bytes).sum()
    }

    pub fn offer(&mut self, item: Item<P, W>) -> Offer {
        if self.backlog {
            return Offer::RefusedBacklog;
        }
        if self.queued() + item.bytes > self.knobs.cap {
            self.backlog = true;
            return Offer::RefusedFull;
        }
        if item.live
            && let Some(tail) = self.queue.back_mut()
            && tail.live
            && tail.bytes + item.bytes <= self.knobs.live_max
        {
            tail.ids.extend(item.ids);
            tail.msgs.extend(item.msgs);
            tail.bytes += item.bytes;
            return Offer::Merged;
        }
        self.queue.push_back(item);
        Offer::Queued
    }

    pub fn take(&mut self) -> Option<Item<P, W>> {
        if self.inflight >= self.knobs.window {
            return None;
        }
        let it = self.queue.pop_front()?;
        self.inflight += 1;
        Some(it)
    }

    pub fn done(&mut self, o: Outcome) -> After {
        self.inflight = self.inflight.saturating_sub(1);
        if o == Outcome::Ack {
            self.streak = 0;
        }
        if o == Outcome::Timeout {
            self.streak += 1;
            self.backlog = true;
            if self.streak >= self.knobs.close_at {
                return After::Close;
            }
        }
        if self.backlog && self.inflight == 0 && self.queued() <= self.knobs.cap {
            self.backlog = false;
            return After::Released;
        }
        After::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub bytes: usize,
    pub tries: u32,
}

pub fn drain_pick(rows: &[Row], max_msgs: usize, max_bytes: usize) -> Vec<String> {
    let Some(first) = rows.first() else { return Vec::new() };
    if first.tries > 0 {
        return vec![first.id.clone()];
    }
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for r in rows {
        if r.tries > 0 || out.len() >= max_msgs {
            break;
        }
        if !out.is_empty() && bytes + r.bytes > max_bytes {
            break;
        }
        out.push(r.id.clone());
        bytes += r.bytes;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    None,
    All,
    Older(u64),
}

pub fn drain_due(owner: bool, was_owner: bool, released: bool, now: u64, last_sweep: u64, sweep_ms: u64) -> Due {
    if !owner {
        return Due::None;
    }
    if !was_owner || released {
        return Due::All;
    }
    if now.saturating_sub(last_sweep) >= sweep_ms {
        return Due::Older(now.saturating_sub(sweep_ms));
    }
    Due::None
}

pub fn opened(cid: &str) -> u64 {
    cid.get(..12).filter(|h| h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).and_then(|h| u64::from_str_radix(h, 16).ok()).unwrap_or(u64::MAX)
}

pub fn drain_owner(conns: &[(String, String)]) -> Option<usize> {
    conns.iter().enumerate().min_by(|(_, (an, ac)), (_, (bn, bc))| (opened(ac), an, ac).cmp(&(opened(bc), bn, bc))).map(|(i, _)| i)
}
