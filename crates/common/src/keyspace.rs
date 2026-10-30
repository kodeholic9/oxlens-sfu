// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 버스 키 공간 — 알릴 것을 전부 키에 적는다(liveliness 토큰은 값을 못 싣는다) · 방 id 가 node 앞.

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
}
