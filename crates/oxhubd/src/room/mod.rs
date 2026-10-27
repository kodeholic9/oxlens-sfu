// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:IMPL-BUS-FAULTS · 정:RULE-BUS-REQUEST · 정:RULE-BUS-PLACEMENT · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-JOIN · 운영:RULE-READ-BUS · model: claude-opus-5-5
//! 게이트웨이의 방 — 방 → node 맵(`room` 토큰에서 배운다 · 소멸은 가라앉힌 뒤 몰아 판정 · 고립이면 거두지 않는다) · `ev/room` 관심 참조계수.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use common::bus::SETTLE_MS;
use common::bus::keys::{NodeToken, RoomToken};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoomLoc {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEv {
    RoomPut(RoomToken),
    RoomDel(RoomToken),
    NodePut(NodeToken),
    NodeDel(NodeToken),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastEvent {
    pub at_ms: u64,
    pub put: bool,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settled {
    pub removed: Vec<String>,
    pub isolated: bool,
}

#[derive(Debug, Default)]
pub struct RoomMap {
    me: String,
    rooms: BTreeMap<String, BTreeSet<RoomLoc>>,
    nodes: BTreeMap<String, BTreeSet<String>>,
    seen_others: bool,
    pending: Vec<TokenEv>,
    last_ms: u64,
    isolated: Option<(u64, BTreeSet<String>)>,
    pub last_event: Option<LastEvent>,
    pub duplicates: u64,
}

impl RoomMap {
    pub fn new(me: &str) -> RoomMap {
        RoomMap { me: me.to_string(), ..RoomMap::default() }
    }

    pub fn on_event(&mut self, ev: TokenEv, key: &str, now_ms: u64, wall_ms: u64) {
        self.last_ms = now_ms;
        let put = matches!(ev, TokenEv::RoomPut(_) | TokenEv::NodePut(_));
        self.last_event = Some(LastEvent { at_ms: wall_ms, put, key: key.to_string() });
        match ev {
            TokenEv::RoomPut(t) => {
                self.pending.retain(|p| !matches!(p, TokenEv::RoomDel(d) if d.room == t.room && d.node == t.node && d.inst == t.inst));
                let locs = self.rooms.entry(t.room.clone()).or_default();
                locs.insert(RoomLoc { node: t.node.clone(), inst: t.inst.clone() });
                let nodes: BTreeSet<&String> = locs.iter().map(|l| &l.node).collect();
                if nodes.len() > 1 {
                    self.duplicates += 1;
                }
                if let Some((_, held)) = self.isolated.as_mut() {
                    held.remove(&t.room);
                }
            }
            TokenEv::NodePut(t) => {
                if t.node != self.me {
                    self.seen_others = true;
                }
                self.pending.retain(|p| !matches!(p, TokenEv::NodeDel(d) if d.node == t.node && d.inst == t.inst));
                self.nodes.entry(t.node).or_default().insert(t.inst);
            }
            other => self.pending.push(other),
        }
    }

    fn others_alive(&self) -> bool {
        self.nodes.iter().any(|(n, insts)| *n != self.me && !insts.is_empty())
    }

    pub fn settle_due(&self, now_ms: u64) -> bool {
        (!self.pending.is_empty() || (self.isolated.is_some() && self.others_alive())) && now_ms.saturating_sub(self.last_ms) >= SETTLE_MS
    }

    pub fn settle(&mut self, now_ms: u64, wall_ms: u64) -> Settled {
        if !self.settle_due(now_ms) {
            return Settled { removed: Vec::new(), isolated: self.isolated.is_some() };
        }
        let batch = std::mem::take(&mut self.pending);
        let mut other_node_gone = false;
        for ev in &batch {
            if let TokenEv::NodeDel(t) = ev {
                if let Some(s) = self.nodes.get_mut(&t.node) {
                    s.remove(&t.inst);
                    if s.is_empty() {
                        self.nodes.remove(&t.node);
                    }
                }
                if t.node != self.me {
                    other_node_gone = true;
                }
            }
        }
        let isolated_now = self.seen_others && other_node_gone && !self.others_alive();
        let mut removed = Vec::new();
        for ev in batch {
            let TokenEv::RoomDel(t) = ev else { continue };
            let mine = t.node == self.me;
            if isolated_now && !mine {
                let held = &mut self.isolated.get_or_insert_with(|| (wall_ms, BTreeSet::new())).1;
                held.insert(t.room.clone());
                continue;
            }
            if self.remove_loc(&t) {
                removed.push(t.room.clone());
            }
        }
        if isolated_now {
            self.isolated.get_or_insert_with(|| (wall_ms, BTreeSet::new()));
        } else if self.others_alive()
            && let Some((_, held)) = self.isolated.take()
        {
            for room in held {
                if self.rooms.remove(&room).is_some() {
                    removed.push(room);
                }
            }
        }
        removed.sort();
        removed.dedup();
        Settled { removed, isolated: self.isolated.is_some() }
    }

    fn remove_loc(&mut self, t: &RoomToken) -> bool {
        let Some(locs) = self.rooms.get_mut(&t.room) else { return false };
        locs.remove(&RoomLoc { node: t.node.clone(), inst: t.inst.clone() });
        if locs.is_empty() {
            self.rooms.remove(&t.room);
            return true;
        }
        false
    }

    pub fn locate(&self, room: &str) -> Option<RoomLoc> {
        self.rooms.get(room).and_then(|l| l.iter().next().cloned())
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<String> {
        self.rooms.values().flat_map(|l| l.iter()).find(|l| l.inst == inst).map(|l| l.node.clone())
    }

    pub fn count(&self) -> usize {
        self.rooms.len()
    }

    pub fn entries(&self) -> Vec<(String, RoomLoc)> {
        self.rooms.iter().filter_map(|(r, l)| l.iter().next().map(|x| (r.clone(), x.clone()))).collect()
    }

    pub fn nodes(&self) -> Vec<(String, String)> {
        self.nodes.iter().flat_map(|(n, s)| s.iter().map(move |i| (n.clone(), i.clone()))).collect()
    }

    pub fn isolated(&self) -> Option<(u64, Vec<String>)> {
        self.isolated.as_ref().map(|(t, h)| (*t, h.iter().cloned().collect()))
    }
}

#[derive(Debug, Default)]
pub struct Interest {
    counts: HashMap<String, u32>,
}

impl Interest {
    pub fn inc(&mut self, room: &str) -> bool {
        let old = self.counts.get(room).copied().unwrap_or(0);
        self.counts.insert(room.to_string(), old.saturating_add(1));
        old == 0
    }

    pub fn dec(&mut self, room: &str) -> bool {
        let Some(c) = self.counts.get(room).copied() else { return false };
        if c <= 1 {
            self.counts.remove(room);
            return true;
        }
        self.counts.insert(room.to_string(), c - 1);
        false
    }

    pub fn held(&self, room: &str) -> bool {
        self.counts.contains_key(room)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(room: &str, node: &str, inst: &str) -> RoomToken {
        RoomToken { room: room.into(), node: node.into(), inst: inst.into() }
    }

    fn nt(node: &str, inst: &str) -> NodeToken {
        NodeToken { node: node.into(), inst: inst.into() }
    }

    #[test]
    fn room_delete_waits_for_settle() {
        let mut m = RoomMap::new("a");
        m.on_event(TokenEv::RoomPut(rt("r", "b", "e")), "k", 0, 0);
        m.on_event(TokenEv::RoomDel(rt("r", "b", "e")), "k", 100, 100);
        assert!(m.locate("r").is_some());
        assert!(m.settle(500, 500).removed.is_empty());
        assert_eq!(m.settle(1_100, 1_100).removed, vec!["r".to_string()]);
        assert!(m.locate("r").is_none());
    }

    #[test]
    fn batch_loss_of_all_others_is_isolation_and_keeps_routing() {
        let mut m = RoomMap::new("a");
        m.on_event(TokenEv::NodePut(nt("a", "h1")), "k", 0, 0);
        m.on_event(TokenEv::NodePut(nt("b", "h2")), "k", 0, 0);
        m.on_event(TokenEv::RoomPut(rt("rb", "b", "e2")), "k", 0, 0);
        m.on_event(TokenEv::RoomPut(rt("ra", "a", "e1")), "k", 0, 0);
        m.on_event(TokenEv::RoomDel(rt("rb", "b", "e2")), "k", 10, 10);
        m.on_event(TokenEv::NodeDel(nt("b", "h2")), "k", 20, 20);
        m.on_event(TokenEv::RoomDel(rt("ra", "a", "e1")), "k", 30, 30);
        let s = m.settle(2_000, 2_000);
        assert!(s.isolated);
        assert_eq!(s.removed, vec!["ra".to_string()]);
        assert!(m.locate("rb").is_some());
        assert_eq!(m.isolated().map(|x| x.1), Some(vec!["rb".to_string()]));
        m.on_event(TokenEv::NodePut(nt("b", "h3")), "k", 3_000, 3_000);
        m.on_event(TokenEv::NodeDel(nt("b", "none")), "k", 3_000, 3_000);
        let s = m.settle(5_000, 5_000);
        assert!(!s.isolated);
        assert_eq!(s.removed, vec!["rb".to_string()]);
    }

    #[test]
    fn solo_node_is_not_isolation() {
        let mut m = RoomMap::new("a");
        m.on_event(TokenEv::NodePut(nt("a", "h1")), "k", 0, 0);
        m.on_event(TokenEv::RoomPut(rt("ra", "a", "e1")), "k", 0, 0);
        m.on_event(TokenEv::RoomDel(rt("ra", "a", "e1")), "k", 10, 10);
        let s = m.settle(2_000, 2_000);
        assert!(!s.isolated);
        assert_eq!(s.removed, vec!["ra".to_string()]);
    }

    #[test]
    fn duplicate_placement_is_counted() {
        let mut m = RoomMap::new("a");
        m.on_event(TokenEv::RoomPut(rt("r", "a", "e1")), "k", 0, 0);
        m.on_event(TokenEv::RoomPut(rt("r", "b", "e2")), "k", 0, 0);
        assert_eq!(m.duplicates, 1);
    }

    #[test]
    fn interest_counts_first_and_last() {
        let mut i = Interest::default();
        assert!(i.inc("r"));
        assert!(!i.inc("r"));
        assert!(!i.dec("r"));
        assert!(i.dec("r"));
        assert!(!i.held("r"));
    }
}
