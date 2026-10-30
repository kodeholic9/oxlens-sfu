// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-AUX-TASK · 연:AUX.op.TASK · 운영:RULE-READ-USER-PROBE · model: claude-opus-5-5
//! `TASK` — 서버가 발제(`req_id` 단조증가) · 클라 `report` 를 새 프레임으로 받는다 · 클라가 보낸 `request` 는 빈 성공 · probe 시한 3s.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use oxsig::msg::{ProbeResult, TaskReq, TaskType};
use oxsig::{Code, Failure, Op};
use tokio::sync::oneshot;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Probes {
    next: AtomicU32,
    waiting: Mutex<HashMap<u32, oneshot::Sender<ProbeResult>>>,
}

impl Probes {
    pub fn open(&self) -> (u32, oneshot::Receiver<ProbeResult>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut g) = self.waiting.lock() {
            g.insert(id, tx);
        }
        (id, rx)
    }

    pub fn close(&self, id: u32) {
        if let Ok(mut g) = self.waiting.lock() {
            g.remove(&id);
        }
    }

    fn report(&self, id: u32, r: ProbeResult) {
        let tx = self.waiting.lock().ok().and_then(|mut g| g.remove(&id));
        if let Some(tx) = tx {
            let _ = tx.send(r);
        }
    }
}

pub fn task_from_client(p: &Probes, _session: &str, body: &serde_json::Value) -> Result<serde_json::Value, Failure> {
    match serde_json::from_value::<TaskReq>(body.clone()) {
        Ok(TaskReq::Request { .. }) => Ok(serde_json::Value::Null),
        Ok(TaskReq::Report { req_id, result, .. }) => {
            p.report(req_id, result);
            Ok(serde_json::Value::Null)
        }
        Err(_) => Err(Code::InvalidPayload.failure().with_message("unknown task")),
    }
}

pub async fn probe(p: &Probes, conn: &crate::ses::Conn) -> Option<ProbeResult> {
    let (id, rx) = p.open();
    let req = TaskReq::Request { req_id: id, type_: TaskType::Probe, params: serde_json::json!({}) };
    let sent = conn.send(crate::ses::Out::Notice { op: Op::Task, body: serde_json::to_vec(&req).unwrap_or_default() });
    if !sent {
        p.close(id);
        return None;
    }
    match tokio::time::timeout(PROBE_TIMEOUT, rx).await {
        Ok(Ok(r)) => Some(r),
        _ => {
            p.close(id);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_request_is_empty_success_report_resolves() {
        let p = Probes::default();
        let v = task_from_client(&p, "s", &serde_json::json!({"phase":"request","req_id":1,"type":"probe","params":{}}));
        assert_eq!(v, Ok(serde_json::Value::Null));
        let (id, mut rx) = p.open();
        let r = task_from_client(&p, "s", &serde_json::json!({"phase":"report","req_id":id,"type":"probe","result":{}}));
        assert!(r.is_ok());
        assert!(rx.try_recv().is_ok());
        assert!(task_from_client(&p, "s", &serde_json::json!({"phase":"x"})).is_err());
    }
}
