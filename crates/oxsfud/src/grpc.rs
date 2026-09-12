// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§15-0 · §15-5 · §16-1 · model: claude-opus-5

//! B 평면 서버 — ★**node 안의 유일한 길**이다.
//!
//! ★**`Hello` 가 답하는 순간이 그 유닛의 `Running`** 이다(정§16-1). 그 전은 `Starting` 이고,
//! ★**둘을 합치면** *"띄웠다"* 와 *"붙었다"* 가 한 값이 되어 급사를 못 본다.

use common::b::sfu_service_server::{SfuService, SfuServiceServer};
use common::b::{Envelope, HelloReply, HelloRequest, SubscribeRequest};
use tonic::{Request, Response, Status};

/// 이 프로세스의 신원. ★**기동마다 새 값**이라 재기동을 가릴 수 있다.
#[derive(Debug, Clone)]
pub struct Identity {
    pub epoch: String,
    pub build: String,
}

#[derive(Debug)]
pub struct Sfu {
    id: Identity,
}

impl Sfu {
    pub fn new(id: Identity) -> Self {
        Self { id }
    }

    pub fn into_server(self) -> SfuServiceServer<Self> {
        SfuServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl SfuService for Sfu {
    /// ★**기동 신원을 건넨다** — hub 가 이것을 받아 들고 `Running` 으로 적는다.
    async fn hello(&self, req: Request<HelloRequest>) -> Result<Response<HelloReply>, Status> {
        let who = req.into_inner().node_id;
        eprintln!("[b] hello ← hub {who}");
        Ok(Response::new(HelloReply {
            epoch: self.id.epoch.clone(),
            build: self.id.build.clone(),
        }))
    }

    /// 미디어 축은 다음 걸음이다 — ★**조용히 성공하지 않는다.**
    async fn handle(&self, _req: Request<Envelope>) -> Result<Response<Envelope>, Status> {
        Err(Status::unimplemented("미디어 축 미구현 — 조용한 성공 금지"))
    }

    type SubscribeStream = tokio_stream::wrappers::ReceiverStream<Result<Envelope, Status>>;

    async fn subscribe(
        &self,
        _req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        Err(Status::unimplemented("통지 스트림 미구현 — 조용한 성공 금지"))
    }
}
