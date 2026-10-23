// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-BUS-PLACEMENT · 정:RULE-BUS-PRESENCE · 정:IMPL-BUS-FAULTS · 정:RULE-BUS-EVENTS · model: claude-opus-5-5
//! BUS 판정(순수) — 배치 해시(FNV-1a 64 → splitmix64) · HRW · 노드 집합(grace) · 방 → node 맵 · 관심 선언 참조계수 · 고립 판정.

use std::collections::{BTreeMap, BTreeSet};

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn splitmix64(z: u64) -> u64 {
    let mut z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

pub fn score(node_id: &str, room_id: &str) -> u64 {
    let mut b = Vec::with_capacity(node_id.len() + 1 + room_id.len());
    b.extend_from_slice(node_id.as_bytes());
    b.push(0);
    b.extend_from_slice(room_id.as_bytes());
    splitmix64(fnv1a64(&b))
}

pub fn place<'a>(nodes: impl IntoIterator<Item = &'a String>, room_id: &str) -> Option<String> {
    let mut best: Option<(u64, &String)> = None;
    for n in nodes {
        let s = score(n, room_id);
        best = match best {
            None => Some((s, n)),
            Some((bs, bn)) if s > bs || (s == bs && n < bn) => Some((s, n)),
            keep => keep,
        };
    }
    best.map(|(_, n)| n.clone())
}

#[derive(Clone, Debug, Default)]
pub struct NodeSet {
    live: BTreeMap<String, String>,
    gone_at: BTreeMap<String, u64>,
}

impl NodeSet {
    pub fn put(&mut self, node: &str, inst: &str) {
        self.live.insert(node.to_string(), inst.to_string());
        self.gone_at.remove(node);
    }

    pub fn delete(&mut self, node: &str, inst: &str, now_ms: u64) {
        if self.live.get(node).map(|i| i == inst).unwrap_or(false) {
            self.live.remove(node);
            self.gone_at.insert(node.to_string(), now_ms);
        }
    }

    pub fn candidates(&mut self, now_ms: u64, grace_ms: u64) -> BTreeSet<String> {
        self.gone_at.retain(|_, at| now_ms.saturating_sub(*at) < grace_ms);
        self.live.keys().chain(self.gone_at.keys()).cloned().collect()
    }

    pub fn is_live(&self, node: &str) -> bool {
        self.live.contains_key(node)
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<&str> {
        self.live.iter().find(|(_, i)| i.as_str() == inst).map(|(n, _)| n.as_str())
    }

    pub fn live(&self) -> &BTreeMap<String, String> {
        &self.live
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub node: String,
    pub inst: String,
}

#[derive(Clone, Debug, Default)]
pub struct RoomMap {
    rooms: BTreeMap<String, BTreeMap<String, String>>,
    pub duplicates: u64,
}

impl RoomMap {
    pub fn put(&mut self, room: &str, node: &str, inst: &str) -> bool {
        let e = self.rooms.entry(room.to_string()).or_default();
        e.insert(node.to_string(), inst.to_string());
        let dup = e.len() > 1;
        if dup {
            self.duplicates += 1;
        }
        dup
    }

    pub fn delete(&mut self, room: &str, node: &str, inst: &str) {
        if let Some(e) = self.rooms.get_mut(room) {
            if e.get(node).map(|i| i == inst).unwrap_or(false) {
                e.remove(node);
            }
            if e.is_empty() {
                self.rooms.remove(room);
            }
        }
    }

    pub fn get(&self, room: &str) -> Option<Placed> {
        self.rooms.get(room).and_then(|e| e.iter().next()).map(|(n, i)| Placed { node: n.clone(), inst: i.clone() })
    }

    pub fn all(&self) -> Vec<(String, Placed)> {
        self.rooms
            .iter()
            .filter_map(|(r, e)| e.iter().next().map(|(n, i)| (r.clone(), Placed { node: n.clone(), inst: i.clone() })))
            .collect()
    }

    pub fn of_node(&self, node: &str) -> Vec<String> {
        self.rooms.iter().filter(|(_, e)| e.contains_key(node)).map(|(r, _)| r.clone()).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Interest {
    count: BTreeMap<String, u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Declare,
    Undeclare,
    Same,
}

impl Interest {
    pub fn add(&mut self, key: &str) -> Edge {
        let c = self.count.entry(key.to_string()).or_insert(0);
        let next = c.saturating_add(1);
        c.clone_from(&next);
        if next == 1 { Edge::Declare } else { Edge::Same }
    }

    pub fn remove(&mut self, key: &str) -> Edge {
        let cur = self.count.get(key).copied();
        match cur {
            Some(1) => {
                self.count.remove(key);
                Edge::Undeclare
            }
            Some(n) => {
                self.count.insert(key.to_string(), n.saturating_sub(1));
                Edge::Same
            }
            None => Edge::Same,
        }
    }
}

pub fn isolated(remote_before: usize, remote_after: usize, deletes_in_burst: usize) -> bool {
    remote_before > 0 && remote_after == 0 && deletes_in_burst >= remote_before && remote_before >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_fnv_then_splitmix_step() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn hrw_ties_go_to_lexicographically_smaller() {
        let nodes: Vec<String> = vec!["node-a".into(), "node-b".into(), "node-c".into()];
        let r = place(&nodes, "r1");
        assert!(r.is_some());
        let one: Vec<String> = vec!["node-b".into()];
        assert_eq!(place(&one, "anything"), Some("node-b".into()));
        let moved = (0..200).filter(|i| {
            let room = format!("room-{i}");
            place(&nodes, &room) != place(&nodes[..2], &room) && place(&nodes, &room) != Some("node-c".into())
        });
        assert_eq!(moved.count(), 0);
    }

    #[test]
    fn deleted_node_stays_candidate_during_grace() {
        let mut s = NodeSet::default();
        s.put("n1", "i1");
        s.delete("n1", "i1", 1_000);
        assert!(!s.is_live("n1"));
        assert!(s.candidates(5_000, 30_000).contains("n1"));
        assert!(!s.candidates(40_000, 30_000).contains("n1"));
        s.put("n1", "i2");
        s.delete("n1", "i1", 50_000);
        assert!(s.is_live("n1"));
    }

    #[test]
    fn interest_declares_once_and_undeclares_last() {
        let mut i = Interest::default();
        assert_eq!(i.add("r"), Edge::Declare);
        assert_eq!(i.add("r"), Edge::Same);
        assert_eq!(i.remove("r"), Edge::Same);
        assert_eq!(i.remove("r"), Edge::Undeclare);
        assert_eq!(i.remove("r"), Edge::Same);
    }
}
