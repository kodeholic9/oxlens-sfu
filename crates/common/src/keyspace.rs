// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-KEYSPACE · 정:RULE-CHAT-EXT-SUB · 정:RULE-HOOK-KEYSPACE · 정:RULE-HOOK-FACTS-SUB · model: claude-opus-5-5
//! 버스 키 공간 — 알릴 것을 전부 키에 적는다(liveliness 토큰은 값을 못 싣는다) · 방 id 가 node 앞 · 3rd 연결 위치(`ext` · 사실 구독 `facts`) · 대외 채팅 · 사실 구독 키(`pub/v1/chat/sub` · `pub/v1/facts/sub`).

pub const FORBIDDEN: &[char] = &['*', '$', '?', '#', '/'];

pub fn valid_piece(s: &str) -> bool {
    !s.is_empty() && !s.contains(FORBIDDEN)
}

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

    pub fn node_token(&self, node: &str, inst: &str) -> String {
        format!("{}/node/{node}/{inst}", self.base())
    }

    pub fn node_all(&self) -> String {
        format!("{}/node/**", self.base())
    }

    pub fn room_token(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/room/{room}/{node}/{inst}", self.base())
    }

    pub fn room_where(&self, room: &str) -> String {
        format!("{}/room/{room}/**", self.base())
    }

    pub fn room_all(&self) -> String {
        format!("{}/room/**", self.base())
    }

    pub fn rooms_of_node(&self, node: &str) -> String {
        format!("{}/room/*/{node}/**", self.base())
    }

    pub fn sess_token(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/sess/{user}/{node}/{sid}", self.base())
    }

    pub fn sess_all(&self) -> String {
        format!("{}/sess/**", self.base())
    }

    pub fn ext_token(&self, key: &str, node: &str, cid: &str) -> String {
        format!("{}/ext/{key}/{node}/{cid}", self.base())
    }

    pub fn ext_all(&self) -> String {
        format!("{}/ext/**", self.base())
    }

    pub fn facts_token(&self, key: &str, node: &str, cid: &str) -> String {
        format!("{}/facts/{key}/{node}/{cid}", self.base())
    }

    pub fn facts_all(&self) -> String {
        format!("{}/facts/**", self.base())
    }

    pub fn pub_prefix(&self) -> String {
        format!("ox/{}/pub/v1/", self.ns)
    }

    pub fn facts_sub_prefix(&self) -> String {
        format!("{}facts/sub/", self.pub_prefix())
    }

    pub fn facts_sub(&self, key: &str) -> String {
        format!("{}{key}", self.facts_sub_prefix())
    }

    pub fn chat_sub_prefix(&self) -> String {
        format!("ox/{}/pub/v1/chat/sub/", self.ns)
    }

    pub fn chat_sub(&self, key: &str) -> String {
        format!("{}{key}", self.chat_sub_prefix())
    }

    pub fn ev_room(&self, room: &str) -> String {
        format!("{}/ev/room/{room}", self.base())
    }

    pub fn ev_room_all(&self) -> String {
        format!("{}/ev/room/*", self.base())
    }

    pub fn ev_user(&self, user: &str) -> String {
        format!("{}/ev/user/{user}", self.base())
    }

    pub fn ev_user_all(&self) -> String {
        format!("{}/ev/user/*", self.base())
    }

    pub fn ev_node(&self, node: &str) -> String {
        format!("{}/ev/node/{node}", self.base())
    }

    pub fn q_node(&self, node: &str, rest: &str) -> String {
        format!("{}/q/node/{node}/{rest}", self.base())
    }

    pub fn q_node_serve(&self, node: &str) -> String {
        format!("{}/q/node/{node}/**", self.base())
    }

    pub fn q_all_nodes(&self, rest: &str) -> String {
        format!("{}/q/node/*/{rest}", self.base())
    }

    pub fn parse(&self, key: &str) -> Option<Parsed> {
        let rest = key.strip_prefix(&format!("{}/", self.base()))?;
        let parts: Vec<&str> = rest.split('/').collect();
        match parts.as_slice() {
            ["node", node, inst] => Some(Parsed::Node { node: node.to_string(), inst: inst.to_string() }),
            ["room", room, node, inst] => {
                Some(Parsed::Room { room: room.to_string(), node: node.to_string(), inst: inst.to_string() })
            }
            ["sess", user, node, sid] => {
                Some(Parsed::Sess { user: user.to_string(), node: node.to_string(), sid: sid.to_string() })
            }
            ["ext", key, node, cid] => Some(Parsed::Ext { key: key.to_string(), node: node.to_string(), cid: cid.to_string() }),
            ["facts", key, node, cid] => {
                Some(Parsed::Facts { key: key.to_string(), node: node.to_string(), cid: cid.to_string() })
            }
            ["ev", "room", room] => Some(Parsed::EvRoom(room.to_string())),
            ["ev", "user", user] => Some(Parsed::EvUser(user.to_string())),
            ["ev", "node", node] => Some(Parsed::EvNode(node.to_string())),
            ["q", "node", node, rest @ ..] => Some(Parsed::Query { node: node.to_string(), path: rest.join("/") }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Node { node: String, inst: String },
    Room { room: String, node: String, inst: String },
    Sess { user: String, node: String, sid: String },
    Ext { key: String, node: String, cid: String },
    Facts { key: String, node: String, cid: String },
    EvRoom(String),
    EvUser(String),
    EvNode(String),
    Query { node: String, path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_before_node_one_wildcard_each_way() {
        let k = Keys::new("c1");
        assert_eq!(k.room_token("r1", "n1", "e1"), "ox/c1/int/room/r1/n1/e1");
        assert_eq!(k.room_where("r1"), "ox/c1/int/room/r1/**");
        assert_eq!(k.rooms_of_node("n1"), "ox/c1/int/room/*/n1/**");
        assert_eq!(
            k.parse("ox/c1/int/room/r1/n1/e1"),
            Some(Parsed::Room { room: "r1".into(), node: "n1".into(), inst: "e1".into() })
        );
    }

    #[test]
    fn forbidden_pieces() {
        for bad in ["", "a*b", "a$", "a?", "#", "a/b"] {
            assert!(!valid_piece(bad));
        }
        assert!(valid_piece("room-1"));
    }

    #[test]
    fn ext_token_and_chat_sub_keys() {
        let k = Keys::new("c1");
        assert_eq!(k.ext_token("acme", "n1", "e1a"), "ox/c1/int/ext/acme/n1/e1a");
        assert_eq!(k.parse("ox/c1/int/ext/acme/n1/e1a"), Some(Parsed::Ext { key: "acme".into(), node: "n1".into(), cid: "e1a".into() }));
        assert_eq!(k.ext_all(), "ox/c1/int/ext/**");
        assert_eq!(k.chat_sub("acme"), "ox/c1/pub/v1/chat/sub/acme");
    }

    #[test]
    fn facts_token_and_facts_sub_keys() {
        let k = Keys::new("c1");
        assert_eq!(k.facts_token("acme", "n1", "e1a"), "ox/c1/int/facts/acme/n1/e1a", "S-rule-RULE-HOOK-DIRECT-2 ② — 사실 구독 선언 → int/facts/{{key}}/{{node}}/{{cid}}");
        assert_eq!(
            k.parse("ox/c1/int/facts/acme/n1/e1a"),
            Some(Parsed::Facts { key: "acme".into(), node: "n1".into(), cid: "e1a".into() })
        );
        assert_eq!(k.facts_all(), "ox/c1/int/facts/**");
        assert_eq!(k.facts_sub("acme"), "ox/c1/pub/v1/facts/sub/acme", "HK-KEY-PUB-V1-FACTS-SUB-KEY ① — hub 가 묻는 키");
        assert_eq!(k.pub_prefix(), "ox/c1/pub/v1/");
    }
}
