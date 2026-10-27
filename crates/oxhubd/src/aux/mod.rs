// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-AUX-TASK · 정:CAND-AUX-TASK-CLIENT-REQUEST · 정:RULE-OPS-OBSERVE · 연:AUX.op.TASK · 운영:RULE-READ-USER-PROBE · model: claude-opus-5-5
//! `TASK` 발제 — `req_id` 는 발급자 단조증가 · 클라 `report` 를 새 프레임으로 받아 기다리는 쪽에 건넨다 · 시한 3s 를 넘긴 세션은 무응답 표시.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use oxsig::types::ProbeResult;
use tokio::sync::oneshot;

pub const PROBE_TIMEOUT_MS: u64 = 3_000;

#[derive(Default)]
pub struct Probes {
    next: AtomicU32,
    waiting: Mutex<HashMap<(String, u32), oneshot::Sender<ProbeResult>>>,
}

impl Probes {
    pub fn open(&self, sid: &str) -> (u32, oneshot::Receiver<ProbeResult>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut w) = self.waiting.lock() {
            w.insert((sid.to_string(), id), tx);
        }
        (id, rx)
    }

    pub fn report(&self, sid: &str, req_id: u32, result: ProbeResult) -> bool {
        let tx = self.waiting.lock().ok().and_then(|mut w| w.remove(&(sid.to_string(), req_id)));
        tx.is_some_and(|t| t.send(result).is_ok())
    }

    pub fn close(&self, sid: &str, req_id: u32) {
        if let Ok(mut w) = self.waiting.lock() {
            w.remove(&(sid.to_string(), req_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_reaches_waiter_once() {
        let p = Probes::default();
        let (a, mut rx) = p.open("s");
        let (b, _rx2) = p.open("s");
        assert!(b > a);
        assert!(p.report("s", a, ProbeResult::default()));
        assert!(!p.report("s", a, ProbeResult::default()));
        assert!(rx.try_recv().is_ok());
    }
}
