// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:CAND-BUS-HANDLE-SHAPE 정:RULE-OPS-SURFACE 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! node 안 Handle 서버 — `Hello(node, hub_inst) → (epoch, build)` · `Call(봉투) → (응답)`.

use std::sync::Arc;

use common::handle::{Envelope, Handle, HelloReq, HelloRes};
use oxsig::envelope::{Call, Reply};
use tonic::{Request, Response, Status};

use super::Sfu;

pub struct Svc {
    pub sfu: Arc<Sfu>,
}

#[tonic::async_trait]
impl Handle for Svc {
    async fn hello(&self, r: Request<HelloReq>) -> Result<Response<HelloRes>, Status> {
        let h = r.into_inner();
        self.sfu.hub_inst.store(Some(Arc::new(h.hub_inst.clone())));
        tracing::info!("hello from hub node={} hub_inst={}", h.node, h.hub_inst);
        Ok(Response::new(HelloRes { epoch: self.sfu.epoch.clone(), build: self.sfu.cfg.build.clone() }))
    }

    async fn call(&self, r: Request<Envelope>) -> Result<Response<Envelope>, Status> {
        let body = r.into_inner().body;
        let reply = match serde_json::from_slice::<Call>(&body) {
            Ok(c) => super::call::dispatch(&self.sfu, c).await,
            Err(e) => Reply::failure(oxsig::Code::InvalidPayload.failure_msg(e.to_string())),
        };
        if let Reply::Fail { failure } = &reply {
            self.sfu.drops.code(failure.code);
        }
        Ok(Response::new(Envelope { body: reply.encode() }))
    }
}
