// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-BUS-SHAPE 정:RULE-BUS-KEYSPACE 정:RULE-BUS-PRESENCE 정:RULE-BUS-PLACEMENT 정:RULE-BUS-EVENTS 정:RULE-BUS-REQUEST 정:CAND-BUS-REQUEST-TIMEOUT · model: claude-opus-5-5
//! 버스 — 키 공간 · HRW 배치(FNV-1a 64 → splitmix64) · zenoh 세션 설정(router · client) · 시한.

use std::time::Duration;

pub const NODE_CALL_TIMEOUT: Duration = Duration::from_secs(10);
pub const SESS_GET_TIMEOUT: Duration = Duration::from_secs(2);
pub const LEAVE_FLUSH: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct Keys {
    pub ns: String,
}

impl Keys {
    pub fn new(ns: &str) -> Keys {
        Keys { ns: ns.to_string() }
    }

    fn base(&self) -> String {
        format!("ox/{}/int", self.ns)
    }

    pub fn node(&self, node: &str, inst: &str) -> String {
        format!("{}/node/{node}/{inst}", self.base())
    }

    pub fn node_all(&self) -> String {
        format!("{}/node/**", self.base())
    }

    pub fn room(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/room/{room}/{node}/{inst}", self.base())
    }

    pub fn room_all(&self) -> String {
        format!("{}/room/**", self.base())
    }

    pub fn sess(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/sess/{user}/{node}/{sid}", self.base())
    }

    pub fn sess_of_user(&self, user: &str) -> String {
        format!("{}/sess/{user}/**", self.base())
    }

    pub fn sess_by_sid(&self, sid: &str) -> String {
        format!("{}/sess/*/*/{sid}", self.base())
    }

    pub fn ev_room(&self, room: &str) -> String {
        format!("{}/ev/room/{room}", self.base())
    }

    pub fn ev_user(&self, user: &str) -> String {
        format!("{}/ev/user/{user}", self.base())
    }

    pub fn ev_node(&self, node: &str) -> String {
        format!("{}/ev/node/{node}", self.base())
    }

    pub fn q_node(&self, node: &str, what: &str) -> String {
        format!("{}/q/node/{node}/{what}", self.base())
    }

    pub fn q_node_all(&self, node: &str) -> String {
        format!("{}/q/node/{node}/**", self.base())
    }

    pub fn tail<'a>(&self, key: &'a str, kind: &str) -> Option<Vec<&'a str>> {
        let pre = format!("{}/{kind}/", self.base());
        key.strip_prefix(pre.as_str())
            .map(|r| r.split('/').collect())
    }
}

pub fn room_id_ok(room: &str) -> bool {
    !room.is_empty() && !room.contains(['/', '*', '$', '?', '#'])
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

pub fn place_score(node: &str, room: &str) -> u64 {
    let mut b = Vec::with_capacity(node.len() + room.len() + 1);
    b.extend_from_slice(node.as_bytes());
    b.push(0);
    b.extend_from_slice(room.as_bytes());
    splitmix64(fnv1a64(&b))
}

pub fn place<'a>(nodes: impl IntoIterator<Item = &'a str>, room: &str) -> Option<&'a str> {
    let mut best: Option<(&str, u64)> = None;
    for n in nodes {
        let s = place_score(n, room);
        best = match best {
            None => Some((n, s)),
            Some((bn, bs)) if s > bs || (s == bs && n < bn) => Some((n, s)),
            keep => keep,
        };
    }
    best.map(|(n, _)| n)
}

pub struct ZenohOpts<'a> {
    pub mode: &'a str,
    pub listen: &'a [String],
    pub connect: &'a [String],
    pub multicast_scouting: bool,
    pub base_config: Option<&'a str>,
}

pub fn zenoh_config(o: &ZenohOpts<'_>) -> Result<zenoh::Config, String> {
    let mut c = match o.base_config {
        Some(p) if !p.is_empty() => {
            zenoh::Config::from_file(p).map_err(|e| format!("zenoh base_config {p}: {e}"))?
        }
        _ => zenoh::Config::default(),
    };
    let js = |v: &serde_json::Value| v.to_string();
    let set = |c: &mut zenoh::Config, k: &str, v: String| {
        c.insert_json5(k, &v).map_err(|e| format!("zenoh {k}: {e}"))
    };
    set(&mut c, "mode", js(&serde_json::Value::from(o.mode)))?;
    set(
        &mut c,
        "listen/endpoints",
        js(&serde_json::Value::from(o.listen.to_vec())),
    )?;
    set(
        &mut c,
        "connect/endpoints",
        js(&serde_json::Value::from(o.connect.to_vec())),
    )?;
    set(
        &mut c,
        "scouting/multicast/enabled",
        js(&serde_json::Value::from(o.multicast_scouting)),
    )?;
    set(&mut c, "transport/unicast/qos/enabled", "true".into())?;
    set(&mut c, "transport/unicast/lowlatency", "false".into())?;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_known_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn splitmix_first_step_from_zero() {
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn hrw_is_pure_and_tie_breaks_by_name() {
        let nodes = ["node-a", "node-b", "node-c"];
        let a = place(nodes.iter().copied(), "r1");
        let b = place(nodes.iter().rev().copied(), "r1");
        assert_eq!(a, b);
        assert!(a.is_some());
    }

    #[test]
    fn keyspace_shapes() {
        let k = Keys::new("ox");
        assert_eq!(k.room("r1", "n1", "e1"), "ox/ox/int/room/r1/n1/e1");
        assert_eq!(
            k.tail("ox/ox/int/room/r1/n1/e1", "room"),
            Some(vec!["r1", "n1", "e1"])
        );
        assert!(!room_id_ok("a/b"));
        assert!(room_id_ok("r-1"));
    }
}
