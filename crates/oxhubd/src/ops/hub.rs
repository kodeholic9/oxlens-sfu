// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-DOC-PLANES 정:RULE-DOC-OWNERSHIP 정:RULE-BUS-REQUEST 정:CAND-DOC-ORCHESTRATION-ASPECT · model: claude-opus-5-5
//! hub 한 프로세스의 상태 — 설정 · 버스 · 세션 표 · 유닛(supervisor) · 로컬 sfu Handle · 발제 대기 · 계수.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use common::log::LogHandle;
use oxsig::Code;
use oxsig::envelope::{Call, Reply};
use tokio::sync::Mutex;

use crate::bus::Bus;
use crate::cfg::Config;
use crate::ops::supervisor::Supervisor;
use crate::ses::Sessions;

pub struct Hub {
    pub cfg: Config,
    pub inst: String,
    pub log: LogHandle,
    pub bus: Arc<Bus>,
    pub ses: Sessions,
    pub sup: Arc<Supervisor>,
    pub aux: crate::aux::Probes,
    pub codes: Mutex<BTreeMap<u16, u64>>,
    pub shutting_down: AtomicBool,
    pub joins: Mutex<(u64, u64)>,
    pub ev_lost: AtomicU64,
}

impl Hub {
    pub fn is_local(&self, node: &str) -> bool {
        node == self.cfg.node_id
    }

    pub async fn count_code(&self, code: u16) {
        self.codes.lock().await.entry(code).and_modify(|v| *v += 1).or_insert(1);
    }

    pub async fn call_node(&self, node: &str, inst: Option<&str>, call: &Call) -> Reply {
        let r = if self.is_local(node) {
            match self.sup.sfu_client(inst).await {
                Some(mut c) => common::handle::call(&mut c, call).await,
                None => Reply::fail(Code::SfuUnavailable),
            }
        } else {
            let body = match serde_json::to_vec(&RemoteCall { inst: inst.map(str::to_string), call: call.clone() }) {
                Ok(b) => b,
                Err(e) => return Reply::failure(Code::InternalError.failure_msg(e.to_string())),
            };
            match self.bus.ask_call(node, body).await {
                Ok(b) => Reply::decode(&b),
                Err(c) => Reply::fail(c),
            }
        };
        if let Reply::Fail { failure } = &r {
            self.count_code(failure.code).await;
        }
        r
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<String> {
        if self.sup.has_epoch(inst) {
            return Some(self.cfg.node_id.clone());
        }
        self.bus.inst_node(inst)
    }

    pub fn stopping(&self) -> bool {
        self.shutting_down.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteCall {
    pub inst: Option<String>,
    pub call: Call,
}
