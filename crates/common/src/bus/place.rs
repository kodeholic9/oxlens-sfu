// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-PLACEMENT · 정:RULE-OPS-SURFACE · 정:RULE-DOC-COND-INDEX-SERVER · model: claude-opus-5-5
//! 배치 — FNV-1a 64 → splitmix64 마무리 · HRW 순수 함수 · 후보 집합(자기 node 는 처음부터 · 소멸은 grace 동안 남긴다).

use std::collections::BTreeMap;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn hash(node_id: &str, room_id: &str) -> u64 {
    let mut z = FNV_OFFSET;
    for b in node_id.as_bytes().iter().chain(std::iter::once(&0u8)).chain(room_id.as_bytes()) {
        z ^= u64::from(*b);
        z = z.wrapping_mul(FNV_PRIME);
    }
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z
}

pub fn pick<'a, I>(nodes: I, room_id: &str) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(u64, &str)> = None;
    for n in nodes {
        let h = hash(n, room_id);
        best = match best {
            None => Some((h, n)),
            Some((bh, bn)) if h > bh || (h == bh && n < bn) => Some((h, n)),
            keep => keep,
        };
    }
    best.map(|(_, n)| n.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct Candidates {
    me: String,
    seen: BTreeMap<String, NodeSeen>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NodeSeen {
    tokens: u32,
    gone_at_ms: Option<u64>,
}

impl Candidates {
    pub fn new(me: &str) -> Candidates {
        Candidates { me: me.to_string(), seen: BTreeMap::new() }
    }

    pub fn me(&self) -> &str {
        &self.me
    }

    pub fn token_put(&mut self, node: &str) {
        let e = self.seen.entry(node.to_string()).or_insert(NodeSeen { tokens: 0, gone_at_ms: None });
        e.tokens += 1;
        e.gone_at_ms = None;
    }

    pub fn token_delete(&mut self, node: &str, now_ms: u64) {
        if let Some(e) = self.seen.get_mut(node) {
            e.tokens = e.tokens.saturating_sub(1);
            if e.tokens == 0 {
                e.gone_at_ms = Some(now_ms);
            }
        }
    }

    pub fn live(&self, node: &str) -> bool {
        self.seen.get(node).is_some_and(|e| e.tokens > 0)
    }

    pub fn expire(&mut self, now_ms: u64, grace_ms: u64) -> Vec<String> {
        let gone: Vec<String> =
            self.seen.iter().filter(|(_, e)| e.tokens == 0 && e.gone_at_ms.is_some_and(|t| now_ms.saturating_sub(t) >= grace_ms)).map(|(n, _)| n.clone()).collect();
        for n in &gone {
            self.seen.remove(n);
        }
        gone
    }

    pub fn set(&self) -> Vec<String> {
        let mut v: Vec<String> = self.seen.keys().cloned().collect();
        if !v.iter().any(|n| n == &self.me) {
            v.push(self.me.clone());
        }
        v.sort();
        v
    }

    pub fn place(&self, room_id: &str) -> Option<String> {
        let set = self.set();
        pick(set.iter().map(String::as_str), room_id)
    }

    pub fn others_seen(&self) -> usize {
        self.seen.keys().filter(|n| **n != self.me).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_the_contract_formula() {
        let mut z: u64 = 0xcbf29ce484222325;
        for b in b"node-a\0room-1" {
            z ^= u64::from(*b);
            z = z.wrapping_mul(0x100000001b3);
        }
        z = z.wrapping_add(0x9e3779b97f4a7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        assert_eq!(hash("node-a", "room-1"), z);
    }

    #[test]
    fn hrw_ties_break_on_node_id_and_pick_is_stable() {
        let nodes = ["node-c", "node-a", "node-b"];
        let p1 = pick(nodes.iter().copied(), "r42");
        let p2 = pick(["node-b", "node-c", "node-a"].iter().copied(), "r42");
        assert_eq!(p1, p2);
        assert_eq!(pick(std::iter::empty(), "r"), None);
    }

    #[test]
    fn self_node_is_candidate_before_its_echo() {
        let c = Candidates::new("node-a");
        assert_eq!(c.set(), vec!["node-a".to_string()], "S-sym-RULE-BUS-PLACEMENT-6 자기 node 가 후보에 든다");
        assert_eq!(c.place("room-1").as_deref(), Some("node-a"), "S-sym-RULE-BUS-PLACEMENT-6 한 대 형상이면 배치가 선다");
    }

    #[test]
    fn deleted_node_stays_until_grace() {
        let mut c = Candidates::new("node-a");
        c.token_put("node-a");
        c.token_put("node-b");
        c.token_delete("node-b", 1_000);
        assert!(!c.live("node-b"));
        assert!(c.set().contains(&"node-b".to_string()));
        assert!(c.expire(20_000, 30_000).is_empty());
        assert_eq!(c.expire(31_000, 30_000), vec!["node-b".to_string()]);
        assert_eq!(c.set(), vec!["node-a".to_string()]);
        c.token_put("node-b");
        assert!(c.live("node-b"));
    }
}
