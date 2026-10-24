// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-SHAPE · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 정:CAND-BUS-NO-ZENOH-BOOT · model: claude-opus-5-5
//! zenoh 세션 — hub 는 router(임베드) · sfud 는 client(로컬 hub 하나). 전제 둘(qos 켬 · lowlatency 끔)을 늘 적는다.

use crate::system::Zenoh as ZenohCfg;

#[derive(Debug, Clone)]
pub struct BusError(pub String);

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bus: {}", self.0)
    }
}

impl std::error::Error for BusError {}

fn set(c: &mut zenoh::Config, key: &str, v: serde_json::Value) -> Result<(), BusError> {
    c.insert_json5(key, &v.to_string()).map_err(|e| BusError(format!("{key}: {e}")))
}

fn fixed(c: &mut zenoh::Config) -> Result<(), BusError> {
    set(c, "transport/unicast/qos/enabled", serde_json::Value::Bool(true))?;
    set(c, "transport/unicast/lowlatency", serde_json::Value::Bool(false))
}

pub fn router_config(z: &ZenohCfg) -> Result<zenoh::Config, BusError> {
    let mut c = match &z.base_config {
        Some(p) if !p.is_empty() => zenoh::Config::from_file(p).map_err(|e| BusError(format!("base_config {p}: {e}")))?,
        _ => zenoh::Config::default(),
    };
    set(&mut c, "mode", serde_json::Value::String(z.mode.clone()))?;
    set(&mut c, "listen/endpoints", serde_json::json!(z.listen))?;
    set(&mut c, "connect/endpoints", serde_json::json!(z.connect))?;
    set(&mut c, "scouting/multicast/enabled", serde_json::Value::Bool(z.multicast_scouting))?;
    set(&mut c, "transport/unicast/qos/enabled", serde_json::Value::Bool(z.qos_enabled))?;
    set(&mut c, "transport/unicast/lowlatency", serde_json::Value::Bool(z.lowlatency))?;
    fixed(&mut c)?;
    Ok(c)
}

pub fn client_config(connect: &str) -> Result<zenoh::Config, BusError> {
    let mut c = zenoh::Config::default();
    set(&mut c, "mode", serde_json::Value::String("client".into()))?;
    set(&mut c, "connect/endpoints", serde_json::json!([connect]))?;
    set(&mut c, "listen/endpoints", serde_json::json!([]))?;
    set(&mut c, "scouting/multicast/enabled", serde_json::Value::Bool(false))?;
    fixed(&mut c)?;
    Ok(c)
}

pub async fn open(c: zenoh::Config) -> Result<zenoh::Session, BusError> {
    zenoh::open(c).await.map_err(|e| BusError(e.to_string()))
}
