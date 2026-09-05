// author: kodeholic (powered by Claude)
//! 서버→클라 흐름 제어 — 연§3-2. 우선순위 4단(같은 단은 FIFO) · 슬라이딩 윈도우(1~10) · ACK 대기.
//! 서버도 자기 `00` 프레임의 응답(ACK)을 같은 규칙으로 기다린다.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use oxsig::frame::{Header, encode};
use oxsig::op::Op;

const TIERS: usize = 4;

struct InFlight {
    pid: u32,
    sent_at: Instant,
}

struct Pending {
    pid: u32,
    frame: Vec<u8>,
}

pub struct OutboundQueue {
    sending: VecDeque<InFlight>,
    pending: [VecDeque<Pending>; TIERS],
    window: usize,
    next_pid: u32,
}

impl OutboundQueue {
    pub fn new(window: usize) -> Self {
        Self {
            sending: VecDeque::new(),
            pending: [VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new()],
            window: window.clamp(oxsig::timers::WINDOW_DEFAULT, oxsig::timers::WINDOW_MAX),
            next_pid: 0,
        }
    }

    /// 연§3-1 — 자기 안에서 하나씩 올리고 넘치면 0 으로 감긴다.
    fn alloc_pid(&mut self) -> u32 {
        self.next_pid = self.next_pid.wrapping_add(1);
        self.next_pid
    }

    /// 통지 하나를 넣고, 지금 보낼 수 있는 프레임을 돌려준다.
    pub fn enqueue(&mut self, op: u16, body: &[u8], now: Instant) -> Vec<Vec<u8>> {
        let pid = self.alloc_pid();
        let frame = encode(&Header::msg(op, pid), body);
        let tier = Op::from_code(op).map_or(1, Op::tier);
        self.pending[usize::from(tier).min(TIERS - 1)].push_back(Pending { pid, frame });
        self.drain(now)
    }

    pub fn ack(&mut self, pid: u32, now: Instant) -> Vec<Vec<u8>> {
        self.sending.retain(|f| f.pid != pid);
        self.drain(now)
    }

    fn drain(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while self.sending.len() < self.window {
            let Some(msg) = self.pending.iter_mut().find_map(VecDeque::pop_front) else { break };
            self.sending.push_back(InFlight { pid: msg.pid, sent_at: now });
            out.push(msg.frame);
        }
        out
    }

    /// 연§3-1 — 보낸 것에 응답이 `timeout` 안에 없다.
    pub fn expired(&self, now: Instant, timeout: Duration) -> bool {
        self.sending.iter().any(|f| now.duration_since(f.sent_at) > timeout)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.iter().map(VecDeque::len).sum()
    }

    pub fn in_flight(&self) -> usize {
        self.sending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::frame::decode;

    fn pid(f: &[u8]) -> u32 {
        decode(f).unwrap().0.pid
    }

    #[test]
    fn window_ack_and_tiers() {
        let now = Instant::now();
        let mut q = OutboundQueue::new(1);
        let a = q.enqueue(Op::TrackEvent.code(), b"{}", now);
        assert_eq!(a.len(), 1);
        assert!(q.enqueue(Op::Message.code(), b"{}", now).is_empty());
        assert!(q.enqueue(Op::ParticipantEvent.code(), b"{}", now).is_empty());
        assert_eq!(q.pending_count(), 2);
        let next = q.ack(pid(&a[0]), now);
        assert_eq!(decode(&next[0]).unwrap().0.op, Op::ParticipantEvent.code(), "1단이 2단보다 먼저");
        assert!(q.expired(now + Duration::from_secs(31), Duration::from_secs(30)));
    }

    #[test]
    fn pid_wraps_to_zero() {
        let mut q = OutboundQueue::new(10);
        q.next_pid = u32::MAX - 1;
        let now = Instant::now();
        let f1 = q.enqueue(Op::RoomEvent.code(), b"", now);
        let f2 = q.enqueue(Op::RoomEvent.code(), b"", now);
        assert_eq!((pid(&f1[0]), pid(&f2[0])), (u32::MAX, 0));
    }
}
