// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-PUBLIC · 연:RULE-ROOM-CREATE-FORM · model: claude-opus-5-5
//! 키 공간 — liveliness 토큰엔 값이 없어 알릴 것을 전부 키에 적는다. 방 id 가 node 앞이다(그 방 어디 · 그 node 의 방 전부 둘 다 `*` 한 칸).

pub const FORBIDDEN: &[char] = &['*', '$', '?', '#', '/'];

pub fn check_fragment(what: &str, v: &str) -> Result<(), String> {
    if v.is_empty() {
        return Err(format!("{what} 가 비었다 — 버스 키 조각이 못 된다"));
    }
    if let Some(c) = v.chars().find(|c| FORBIDDEN.contains(c)) {
        return Err(format!("{what} = {v} — 버스 키 조각에 못 쓰는 글자 `{c}`"));
    }
    Ok(())
}

pub fn fragment_ok(v: &str) -> bool {
    !v.is_empty() && !v.chars().any(|c| FORBIDDEN.contains(&c))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpace {
    pub ns: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomToken {
    pub room: String,
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeToken {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessToken {
    pub user: String,
    pub node: String,
    pub sid: String,
}

impl KeySpace {
    pub fn new(ns: &str) -> KeySpace {
        KeySpace { ns: ns.to_string() }
    }

    fn int(&self) -> String {
        format!("ox/{}/int", self.ns)
    }

    pub fn node(&self, node: &str, inst: &str) -> String {
        format!("{}/node/{node}/{inst}", self.int())
    }

    pub fn node_all(&self) -> String {
        format!("{}/node/*/*", self.int())
    }

    pub fn room(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/room/{room}/{node}/{inst}", self.int())
    }

    pub fn room_where(&self, room: &str) -> String {
        format!("{}/room/{room}/*/*", self.int())
    }

    pub fn rooms_of_node(&self, node: &str) -> String {
        format!("{}/room/*/{node}/*", self.int())
    }

    pub fn room_all(&self) -> String {
        format!("{}/room/*/*/*", self.int())
    }

    pub fn sess(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/sess/{user}/{node}/{sid}", self.int())
    }

    pub fn sess_of_user(&self, user: &str) -> String {
        format!("{}/sess/{user}/**", self.int())
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

    pub fn q_node_queryable(&self, node: &str) -> String {
        format!("{}/q/node/{node}/**", self.int())
    }

    pub fn q_all(&self, what: &str) -> String {
        format!("{}/q/node/*/{what}", self.int())
    }

    fn rest<'a>(&self, key: &'a str, what: &str) -> Option<Vec<&'a str>> {
        let head = format!("ox/{}/int/{what}/", self.ns);
        key.strip_prefix(head.as_str()).map(|r| r.split('/').collect())
    }

    pub fn parse_room(&self, key: &str) -> Option<RoomToken> {
        match self.rest(key, "room")?.as_slice() {
            [room, node, inst] => Some(RoomToken { room: room.to_string(), node: node.to_string(), inst: inst.to_string() }),
            _ => None,
        }
    }

    pub fn parse_node(&self, key: &str) -> Option<NodeToken> {
        match self.rest(key, "node")?.as_slice() {
            [node, inst] => Some(NodeToken { node: node.to_string(), inst: inst.to_string() }),
            _ => None,
        }
    }

    pub fn parse_sess(&self, key: &str) -> Option<SessToken> {
        match self.rest(key, "sess")?.as_slice() {
            [user, node, sid] => Some(SessToken { user: user.to_string(), node: node.to_string(), sid: sid.to_string() }),
            _ => None,
        }
    }

    pub fn parse_q_node(&self, key: &str) -> Option<(String, String)> {
        let parts = self.rest(key, "q/node")?;
        let (node, what) = parts.split_first()?;
        Some((node.to_string(), what.join("/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_id_comes_before_node() {
        let k = KeySpace::new("ox");
        let key = k.room("r1", "node-a", "e1");
        assert_eq!(key, "ox/ox/int/room/r1/node-a/e1");
        assert_eq!(k.parse_room(&key), Some(RoomToken { room: "r1".into(), node: "node-a".into(), inst: "e1".into() }));
        assert_eq!(k.room_where("r1"), "ox/ox/int/room/r1/*/*", "S-sym-RULE-BUS-KEYSPACE-2 그 방 어디 = `*` 한 칸");
        assert_eq!(k.rooms_of_node("node-a"), "ox/ox/int/room/*/node-a/*");
        let wk = k.room_where("r1");
        let where_parts: Vec<&str> = wk.split('/').collect();
        assert_eq!(where_parts[4], "r1", "S-sym-RULE-BUS-KEYSPACE-2 방 조각이 node 조각 앞");
    }

    #[test]
    fn forbidden_fragments() {
        for bad in ["", "a*b", "a$", "?", "#x", "a/b"] {
            assert!(check_fragment("x", bad).is_err(), "{bad}");
        }
        assert!(check_fragment("x", "room-1_ok.한글").is_ok());
    }

    #[test]
    fn parse_other_tokens() {
        let k = KeySpace::new("c");
        assert_eq!(k.parse_node(&k.node("n", "i")), Some(NodeToken { node: "n".into(), inst: "i".into() }));
        assert_eq!(k.parse_sess(&k.sess("u", "n", "s")), Some(SessToken { user: "u".into(), node: "n".into(), sid: "s".into() }));
        assert_eq!(k.parse_q_node(&k.q_node("n", "call")), Some(("n".into(), "call".into())));
        assert_eq!(k.parse_room("ox/c/int/room/r/n"), None);
    }
}
