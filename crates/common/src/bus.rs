// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-SHAPE · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-PRESENCE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 버스 — 키 공간(값 없는 토큰은 키에 다 적는다) · 세션 열기(hub = router · sfud = client) · 통지 봉투.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub ns: String,
}

impl Keys {
    pub fn new(ns: &str) -> Keys {
        Keys { ns: ns.to_string() }
    }

    fn int(&self) -> String {
        format!("{}/int", self.ns)
    }

    pub fn node_token(&self, node: &str, inst: &str) -> String {
        format!("{}/node/{node}/{inst}", self.int())
    }

    pub fn node_all(&self) -> String {
        format!("{}/node/**", self.int())
    }

    pub fn room_token(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/room/{room}/{node}/{inst}", self.int())
    }

    pub fn room_all(&self) -> String {
        format!("{}/room/**", self.int())
    }

    pub fn room_where(&self, room: &str) -> String {
        format!("{}/room/{room}/*/*", self.int())
    }

    pub fn rooms_of_node(&self, node: &str) -> String {
        format!("{}/room/*/{node}/*", self.int())
    }

    pub fn sess_token(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/sess/{user}/{node}/{sid}", self.int())
    }

    pub fn sess_of_user(&self, user: &str) -> String {
        format!("{}/sess/{user}/**", self.int())
    }

    pub fn sess_by_sid(&self, sid: &str) -> String {
        format!("{}/sess/*/*/{sid}", self.int())
    }

    pub fn ev_room(&self, room: &str) -> String {
        format!("{}/ev/room/{room}", self.int())
    }

    pub fn ev_user(&self, user: &str) -> String {
        format!("{}/ev/user/{user}", self.int())
    }

    pub fn ev_node(&self, node: &str) -> String {
        format!("{}/ev/node/{node}", self.int())
    }

    pub fn q_node(&self, node: &str, what: &str) -> String {
        format!("{}/q/node/{node}/{what}", self.int())
    }

    pub fn q_node_all_of(&self, node: &str) -> String {
        format!("{}/q/node/{node}/**", self.int())
    }

    pub fn q_fanout(&self, what: &str) -> String {
        format!("{}/q/node/*/{what}", self.int())
    }

    pub fn parse(&self, key: &str) -> Option<Vec<String>> {
        let rest = key.strip_prefix(&format!("{}/", self.int()))?;
        Some(rest.split('/').map(str::to_string).collect())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub op: u16,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sfu_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Value>,
}

pub struct BusConf<'a> {
    pub mode: &'a str,
    pub listen: &'a [String],
    pub connect: &'a [String],
    pub multicast_scouting: bool,
}

pub fn zenoh_config(c: &BusConf<'_>) -> Result<zenoh::Config, String> {
    let mut cfg = zenoh::Config::default();
    let put = |cfg: &mut zenoh::Config, k: &str, v: String| cfg.insert_json5(k, &v).map_err(|e| format!("zenoh {k}: {e}"));
    put(&mut cfg, "mode", format!("\"{}\"", c.mode))?;
    put(&mut cfg, "listen/endpoints", serde_json::to_string(c.listen).map_err(|e| e.to_string())?)?;
    put(&mut cfg, "connect/endpoints", serde_json::to_string(c.connect).map_err(|e| e.to_string())?)?;
    put(&mut cfg, "scouting/multicast/enabled", c.multicast_scouting.to_string())?;
    put(&mut cfg, "transport/unicast/qos/enabled", "true".to_string())?;
    put(&mut cfg, "transport/unicast/lowlatency", "false".to_string())?;
    if c.mode == "client" {
        put(&mut cfg, "connect/timeout_ms", "-1".to_string()).ok();
    }
    Ok(cfg)
}

pub async fn open(c: &BusConf<'_>) -> Result<zenoh::Session, String> {
    let cfg = zenoh_config(c)?;
    zenoh::open(cfg).await.map_err(|e| format!("zenoh open: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_before_node_is_one_star_each_way() {
        let k = Keys::new("ox");
        assert_eq!(k.room_token("r1", "n1", "i1"), "ox/int/room/r1/n1/i1");
        assert_eq!(k.parse("ox/int/room/r1/n1/i1"), Some(vec!["room".into(), "r1".into(), "n1".into(), "i1".into()]));
        assert_eq!(k.sess_by_sid("s1"), "ox/int/sess/*/*/s1");
        assert!(k.parse("other/int/room/r").is_none());
    }

    #[test]
    fn room_question_is_one_star_not_a_node_sweep() {
        let k = Keys::new("ox");
        let stars = |s: &str| s.split('/').filter(|p| *p == "*").count();
        let where_r1 = k.room_where("r1");
        assert!(where_r1.starts_with("ox/int/room/r1/"), "S-sym-RULE-BUS-KEYSPACE-2 {where_r1}");
        assert_eq!(stars(&k.rooms_of_node("n1")), 2, "S-sym-RULE-BUS-KEYSPACE-2");
        let flipped = format!("ox/int/room/*/{}/*", "r1");
        assert_ne!(where_r1, flipped, "S-sym-RULE-BUS-KEYSPACE-2 뒤집힌 순서는 방 질의가 노드 칸을 훑는다");
    }
}
