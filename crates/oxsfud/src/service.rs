// author: kodeholic (powered by Claude)
//! B 평면 서버측 — `Handle`(요청→응답 하나) · `Subscribe`(통지 스트림). 정§15-2.

use std::pin::Pin;
use std::sync::Arc;

use common::bplane::{Envelope, SfuService, SubscribeRequest};
use tokio_stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::handlers::Sfu;

pub struct Service {
    pub sfu: Arc<Sfu>,
}

#[tonic::async_trait]
impl SfuService for Service {
    async fn handle(&self, req: Request<Envelope>) -> Result<Response<Envelope>, Status> {
        let env = req.into_inner();
        let wire = self.sfu.handle(&env);
        Ok(Response::new(Envelope { session_id: env.session_id, user_id: env.user_id, room_id: env.room_id, target: String::new(), exclude: Vec::new(), wire, pc_mode: String::new(), participant_type: 0, hidden: false, metadata: String::new() }))
    }

    type SubscribeStream = Pin<Box<dyn Stream<Item = Result<Envelope, Status>> + Send>>;

    async fn subscribe(&self, req: Request<SubscribeRequest>) -> Result<Response<Self::SubscribeStream>, Status> {
        let hub = req.into_inner().hub_id;
        info!(hub = %hub, "hub subscribed");
        let rx = self.sfu.bus.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(move |r| match r {
            Ok(env) => Some(Ok(env)),
            Err(e) => {
                warn!(hub = %hub, error = %e, "event stream lagged — notifications dropped");
                None
            }
        });
        Ok(Response::new(Box::pin(stream)))
    }
}
