// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-AFF-ONE-SET 정:IMPL-AFF-PUB-ROOM 정:RULE-AFF-SERVER-SCOPE 정:RULE-CONC-SIX 연:RULE-AFF-PROC 연:RULE-AFF-PUB-IN-SUB 연:RULE-AFF-PARTIAL · model: claude-opus-5-5
//! 소속 — 입장 방 집합(= 구독 집합) 하나와 `pub_room`(RCU) 하나. 두 집합을 두지 않는다.

use std::collections::BTreeSet;
use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};
use oxsig::msg::Affiliation;

#[derive(Default)]
pub struct Aff {
    rooms: ArcSwap<BTreeSet<String>>,
    pub_room: ArcSwapOption<String>,
}

impl Aff {
    pub fn rooms(&self) -> Vec<String> {
        self.rooms.load().iter().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.rooms.load().len()
    }

    pub fn has(&self, room: &str) -> bool {
        self.rooms.load().contains(room)
    }

    pub fn add(&self, room: &str) -> bool {
        let cur = self.rooms.load();
        if cur.contains(room) {
            return false;
        }
        let mut n = (**cur).clone();
        n.insert(room.to_string());
        self.rooms.store(Arc::new(n));
        true
    }

    pub fn remove(&self, room: &str) -> bool {
        let cur = self.rooms.load();
        if !cur.contains(room) {
            return false;
        }
        let mut n = (**cur).clone();
        n.remove(room);
        self.rooms.store(Arc::new(n));
        if self.pub_room.load().as_deref().map(String::as_str) == Some(room) {
            self.pub_room.store(None);
        }
        true
    }

    pub fn pub_room(&self) -> Option<Arc<String>> {
        self.pub_room.load_full()
    }

    pub fn is_pub(&self, room: &str) -> bool {
        self.pub_room.load().as_deref().map(String::as_str) == Some(room)
    }

    pub fn set_pub(&self, room: Option<&str>) -> Option<Arc<String>> {
        self.pub_room.swap(room.map(|r| Arc::new(r.to_string())))
    }

    pub fn snapshot(&self) -> Affiliation {
        let pub_room = self.pub_room.load().as_deref().cloned().filter(|p| self.has(p));
        Affiliation { sub_rooms: self.rooms(), pub_room }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pub_is_always_in_sub() {
        let a = Aff::default();
        a.add("r1");
        a.add("r2");
        a.set_pub(Some("r1"));
        assert_eq!(a.snapshot().pub_room.as_deref(), Some("r1"));
        a.remove("r1");
        let s = a.snapshot();
        assert_eq!(s.pub_room, None, "연:RULE-AFF-PUB-IN-SUB — 듣던 방을 나가면 발행도 풀린다");
        assert_eq!(s.sub_rooms, vec!["r2".to_string()]);
    }
}
