// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SES-SOCKET-LIFE · 정:RULE-SES-READ-AND-PROCESS · 연:RULE-TRN-FLOW-CONTROL · 연:RULE-TRN-LANES · 연:RULE-TRN-WINDOW-PER-DIRECTION · 연:RULE-TRN-ROOM-STATE-ONE-LANE · 연:RULE-TRN-CLOSE · model: claude-opus-5-5
//! 보내는 통로(순수) — 단 넷 · 같은 단 FIFO · 서버 `00` 프레임은 윈도우 안에서만 · ACK 대기 시한 · 밀린 상한.

use std::collections::{BTreeMap, VecDeque};

use oxsig::trn::{Kind, WsFrame};

pub const ACK_TIMEOUT_MS: u64 = 30_000;
pub const BIND_TIMEOUT_MS: u64 = 10_000;
pub const BACKLOG_MAX: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Notice { op: u16, body: Vec<u8> },
    Reply { frame: Vec<u8> },
}

#[derive(Clone, Debug)]
pub struct Outbox {
    lanes: [VecDeque<Out>; 4],
    inflight: BTreeMap<u32, (u16, u64)>,
    window: usize,
    next_pid: u32,
}

impl Outbox {
    pub fn new(window: usize) -> Outbox {
        Outbox { lanes: Default::default(), inflight: BTreeMap::new(), window: window.clamp(1, 10), next_pid: 1 }
    }

    pub fn push(&mut self, lane: u8, out: Out) {
        let i = (lane as usize).min(3);
        self.lanes[i].push_back(out);
    }

    pub fn queued(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    pub fn ack(&mut self, pid: u32) -> bool {
        self.inflight.remove(&pid).is_some()
    }

    pub fn next(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        for lane in self.lanes.iter_mut() {
            let Some(head) = lane.front() else { continue };
            match head {
                Out::Reply { .. } => {
                    if let Some(Out::Reply { frame }) = lane.pop_front() {
                        return Some(frame);
                    }
                }
                Out::Notice { .. } => {
                    if self.inflight.len() >= self.window {
                        return None;
                    }
                    if let Some(Out::Notice { op, body }) = lane.pop_front() {
                        let pid = self.next_pid;
                        self.next_pid = self.next_pid.wrapping_add(1).max(1);
                        self.inflight.insert(pid, (op, now_ms));
                        return Some(WsFrame::new(Kind::Req, op, pid, body).encode());
                    }
                }
            }
        }
        None
    }

    pub fn ack_overdue(&self, now_ms: u64) -> bool {
        self.inflight.values().any(|(_, at)| now_ms.saturating_sub(*at) >= ACK_TIMEOUT_MS)
    }
}

pub fn reply_frame(op: u16, pid: u32, ok: bool, body: &[u8]) -> Vec<u8> {
    WsFrame::new(if ok { Kind::Ok } else { Kind::Fail }, op, pid, body.to_vec()).encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::op;

    #[test]
    fn window_blocks_notices_but_lane_order_holds() {
        let mut o = Outbox::new(1);
        o.push(1, Out::Notice { op: op::TRACK_EVENT, body: b"{}".to_vec() });
        o.push(1, Out::Notice { op: op::TRACK_STATE, body: b"{}".to_vec() });
        o.push(1, Out::Reply { frame: reply_frame(op::ROOM_JOIN, 3, true, b"{}") });
        let first = o.next(0).unwrap_or_default();
        assert_eq!(WsFrame::decode(&first).map(|f| f.op), Ok(op::TRACK_EVENT));
        assert!(o.next(0).is_none(), "같은 단 FIFO — 윈도우에 막힌 통지 뒤의 응답은 앞지르지 않는다");
        let pid = WsFrame::decode(&first).map(|f| f.pid).unwrap_or(0);
        assert!(o.ack(pid));
        let second = o.next(0).unwrap_or_default();
        assert_eq!(WsFrame::decode(&second).map(|f| f.op), Ok(op::TRACK_STATE));
        let third = o.next(0).unwrap_or_default();
        assert_eq!(WsFrame::decode(&third).map(|f| (f.op, f.kind())), Ok((op::ROOM_JOIN, Some(Kind::Ok))));
    }

    #[test]
    fn lower_lane_number_goes_first_and_ack_deadline() {
        let mut o = Outbox::new(8);
        o.push(3, Out::Notice { op: op::TASK, body: vec![] });
        o.push(1, Out::Notice { op: op::PARTICIPANT_EVENT, body: vec![] });
        let f = WsFrame::decode(&o.next(100).unwrap_or_default()).map(|f| f.op);
        assert_eq!(f, Ok(op::PARTICIPANT_EVENT));
        assert!(!o.ack_overdue(100 + ACK_TIMEOUT_MS - 1));
        assert!(o.ack_overdue(100 + ACK_TIMEOUT_MS));
    }
}
