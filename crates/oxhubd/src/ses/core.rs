// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SES-SOCKET-LIFE · 정:RULE-SES-READ-AND-PROCESS · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-WINDOW-PER-DIRECTION · 연:RULE-TRN-ROOM-STATE-ONE-LANE · 연:COND-TRN-FIFO-IN-LANE · 연:RULE-TRN-NOTIFY-ACK · model: claude-opus-5-5
//! 송신 큐(순수) — 우선순위 단 넷 · 단 안 FIFO(응답과 통지가 한 줄) · 서버 `00` 프레임만 윈도를 먹는다 · ACK 시한 · 밀린 프레임 상한.

use std::collections::{BTreeMap, VecDeque};

pub const ACK_TIMEOUT_MS: u64 = 30_000;
pub const BACKLOG_MAX: usize = 1_000;
pub const BIND_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub lane: usize,
    pub op: u16,
    pub needs_ack: bool,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Overflow,
}

#[derive(Debug, Default)]
pub struct OutQueue {
    lanes: [VecDeque<Item>; 4],
    inflight: BTreeMap<(u16, u32), u64>,
    window: usize,
}

impl OutQueue {
    pub fn new(window: usize) -> OutQueue {
        OutQueue { window: window.clamp(1, 10), ..OutQueue::default() }
    }

    fn len(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    pub fn push(&mut self, item: Item) -> Verdict {
        if self.len() >= BACKLOG_MAX {
            return Verdict::Overflow;
        }
        let lane = item.lane.min(3);
        self.lanes[lane].push_back(item);
        Verdict::Ok
    }

    pub fn pop(&mut self) -> Option<Item> {
        let room = self.inflight.len() < self.window;
        for lane in self.lanes.iter_mut() {
            if let Some(head) = lane.front() {
                if head.needs_ack && !room {
                    return None;
                }
                return lane.pop_front();
            }
        }
        None
    }

    pub fn sent(&mut self, op: u16, pid: u32, now_ms: u64) {
        self.inflight.insert((op, pid), now_ms);
    }

    pub fn ack(&mut self, op: u16, pid: u32) -> bool {
        self.inflight.remove(&(op, pid)).is_some()
    }

    pub fn ack_overdue(&self, now_ms: u64) -> bool {
        self.inflight.values().any(|t| now_ms.saturating_sub(*t) >= ACK_TIMEOUT_MS)
    }

    pub fn clear(&mut self) {
        for l in self.lanes.iter_mut() {
            l.clear();
        }
    }
}

pub fn idle_expired(last_rx_ms: u64, now_ms: u64, timeout_ms: u64) -> bool {
    now_ms.saturating_sub(last_rx_ms) >= timeout_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(lane: usize, op: u16, needs_ack: bool) -> Item {
        Item { lane, op, needs_ack, bytes: vec![op as u8] }
    }

    #[test]
    fn lane_fifo_and_window_blocks_only_its_lane_head() {
        let mut q = OutQueue::new(1);
        q.push(it(1, 0x0701, true));
        q.push(it(1, 0x0201, false));
        q.push(it(2, 0x0501, true));
        let a = q.pop().expect("first");
        assert_eq!(a.op, 0x0701);
        q.sent(0x0701, 1, 0);
        assert_eq!(q.pop().map(|i| i.op), Some(0x0201), "응답은 윈도를 안 먹는다");
        assert_eq!(q.pop(), None, "윈도가 차면 다음 00 프레임은 기다린다");
        assert!(q.ack(0x0701, 1));
        assert_eq!(q.pop().map(|i| i.op), Some(0x0501));
    }

    #[test]
    fn overflow_and_ack_timeout() {
        let mut q = OutQueue::new(8);
        for _ in 0..BACKLOG_MAX {
            assert_eq!(q.push(it(1, 1, false)), Verdict::Ok);
        }
        assert_eq!(q.push(it(1, 1, false)), Verdict::Overflow, "SV-CODE-4006-4008 밀린 프레임 1,000 초과는 4007");
        q.sent(0x0702, 9, 100);
        assert!(!q.ack_overdue(100 + ACK_TIMEOUT_MS - 1));
        assert!(q.ack_overdue(100 + ACK_TIMEOUT_MS), "SV-CODE-4006-4008 ACK 30초는 4006");
        assert!(idle_expired(0, BIND_TIMEOUT_MS, BIND_TIMEOUT_MS));
    }
}
