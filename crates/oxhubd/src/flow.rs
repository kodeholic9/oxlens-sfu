// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§3-2 · 정§3-5 · model: claude-opus-5

//! 흐름 제어 — ★★**순서가 곧 상태다.**
//!
//! ★**같은 단 안에서는 먼저 온 것부터**(FIFO)이고, ★**방 상태를 나르는 것은 전부 같은 단**이다.
//! 급하다는 이유로 하나만 위로 올리면 ★**뒤에 온 것이 먼저 도착해 상태가 되감긴다.**
//!
//! ★**재전송이 없다** — 응답이 30초 안에 안 오면 끊는다(`4006`). 밀린 것이 1,000 을 넘어도 끊는다(`4007`).

use oxsig::{Code, Lane, Op};

/// 밀린 프레임 상한. 넘으면 ★**끊는다** — 무한히 쌓지 않는다.
pub const MAX_PENDING: usize = 1_000;
/// 응답 대기 상한.
pub const ACK_TIMEOUT_MS: u64 = 30_000;

/// 내보낼 것 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub op: Op,
    pub pid: u32,
    pub body: Vec<u8>,
    /// 응답을 기다리는가 — 통지(ACK 요구)면 참.
    pub expects_ack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    pid: u32,
    sent_at: u64,
}

/// 한 세션의 송신 큐.
#[derive(Debug)]
pub struct FlowOut {
    /// 단별 큐 — ★**단 안에서는 FIFO 다.**
    lanes: [Vec<Outgoing>; 4],
    /// 응답을 기다리는 것들.
    inflight: Vec<Pending>,
    /// 동시에 기다릴 수 있는 개수(정책 `ws_flow_window`, 1~10).
    window: usize,
    queued: usize,
}

fn lane_index(l: Lane) -> usize {
    match l {
        Lane::Recovery => 0,
        Lane::Session => 1,
        Lane::Data => 2,
        Lane::Diagnostic => 3,
    }
}

impl FlowOut {
    pub fn new(window: u8) -> Self {
        Self {
            lanes: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            inflight: Vec::new(),
            // ★범위는 wire 가 정하고(1~10) 그 안에서 고르는 값이 정책이다.
            window: window.clamp(1, 10) as usize,
            queued: 0,
        }
    }

    pub fn queued(&self) -> usize {
        self.queued
    }

    pub fn inflight(&self) -> usize {
        self.inflight.len()
    }

    /// 큐에 넣는다. ★**상한을 넘으면 `4007`** — 여기서 버리지 않고 끊을 사유를 돌려준다.
    pub fn push(&mut self, item: Outgoing) -> Result<(), Code> {
        if self.queued >= MAX_PENDING {
            return Err(Code::FlowOverflow);
        }
        self.lanes[lane_index(item.op.lane())].push(item);
        self.queued += 1;
        Ok(())
    }

    /// 보낼 수 있는 것을 꺼낸다 — ★**윗 단부터, 단 안에서는 넣은 순서대로.**
    pub fn drain(&mut self, now: u64) -> Vec<Outgoing> {
        let mut out = Vec::new();
        while self.inflight.len() < self.window {
            let Some(i) = self.lanes.iter().position(|l| !l.is_empty()) else { break };
            let item = self.lanes[i].remove(0);
            self.queued -= 1;
            if item.expects_ack {
                self.inflight.push(Pending { pid: item.pid, sent_at: now });
            }
            out.push(item);
        }
        out
    }

    /// 응답(ACK)이 왔다.
    pub fn on_ack(&mut self, pid: u32) {
        self.inflight.retain(|p| p.pid != pid);
    }

    /// ★**30초 안에 응답이 없으면 `4006`** — 재전송하지 않는다(재전송은 새 요청으로 읽힌다).
    pub fn timed_out(&self, now: u64) -> Option<Code> {
        self.inflight
            .iter()
            .any(|p| now.saturating_sub(p.sent_at) >= ACK_TIMEOUT_MS)
            .then_some(Code::FlowTimeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(op: Op, pid: u32) -> Outgoing {
        Outgoing { op, pid, body: Vec::new(), expects_ack: true }
    }

    #[test]
    fn 방_상태는_한_단이라_순서가_지켜진다() {
        // ★하나만 위로 올리면 뒤에 온 것이 먼저 도착해 상태가 되감긴다.
        let mut f = FlowOut::new(10);
        for (i, op) in [Op::ParticipantEvent, Op::TrackEvent, Op::TrackState, Op::ParticipantState]
            .into_iter()
            .enumerate()
        {
            f.push(note(op, i as u32)).expect("push");
        }
        let sent: Vec<u32> = f.drain(0).iter().map(|o| o.pid).collect();
        assert_eq!(sent, vec![0, 1, 2, 3], "★단 안에서는 넣은 순서다");
    }

    #[test]
    fn 데이터와_진단은_뒤로_간다() {
        let mut f = FlowOut::new(10);
        f.push(note(Op::Task, 1)).expect("push");
        f.push(note(Op::Message, 2)).expect("push");
        f.push(note(Op::TrackEvent, 3)).expect("push");
        let sent: Vec<u32> = f.drain(0).iter().map(|o| o.pid).collect();
        assert_eq!(sent, vec![3, 2, 1], "★1단 → 2단 → 3단");
    }

    #[test]
    fn 윈도우만큼만_나간다() {
        let mut f = FlowOut::new(1);
        for i in 0..3 {
            f.push(note(Op::TrackEvent, i)).expect("push");
        }
        assert_eq!(f.drain(0).len(), 1, "★기본 1 이면 순서가 전송 계층에서 저절로 지켜진다");
        assert_eq!(f.drain(0).len(), 0, "★응답 전에는 더 안 나간다");
        f.on_ack(0);
        assert_eq!(f.drain(0).len(), 1);
    }

    #[test]
    fn 윈도우는_wire_범위로_잘린다() {
        assert_eq!(FlowOut::new(0).window, 1);
        assert_eq!(FlowOut::new(99).window, 10);
    }

    #[test]
    fn 응답이_없으면_4006_이다() {
        let mut f = FlowOut::new(1);
        f.push(note(Op::TrackEvent, 7)).expect("push");
        f.drain(1_000);
        assert_eq!(f.timed_out(1_000 + ACK_TIMEOUT_MS - 1), None);
        // ★재전송하지 않는다 — 재전송은 서버에게 새 요청이라 중복 등록이 된다.
        assert_eq!(f.timed_out(1_000 + ACK_TIMEOUT_MS), Some(Code::FlowTimeout));
        f.on_ack(7);
        assert_eq!(f.timed_out(u64::MAX / 2), None);
    }

    #[test]
    fn 밀리면_끊는다() {
        let mut f = FlowOut::new(1);
        for i in 0..MAX_PENDING {
            f.push(note(Op::TrackEvent, i as u32)).expect("push");
        }
        // ★버리지 않고 끊을 사유를 돌려준다 — 조용히 버리면 상태가 갈린 채로 산다.
        assert_eq!(f.push(note(Op::TrackEvent, 9_999)), Err(Code::FlowOverflow));
    }

    #[test]
    fn 응답을_안_기다리는_것은_창을_안_먹는다() {
        let mut f = FlowOut::new(1);
        let mut a = note(Op::Leave, 1);
        a.expects_ack = false;
        f.push(a).expect("push");
        f.push(note(Op::TrackEvent, 2)).expect("push");
        // `LEAVE` 는 응답 없는 op 이라 창을 잡지 않는다.
        assert_eq!(f.drain(0).len(), 2);
    }
}
