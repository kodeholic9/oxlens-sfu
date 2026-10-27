// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:CAND-BUS-HANDLE-SHAPE · 정:RULE-OPS-SUPERVISOR · model: claude-opus-5-5
//! node 안 Handle — rpc 둘(`Hello` · `Call`). 유닛 준비 = `Hello` 성공.

pub mod pb {
    tonic::include_proto!("oxlens.handle.v1");
}

pub use pb::handle_client::HandleClient;
pub use pb::handle_server::{Handle, HandleServer};
pub use pb::{CallReq, CallRes, HelloReq, HelloRes};
