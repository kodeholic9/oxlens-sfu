// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-PLACEMENT · 정:IMPL-BUS-FAULTS · 정:RULE-OPS-SURFACE · model: claude-opus-5-5
//! 버스 판정(순수) — node 집합(소멸은 grace 동안 후보에 남는다) · 방 → node 지도(토큰이 유일한 입력) · 방 중복 · 자기 고립.

use std::collections::{BTreeMap, BTreeSet};

pub const BULK_WINDOW_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeEntry {
    pub inst: String,
    pub alive: bool,
    pub gone_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct NodeSet {
    pub me: String,
    pub nodes: BTreeMap<String, NodeEntry>,
    recent_gone: Vec<(String, u64)>,
    pub isolated: bool,
    pub last_event: Option<(u64, String, String)>,
}

impl NodeSet {
    pub fn new(me: &str) -> NodeSet {
        NodeSet { me: me.to_string(), ..NodeSet::default() }
    }

    pub fn put(&mut self, node: &str, inst: &str, now_ms: u64) {
        self.nodes.insert(node.to_string(), NodeEntry { inst: inst.to_string(), alive: true, gone_at_ms: 0 });
        if node != self.me {
            self.isolated = false;
        }
        self.last_event = Some((now_ms, "put".into(), format!("{node}/{inst}")));
    }

    pub fn delete(&mut self, node: &str, inst: &str, now_ms: u64) -> bool {
        self.last_event = Some((now_ms, "delete".into(), format!("{node}/{inst}")));
        let Some(e) = self.nodes.get_mut(node) else { return false };
        if e.inst != inst || !e.alive {
            return false;
        }
        e.alive = false;
        e.gone_at_ms = now_ms;
        if node != self.me {
            self.recent_gone.retain(|(_, t)| now_ms.saturating_sub(*t) <= BULK_WINDOW_MS);
            self.recent_gone.push((node.to_string(), now_ms));
            let others_alive = self.nodes.iter().any(|(n, e)| n != &self.me && e.alive);
            if !others_alive && self.recent_gone.len() >= 2 {
                self.isolated = true;
            }
        }
        true
    }

    pub fn live(&self, node: &str) -> bool {
        self.nodes.get(node).map(|e| e.alive).unwrap_or(false)
    }

    pub fn inst_of(&self, node: &str) -> Option<&str> {
        self.nodes.get(node).filter(|e| e.alive).map(|e| e.inst.as_str())
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<&str> {
        self.nodes.iter().find(|(_, e)| e.alive && e.inst == inst).map(|(n, _)| n.as_str())
    }

    pub fn candidates(&self, now_ms: u64, grace_ms: u64) -> Vec<&str> {
        self.nodes.iter().filter(|(_, e)| e.alive || now_ms.saturating_sub(e.gone_at_ms) < grace_ms).map(|(n, _)| n.as_str()).collect()
    }

    pub fn alive_nodes(&self) -> Vec<String> {
        self.nodes.iter().filter(|(_, e)| e.alive).map(|(n, _)| n.clone()).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RoomMap {
    pub rooms: BTreeMap<String, BTreeSet<(String, String)>>,
    pub duplicates_seen: u64,
}

impl RoomMap {
    pub fn put(&mut self, room: &str, node: &str, inst: &str) -> bool {
        let set = self.rooms.entry(room.to_string()).or_default();
        set.insert((node.to_string(), inst.to_string()));
        let nodes: BTreeSet<&String> = set.iter().map(|(n, _)| n).collect();
        if nodes.len() > 1 {
            self.duplicates_seen += 1;
            return true;
        }
        false
    }

    pub fn delete(&mut self, room: &str, node: &str, inst: &str) {
        if let Some(set) = self.rooms.get_mut(room) {
            set.remove(&(node.to_string(), inst.to_string()));
            if set.is_empty() {
                self.rooms.remove(room);
            }
        }
    }

    pub fn node_of(&self, room: &str) -> Option<&str> {
        self.rooms.get(room).and_then(|s| s.iter().next()).map(|(n, _)| n.as_str())
    }

    pub fn duplicates_now(&self) -> Vec<String> {
        self.rooms
            .iter()
            .filter(|(_, s)| s.iter().map(|(n, _)| n).collect::<BTreeSet<_>>().len() > 1)
            .map(|(r, _)| r.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rooms.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_keeps_candidate_and_delete_never_relocates() {
        let mut s = NodeSet::new("a");
        s.put("a", "ia", 0);
        s.put("b", "ib", 0);
        assert!(s.delete("b", "ib", 100));
        assert!(!s.live("b"));
        assert_eq!(s.candidates(200, 30_000), vec!["a", "b"], "S-sym-RULE-BUS-PLACEMENT-1 소멸한 노드도 grace 동안 후보");
        assert_eq!(s.candidates(40_000, 30_000), vec!["a"], "S-sym-RULE-BUS-PLACEMENT-3 grace 뒤에만 빠진다");
        assert!(!s.isolated, "노드 하나의 소멸은 고립이 아니다");
    }

    #[test]
    fn bulk_loss_is_isolation_and_old_inst_delete_is_ignored() {
        let mut s = NodeSet::new("a");
        s.put("a", "ia", 0);
        s.put("b", "ib", 0);
        s.put("c", "ic", 0);
        s.put("b", "ib2", 5);
        assert!(!s.delete("b", "ib", 10), "S-sym-RULE-BUS-KEYSPACE-1 옛 소멸이 새 선언을 덮지 않는다");
        s.delete("b", "ib2", 20);
        s.delete("c", "ic", 30);
        assert!(s.isolated);
        s.put("b", "ib3", 40);
        assert!(!s.isolated);
    }

    #[test]
    fn room_duplicates_are_counted() {
        let mut m = RoomMap::default();
        assert!(!m.put("r", "a", "1"));
        assert!(m.put("r", "b", "2"), "S-sym-RULE-BUS-PLACEMENT-5 방 하나에 node 둘을 탐지한다");
        assert_eq!(m.duplicates_now(), vec!["r".to_string()]);
        m.delete("r", "b", "2");
        assert_eq!(m.node_of("r"), Some("a"));
    }
}
