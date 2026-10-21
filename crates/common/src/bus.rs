// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-SHAPE 정:RULE-BUS-PRESENCE 정:RULE-BUS-EVENTS 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! zenoh 버스 — hub 는 router, sfud 는 client. `ev` put 은 Drop, 존재는 liveliness 토큰, 요청은 queryable.

use std::time::Duration;

use zenoh::Session;
use zenoh::qos::CongestionControl;

#[derive(Debug, Clone)]
pub struct BusConfig {
    pub mode: &'static str,
    pub listen: Vec<String>,
    pub connect: Vec<String>,
    pub multicast_scouting: bool,
}

fn json_list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

pub async fn open(cfg: &BusConfig) -> Result<Session, String> {
    let mut c = zenoh::Config::default();
    let set = |c: &mut zenoh::Config, k: &str, v: &str| c.insert_json5(k, v).map_err(|e| format!("zenoh config {k}: {e}"));
    set(&mut c, "mode", &format!("\"{}\"", cfg.mode))?;
    set(&mut c, "listen/endpoints", &json_list(&cfg.listen))?;
    set(&mut c, "connect/endpoints", &json_list(&cfg.connect))?;
    set(&mut c, "scouting/multicast/enabled", if cfg.multicast_scouting { "true" } else { "false" })?;
    set(&mut c, "transport/unicast/qos/enabled", "true")?;
    set(&mut c, "transport/unicast/lowlatency", "false")?;
    zenoh::open(c).await.map_err(|e| e.to_string())
}

pub async fn put_event(s: &Session, key: &str, payload: Vec<u8>) {
    if let Err(e) = s.put(key, payload).congestion_control(CongestionControl::Drop).await {
        tracing::warn!(kind = "diag", key = key, "bus put failed: {e}");
    }
}

pub async fn query(s: &Session, key: &str, payload: Vec<u8>, timeout: Duration) -> Vec<Result<Vec<u8>, String>> {
    let mut out = Vec::new();
    let replies = match s.get(key).payload(payload).timeout(timeout).await {
        Ok(r) => r,
        Err(e) => {
            out.push(Err(e.to_string()));
            return out;
        }
    };
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) => out.push(Ok(sample.payload().to_bytes().to_vec())),
            Err(e) => out.push(Err(String::from_utf8_lossy(&e.payload().to_bytes()).to_string())),
        }
    }
    out
}

pub async fn live_keys(s: &Session, key: &str, timeout: Duration) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(replies) = s.liveliness().get(key).timeout(timeout).await else { return out };
    while let Ok(reply) = replies.recv_async().await {
        if let Ok(sample) = reply.result() {
            out.push(sample.key_expr().as_str().to_string());
        }
    }
    out
}
