// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-SHAPE 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! node 안 B 평면 — 로컬 gRPC Handle. 봉투는 `oxsig::bplane` 의 형을 JSON 으로 싣는다.

pub mod pb {
    tonic::include_proto!("oxlens.b.v1");
}

pub use pb::Blob;
pub use pb::unit_client::UnitClient;
pub use pb::unit_server::{Unit, UnitServer};

use oxsig::bplane::{UnitReq, UnitRes};

pub fn blob<T: serde::Serialize>(v: &T) -> Blob {
    Blob { json: oxsig::bplane::encode(v) }
}

pub async fn call(
    client: &mut UnitClient<tonic::transport::Channel>,
    req: &UnitReq,
    timeout: std::time::Duration,
) -> Result<UnitRes, String> {
    let fut = client.call(blob(req));
    match tokio::time::timeout(timeout, fut).await {
        Err(_) => Err("timeout".into()),
        Ok(Err(e)) => Err(e.message().to_string()),
        Ok(Ok(res)) => oxsig::bplane::decode::<UnitRes>(&res.into_inner().json).map_err(|e| e.to_string()),
    }
}

pub async fn connect(addr: &str) -> Result<UnitClient<tonic::transport::Channel>, String> {
    let uri = if addr.starts_with("http") { addr.to_string() } else { format!("http://{addr}") };
    UnitClient::connect(uri).await.map_err(|e| e.to_string())
}
