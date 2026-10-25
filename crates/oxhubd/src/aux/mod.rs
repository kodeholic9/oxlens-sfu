// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-AUX-TASK 정:RULE-OPS-OBSERVE 연:AUX.op.TASK 연:RULE-AUX-TASK-PROC 연:RULE-AUX-PROBE-NO-INVENT 운영:RULE-READ-USER-PROBE · model: claude-opus-5-5
//! `TASK` 발제 — `req_id` 발급자 단조증가 · 세션마다 `probe` 를 보내고 시한(3s) 안의 `report` 를 모은다(저장 0).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use oxsig::Op;
use oxsig::msg::{ProbeResult, TaskReq, TaskReqRequestType};
use tokio::sync::{Mutex, oneshot};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Probes {
    next: AtomicU32,
    wait: Mutex<HashMap<u32, oneshot::Sender<ProbeResult>>>,
}

impl Probes {
    pub async fn report(&self, req_id: u32, result: ProbeResult) {
        if let Some(tx) = self.wait.lock().await.remove(&req_id) {
            let _ = tx.send(result);
        }
    }

    pub async fn ask(&self, s: &crate::ses::Session) -> Option<ProbeResult> {
        let req_id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let (tx, rx) = oneshot::channel();
        self.wait.lock().await.insert(req_id, tx);
        let body = serde_json::to_vec(&TaskReq::Request { req_id, type_: TaskReqRequestType::Probe, params: serde_json::json!({}) }).unwrap_or_default();
        if !s.send(crate::ses::Out::Notify { op: Op::Task.as_u16(), body }).await {
            self.wait.lock().await.remove(&req_id);
            return None;
        }
        common::ev!(spec = "CP-OP-TASK", session = s.fp.as_str(), user = s.id.user.as_str(), "probe request req_id={req_id}");
        let r = tokio::time::timeout(PROBE_TIMEOUT, rx).await.ok().and_then(Result::ok);
        if r.is_none() {
            self.wait.lock().await.remove(&req_id);
        }
        r
    }
}
