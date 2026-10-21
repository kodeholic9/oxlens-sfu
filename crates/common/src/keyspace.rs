// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-BUS-KEYSPACE 정:RULE-BUS-PRESENCE 연:RULE-ROOM-CREATE-FORM · model: claude-opus-5-5
//! 버스 키 공간 — 존재는 키에 다 적는다(liveliness 토큰은 값을 못 싣는다).

pub const FORBIDDEN: &[char] = &['/', '*', '$', '?', '#'];

pub fn valid_piece(s: &str) -> bool {
    !s.is_empty() && !s.contains(FORBIDDEN)
}

#[derive(Debug, Clone)]
pub struct Keys {
    c: String,
}

impl Keys {
    pub fn new(namespace: &str) -> Keys {
        Keys { c: namespace.to_string() }
    }

    pub fn node(&self, node: &str, inst: &str) -> String {
        format!("{}/int/node/{node}/{inst}", self.c)
    }

    pub fn node_all(&self) -> String {
        format!("{}/int/node/*/*", self.c)
    }

    pub fn room(&self, room: &str, node: &str, inst: &str) -> String {
        format!("{}/int/room/{room}/{node}/{inst}", self.c)
    }

    pub fn room_all(&self) -> String {
        format!("{}/int/room/*/*/*", self.c)
    }

    pub fn sess(&self, user: &str, node: &str, sid: &str) -> String {
        format!("{}/int/sess/{user}/{node}/{sid}", self.c)
    }

    pub fn sess_by_sid(&self, sid: &str) -> String {
        format!("{}/int/sess/*/*/{sid}", self.c)
    }

    pub fn sess_by_user(&self, user: &str) -> String {
        format!("{}/int/sess/{user}/*/*", self.c)
    }

    pub fn ev_room(&self, room: &str) -> String {
        format!("{}/int/ev/room/{room}", self.c)
    }

    pub fn ev_user(&self, user: &str) -> String {
        format!("{}/int/ev/user/{user}", self.c)
    }

    pub fn ev_node(&self, node: &str) -> String {
        format!("{}/int/ev/node/{node}", self.c)
    }

    pub fn q_node(&self, node: &str) -> String {
        format!("{}/int/q/node/{node}", self.c)
    }

    pub fn q_node_sub(&self, node: &str) -> String {
        format!("{}/int/q/node/{node}/**", self.c)
    }

    pub fn q_node_sess(&self, node: &str) -> String {
        format!("{}/int/q/node/{node}/sess", self.c)
    }

    pub fn prefix(&self) -> &str {
        &self.c
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Node { node: String, inst: String },
    Room { room: String, node: String, inst: String },
    Sess { user: String, node: String, sid: String },
}

pub fn parse_token(namespace: &str, key: &str) -> Option<Token> {
    let rest = key.strip_prefix(namespace)?.strip_prefix("/int/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        ["node", node, inst] => Some(Token::Node { node: (*node).into(), inst: (*inst).into() }),
        ["room", room, node, inst] => Some(Token::Room { room: (*room).into(), node: (*node).into(), inst: (*inst).into() }),
        ["sess", user, node, sid] => Some(Token::Sess { user: (*user).into(), node: (*node).into(), sid: (*sid).into() }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let k = Keys::new("oxdev");
        assert_eq!(
            parse_token("oxdev", &k.room("r1", "node-a", "i1")),
            Some(Token::Room { room: "r1".into(), node: "node-a".into(), inst: "i1".into() })
        );
        assert!(!valid_piece("a/b"));
        assert!(!valid_piece(""));
        assert!(valid_piece("qa_room-1"));
    }
}
