// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-KEYSPACE · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-REQUEST · model: claude-opus-5-5
//! 키 공간 — 존재(liveliness 토큰 · 키뿐) · 이벤트(put) · 요청(queryable). `int/` 는 우리끼리다.

#[derive(Debug, Clone, PartialEq, Eq)]
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

    pub fn room_where(&self, room: &str) -> String {
        format!("{}/room/{room}/*/*", self.base())
    }

    pub fn rooms_of_node(&self, node: &str) -> String {
        format!("{}/room/*/{node}/*", self.base())
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

    pub fn q_node_queryable(&self, node: &str) -> String {
        format!("{}/q/node/{node}/**", self.base())
    }

    pub fn q_all(&self, what: &str) -> String {
        format!("{}/q/node/*/{what}", self.base())
    }

    pub fn chunks<'a>(&self, key: &'a str) -> Option<Vec<&'a str>> {
        let pre = format!("{}/", self.base());
        key.strip_prefix(pre.as_str()).map(|rest| rest.split('/').collect())
    }

    pub fn parse_node(&self, key: &str) -> Option<(String, String)> {
        match self.chunks(key)?.as_slice() {
            ["node", node, inst] => Some((node.to_string(), inst.to_string())),
            _ => None,
        }
    }

    pub fn parse_room(&self, key: &str) -> Option<(String, String, String)> {
        match self.chunks(key)?.as_slice() {
            ["room", room, node, inst] => Some((room.to_string(), node.to_string(), inst.to_string())),
            _ => None,
        }
    }

    pub fn parse_sess(&self, key: &str) -> Option<(String, String, String)> {
        match self.chunks(key)?.as_slice() {
            ["sess", user, node, sid] => Some((user.to_string(), node.to_string(), sid.to_string())),
            _ => None,
        }
    }
}

pub fn chunk_ok(s: &str) -> bool {
    !s.is_empty() && !s.contains(['/', '*', '$', '?', '#'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_before_node_keeps_both_questions_one_wildcard() {
        let k = Keys::new("oxdev");
        let key = k.room("r1", "node-a", "sfu-1");
        assert_eq!(key, "ox/oxdev/int/room/r1/node-a/sfu-1");
        let where_ = k.room_where("r1");
        let of_node = k.rooms_of_node("node-a");
        assert_eq!(where_.matches('*').count(), 2, "S-sym-RULE-BUS-KEYSPACE-2 방 위치 질의는 방 조각을 고정한다");
        assert_eq!(of_node.split('/').filter(|c| *c == "*").count(), 2, "S-sym-RULE-BUS-KEYSPACE-2 노드의 방 전부도 * 한 칸 + inst");
        assert!(where_.starts_with("ox/oxdev/int/room/r1/"), "S-sym-RULE-BUS-KEYSPACE-2 방 id 가 node 앞");
        assert_eq!(k.parse_room(&key), Some(("r1".into(), "node-a".into(), "sfu-1".into())));
    }

    #[test]
    fn inst_is_in_every_presence_key() {
        let k = Keys::new("c");
        assert!(k.node("n", "i1") != k.node("n", "i2"), "S-sym-RULE-BUS-KEYSPACE-1 재기동이 키를 가른다");
        assert_eq!(k.parse_sess(&k.sess("u", "n", "s")), Some(("u".into(), "n".into(), "s".into())));
        assert!(!chunk_ok("a/b") && !chunk_ok("") && chunk_ok("r1"));
    }
}
