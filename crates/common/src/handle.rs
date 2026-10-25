// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:CAND-BUS-HANDLE-SHAPE 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! node 안 Handle — rpc 둘(`Hello(node, hub_inst) → (epoch, build)` · `Call(봉투) → (응답)`).

#[allow(clippy::all, clippy::pedantic, clippy::nursery)]
pub mod pb {
    tonic::include_proto!("oxlens.handle.v1");
}

use std::time::Duration;

use oxsig::Code;
use oxsig::envelope::{Call, Reply};

pub use pb::handle_client::HandleClient;
pub use pb::handle_server::{Handle, HandleServer};
pub use pb::{Envelope, HelloReq, HelloRes};

pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

pub type Client = HandleClient<tonic::transport::Channel>;

pub async fn connect(addr: &str, timeout: Duration) -> Result<Client, String> {
    let uri = if addr.starts_with("http") { addr.to_string() } else { format!("http://{addr}") };
    let ep = tonic::transport::Endpoint::from_shared(uri).map_err(|e| e.to_string())?.connect_timeout(timeout).timeout(CALL_TIMEOUT);
    let ch = ep.connect().await.map_err(|e| e.to_string())?;
    Ok(HandleClient::new(ch))
}

pub async fn call(c: &mut Client, call: &Call) -> Reply {
    let body = match serde_json::to_vec(call) {
        Ok(b) => b,
        Err(e) => {
            return Reply::Fail { failure: Code::InternalError.failure_msg(e.to_string()) };
        }
    };
    match c.call(Envelope { body }).await {
        Ok(r) => Reply::decode(&r.into_inner().body),
        Err(s) => Reply::Fail { failure: Code::SfuError.failure_msg(s.message().to_string()) },
    }
}
