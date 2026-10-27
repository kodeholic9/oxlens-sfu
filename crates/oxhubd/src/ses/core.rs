// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SES-BIND · 정:IMPL-SES-SOCKET-LIFE · 연:RULE-SES-AUTH-TWO-WAYS · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-WINDOW-PER-DIRECTION · 연:RULE-TRN-LANES · 연:RULE-TRN-ROOM-STATE-ONE-LANE · 연:DEV-TRN-SESSION-SECRET · model: claude-opus-5-5
//! 세션 판정(순수) — 인증 두 갈래 · 발급값 · 서버발 통지 송신 창(단 4 · 같은 단 FIFO · 윈도우 · 밀린 프레임 상한 · ACK 시한).

use std::collections::{BTreeMap, VecDeque};

use oxsig::op::Lane;

pub const MAX_PENDING: usize = 1_000;
pub const ACK_TIMEOUT_MS: u64 = 30_000;
pub const BIND_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Branch {
    Resume(String),
    Elsewhere(String),
    Token,
}

pub fn bind_branch(session_id: Option<&str>, local_live: bool) -> Branch {
    match session_id {
        Some(s) if !s.is_empty() && local_live => Branch::Resume(s.to_string()),
        Some(s) if !s.is_empty() => Branch::Elsewhere(s.to_string()),
        _ => Branch::Token,
    }
}

pub fn new_session_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Push {
    Queued,
    Overflow,
}

#[derive(Debug)]
pub struct OutQueue {
    lanes: [VecDeque<(u16, Vec<u8>)>; 4],
    inflight: BTreeMap<u32, u64>,
    window: usize,
    next_pid: u32,
}

impl OutQueue {
    pub fn new(window: u8) -> OutQueue {
        OutQueue { lanes: Default::default(), inflight: BTreeMap::new(), window: usize::from(window.clamp(1, 10)), next_pid: 1 }
    }

    pub fn pending(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    pub fn push(&mut self, lane: Lane, op: u16, body: Vec<u8>) -> Push {
        if self.pending() >= MAX_PENDING {
            return Push::Overflow;
        }
        self.lanes[lane.index()].push_back((op, body));
        Push::Queued
    }

    pub fn pop_ready(&mut self, now_ms: u64) -> Option<(u32, u16, Vec<u8>)> {
        if self.inflight.len() >= self.window {
            return None;
        }
        let (op, body) = self.lanes.iter_mut().find_map(VecDeque::pop_front)?;
        let pid = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);
        self.inflight.insert(pid, now_ms);
        Some((pid, op, body))
    }

    pub fn ack(&mut self, pid: u32) -> bool {
        self.inflight.remove(&pid).is_some()
    }

    pub fn overdue(&self, now_ms: u64, limit_ms: u64) -> bool {
        self.inflight.values().any(|t| now_ms.saturating_sub(*t) >= limit_ms)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_branch_wins_over_token() {
        assert_eq!(bind_branch(Some("abc"), true), Branch::Resume("abc".into()));
        assert_eq!(bind_branch(Some("abc"), false), Branch::Elsewhere("abc".into()));
        assert_eq!(bind_branch(Some(""), true), Branch::Token);
        assert_eq!(bind_branch(None, true), Branch::Token);
    }

    #[test]
    fn session_ids_are_unpredictable_hex() {
        let a = new_session_id();
        assert_eq!(a.len(), 32);
        assert_ne!(a, new_session_id());
    }

    #[test]
    fn window_and_lane_priority() {
        let mut q = OutQueue::new(2);
        q.push(Lane::Diagnostic, 0x0601, vec![]);
        q.push(Lane::Session, 0x0701, vec![1]);
        q.push(Lane::Session, 0x0702, vec![2]);
        let a = q.pop_ready(0).expect("a");
        let b = q.pop_ready(0).expect("b");
        assert_eq!((a.1, b.1), (0x0701, 0x0702));
        assert!(q.pop_ready(0).is_none());
        assert!(q.ack(a.0));
        assert_eq!(q.pop_ready(0).map(|x| x.1), Some(0x0601));
        assert!(!q.overdue(29_999, ACK_TIMEOUT_MS));
        assert!(q.overdue(30_000, ACK_TIMEOUT_MS));
    }

    #[test]
    fn overflow_after_thousand_pending() {
        let mut q = OutQueue::new(1);
        for _ in 0..MAX_PENDING {
            assert_eq!(q.push(Lane::Session, 1, vec![]), Push::Queued);
        }
        assert_eq!(q.push(Lane::Session, 1, vec![]), Push::Overflow);
    }
}
