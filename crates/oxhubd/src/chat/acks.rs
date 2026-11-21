// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CHAT-UNDELIVERED · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! ACK 줄 둘 — 멤버 몫(미전송 기록)과 3rd 몫(`ext_pending`)을 따로 둔 줄에 넣는다(줄마다 상한 · 한쪽이 차도 다른 쪽은 받는다) · 지우는 주인은 하나 — 기다렸다가 줄 하나를 골라(둘 다 있으면 번갈아) 그 줄에서 묶음을 거둔다 · 닫히면 남은 것을 다 거둔 뒤 끝.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

use super::store::{Ack, Lane};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    Queued,
    Full,
    Closed,
}

#[derive(Default)]
struct Queues {
    member: VecDeque<Ack>,
    ext: VecDeque<Ack>,
    last: Option<Lane>,
    closed: bool,
}

impl Queues {
    fn of(&mut self, lane: Lane) -> &mut VecDeque<Ack> {
        match lane {
            Lane::Member => &mut self.member,
            Lane::Ext => &mut self.ext,
        }
    }

    fn pick(&self) -> Option<Lane> {
        match (self.member.is_empty(), self.ext.is_empty()) {
            (true, true) => None,
            (false, true) => Some(Lane::Member),
            (true, false) => Some(Lane::Ext),
            (false, false) => Some(if self.last == Some(Lane::Member) { Lane::Ext } else { Lane::Member }),
        }
    }
}

pub struct AckLanes {
    cap_member: usize,
    cap_ext: usize,
    q: Mutex<Queues>,
    cv: Condvar,
}

impl AckLanes {
    pub fn new(cap_member: usize, cap_ext: usize) -> AckLanes {
        AckLanes { cap_member, cap_ext, q: Mutex::new(Queues::default()), cv: Condvar::new() }
    }

    fn cap(&self, lane: Lane) -> usize {
        match lane {
            Lane::Member => self.cap_member,
            Lane::Ext => self.cap_ext,
        }
    }

    pub fn push(&self, lane: Lane, ack: Ack) -> Push {
        let Ok(mut g) = self.q.lock() else { return Push::Closed };
        if g.closed {
            return Push::Closed;
        }
        let cap = self.cap(lane);
        let q = g.of(lane);
        if q.len() >= cap {
            return Push::Full;
        }
        q.push_back(ack);
        drop(g);
        self.cv.notify_one();
        Push::Queued
    }

    pub fn next(&self) -> Option<(Lane, Ack)> {
        let mut g = self.q.lock().ok()?;
        loop {
            if let Some(lane) = g.pick() {
                g.last = Some(lane);
                return g.of(lane).pop_front().map(|a| (lane, a));
            }
            if g.closed {
                return None;
            }
            g = self.cv.wait(g).ok()?;
        }
    }

    pub fn more(&self, lane: Lane) -> Option<Ack> {
        self.q.lock().ok()?.of(lane).pop_front()
    }

    pub fn len(&self, lane: Lane) -> usize {
        self.q.lock().map(|mut g| g.of(lane).len()).unwrap_or(0)
    }

    pub fn close(&self) {
        if let Ok(mut g) = self.q.lock() {
            g.closed = true;
        }
        self.cv.notify_all();
    }
}
