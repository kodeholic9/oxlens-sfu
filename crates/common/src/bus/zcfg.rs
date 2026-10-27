// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-SHAPE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! zenoh 설정 — hub 는 router(임베드) · sfud 는 client(임베드). 전제 둘(qos 켬 · lowlatency 끔)을 늘 적는다 · 시스템 파일 값이 최종 승자.

use crate::system::Zenoh;

fn list(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())
}

fn base(base_config: Option<&str>) -> Result<zenoh::Config, String> {
    match base_config {
        Some(p) if !p.is_empty() => zenoh::Config::from_file(p).map_err(|e| format!("zenoh base_config {p}: {e}")),
        _ => Ok(zenoh::Config::default()),
    }
}

fn put(c: &mut zenoh::Config, key: &str, v: &str) -> Result<(), String> {
    c.insert_json5(key, v).map_err(|e| format!("zenoh {key}: {e}"))
}

fn premises(c: &mut zenoh::Config) -> Result<(), String> {
    put(c, "transport/unicast/qos/enabled", "true")?;
    put(c, "transport/unicast/lowlatency", "false")
}

pub fn router(z: &Zenoh) -> Result<zenoh::Config, String> {
    let mut c = base(z.base_config.as_deref())?;
    put(&mut c, "mode", "\"router\"")?;
    put(&mut c, "listen/endpoints", &list(&z.listen))?;
    put(&mut c, "connect/endpoints", &list(&z.connect))?;
    put(&mut c, "scouting/multicast/enabled", if z.multicast_scouting { "true" } else { "false" })?;
    premises(&mut c)?;
    Ok(c)
}

pub fn client(connect: &str) -> Result<zenoh::Config, String> {
    let mut c = zenoh::Config::default();
    put(&mut c, "mode", "\"client\"")?;
    put(&mut c, "connect/endpoints", &list(&[connect.to_string()]))?;
    put(&mut c, "scouting/multicast/enabled", "false")?;
    premises(&mut c)?;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_build() {
        let z = Zenoh {
            mode: "router".into(),
            listen: vec!["tcp/127.0.0.1:7447".into()],
            connect: vec![],
            multicast_scouting: false,
            qos_enabled: true,
            lowlatency: false,
            namespace: "ox".into(),
            base_config: None,
        };
        assert!(router(&z).is_ok());
        assert!(client("tcp/127.0.0.1:7447").is_ok());
    }
}
