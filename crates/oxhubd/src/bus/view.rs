// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-EVENTS · 정:RULE-BUS-PLACEMENT · 정:IMPL-BUS-FAULTS · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! hub 가 버스에서 본 것 — node 토큰 · 방 토큰 · 방 → node 맵 · 고립 판정 · 가라앉힌 뒤 몰아 판정(시계는 인자).

use std::collections::{BTreeMap, BTreeSet};

pub const SETTLE_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEv {
    NodePut { node: String, inst: String },
    NodeDel { node: String, inst: String },
    RoomPut { room: String, node: String, inst: String },
    RoomDel { room: String, node: String, inst: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSeen {
    pub inst: String,
    pub live: bool,
    pub gone_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    pub node: String,
    pub inst: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastEvent {
    pub at_ms: u64,
    pub kind: &'static str,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct View {
    pub self_node: String,
    pub self_inst: String,
    pub self_declared: bool,
    pub grace_ms: u64,
    pub nodes: BTreeMap<String, NodeSeen>,
    pub routing: BTreeMap<String, Loc>,
    pub seen_other: bool,
    pub isolated: Option<u64>,
    pub held: BTreeSet<String>,
    pending: Vec<TokenEv>,
    last_token_at: u64,
    pub last_event: Option<LastEvent>,
    pub duplicates: u64,
    pub recovered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settled {
    pub dropped_rooms: Vec<String>,
    pub released: Vec<String>,
    pub isolated_now: bool,
}

impl View {
    pub fn new(self_node: &str, self_inst: &str, grace_ms: u64) -> View {
        View {
            self_node: self_node.to_string(),
            self_inst: self_inst.to_string(),
            self_declared: false,
            grace_ms,
            nodes: BTreeMap::new(),
            routing: BTreeMap::new(),
            seen_other: false,
            isolated: None,
            held: BTreeSet::new(),
            pending: Vec::new(),
            last_token_at: 0,
            last_event: None,
            duplicates: 0,
            recovered: false,
        }
    }

    pub fn on_token(&mut self, ev: TokenEv, now: u64) {
        self.last_token_at = now;
        let (kind, key) = match &ev {
            TokenEv::NodePut { node, inst } => ("put", format!("node/{node}/{inst}")),
            TokenEv::NodeDel { node, inst } => ("delete", format!("node/{node}/{inst}")),
            TokenEv::RoomPut { room, node, inst } => ("put", format!("room/{room}/{node}/{inst}")),
            TokenEv::RoomDel { room, node, inst } => ("delete", format!("room/{room}/{node}/{inst}")),
        };
        self.last_event = Some(LastEvent { at_ms: now, kind, key });
        match ev {
            TokenEv::NodePut { node, inst } => {
                self.pending.retain(|p| !matches!(p, TokenEv::NodeDel { node: n, inst: i } if *n == node && *i == inst));
                if node != self.self_node {
                    self.seen_other = true;
                    if self.isolated.is_some() {
                        self.isolated = None;
                        self.recovered = true;
                    }
                }
                self.nodes.insert(node, NodeSeen { inst, live: true, gone_at: None });
            }
            TokenEv::RoomPut { room, node, inst } => {
                self.pending.retain(
                    |p| !matches!(p, TokenEv::RoomDel { room: r, node: n, inst: i } if *r == room && *n == node && *i == inst),
                );
                if let Some(old) = self.routing.get(&room)
                    && old.node != node
                {
                    self.duplicates += 1;
                }
                self.held.remove(&room);
                self.routing.insert(room, Loc { node, inst });
            }
            del => self.pending.push(del),
        }
    }

    pub fn settle_due(&self, now: u64) -> bool {
        !self.pending.is_empty() && now >= self.last_token_at + SETTLE_MS
    }

    pub fn settle(&mut self, now: u64) -> Settled {
        let mut out = Settled::default();
        if self.pending.is_empty() && self.isolated.is_none() && !self.held.is_empty() {
            out.dropped_rooms = self.release_held();
            out.released = out.dropped_rooms.clone();
            return out;
        }
        if !self.settle_due(now) {
            return out;
        }
        let batch = std::mem::take(&mut self.pending);
        let mut other_node_del = false;
        for ev in &batch {
            if let TokenEv::NodeDel { node, inst } = ev
                && let Some(n) = self.nodes.get_mut(node)
                && &n.inst == inst
            {
                n.live = false;
                n.gone_at = Some(now);
                if node != &self.self_node {
                    other_node_del = true;
                }
            }
        }
        let others_live = self.nodes.iter().any(|(k, n)| k != &self.self_node && n.live);
        if self.seen_other && other_node_del && !others_live && self.isolated.is_none() {
            self.isolated = Some(now);
            out.isolated_now = true;
        }
        for ev in &batch {
            if let TokenEv::RoomDel { room, node, inst } = ev {
                let matches = self.routing.get(room).map(|l| &l.node == node && &l.inst == inst).unwrap_or(false);
                if !matches {
                    continue;
                }
                if self.isolated.is_some() && node != &self.self_node {
                    self.held.insert(room.clone());
                    continue;
                }
                self.routing.remove(room);
                out.dropped_rooms.push(room.clone());
            }
        }
        if self.isolated.is_none() {
            let r = self.release_held();
            out.released.extend(r.iter().cloned());
            out.dropped_rooms.extend(r);
        }
        out
    }

    fn release_held(&mut self) -> Vec<String> {
        let held = std::mem::take(&mut self.held);
        let mut dropped = Vec::new();
        for room in held {
            let alive = self
                .routing
                .get(&room)
                .map(|l| self.nodes.get(&l.node).map(|n| n.live).unwrap_or(l.node == self.self_node))
                .unwrap_or(false);
            if !alive {
                self.routing.remove(&room);
                dropped.push(room);
            }
        }
        dropped
    }

    pub fn candidates(&self, now: u64) -> Vec<String> {
        let mut c: BTreeSet<String> = BTreeSet::new();
        c.insert(self.self_node.clone());
        for (k, n) in &self.nodes {
            let in_grace = n.gone_at.map(|g| now < g + self.grace_ms).unwrap_or(false);
            if n.live || in_grace {
                c.insert(k.clone());
            }
        }
        c.into_iter().collect()
    }

    pub fn node_live(&self, node: &str) -> bool {
        if node == self.self_node {
            return self.self_declared;
        }
        self.nodes.get(node).map(|n| n.live).unwrap_or(false)
    }

    pub fn room_node(&self, room: &str) -> Option<&Loc> {
        self.routing.get(room)
    }

    pub fn node_of_inst(&self, inst: &str) -> Option<String> {
        self.routing.values().find(|l| l.inst == inst).map(|l| l.node.clone())
    }

    pub fn live_nodes(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> =
            self.nodes.iter().filter(|(_, n)| n.live).map(|(k, n)| (k.clone(), n.inst.clone())).collect();
        if self.self_declared && !v.iter().any(|(k, _)| k == &self.self_node) {
            v.push((self.self_node.clone(), self.self_inst.clone()));
        }
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rp(room: &str, node: &str) -> TokenEv {
        TokenEv::RoomPut { room: room.into(), node: node.into(), inst: format!("{node}-i") }
    }
    fn rd(room: &str, node: &str) -> TokenEv {
        TokenEv::RoomDel { room: room.into(), node: node.into(), inst: format!("{node}-i") }
    }
    fn np(node: &str) -> TokenEv {
        TokenEv::NodePut { node: node.into(), inst: format!("{node}-h") }
    }
    fn nd(node: &str) -> TokenEv {
        TokenEv::NodeDel { node: node.into(), inst: format!("{node}-h") }
    }

    #[test]
    fn self_in_candidates_before_echo() {
        let v = View::new("a", "ai", 30_000);
        assert_eq!(v.candidates(0), vec!["a".to_string()]);
    }

    #[test]
    fn delete_waits_settle_and_put_cancels() {
        let mut v = View::new("a", "ai", 30_000);
        v.on_token(np("b"), 0);
        v.on_token(rp("r1", "b"), 0);
        v.on_token(rd("r1", "b"), 100);
        assert!(v.settle(500).dropped_rooms.is_empty());
        assert!(v.room_node("r1").is_some());
        v.on_token(rp("r1", "b"), 600);
        assert!(v.settle(5_000).dropped_rooms.is_empty());
        assert!(v.room_node("r1").is_some());
        v.on_token(rd("r1", "b"), 6_000);
        assert_eq!(v.settle(7_000).dropped_rooms, vec!["r1".to_string()]);
    }

    #[test]
    fn batch_of_other_deletes_is_isolation_and_holds_rooms() {
        let mut v = View::new("a", "ai", 30_000);
        v.on_token(np("b"), 0);
        v.on_token(np("c"), 0);
        v.on_token(rp("rb", "b"), 0);
        v.on_token(rp("ra", "a"), 0);
        v.on_token(rd("rb", "b"), 10);
        v.on_token(nd("b"), 20);
        v.on_token(nd("c"), 25);
        v.on_token(rd("ra", "a"), 30);
        let s = v.settle(2_000);
        assert!(s.isolated_now);
        assert_eq!(s.dropped_rooms, vec!["ra".to_string()]);
        assert!(v.held.contains("rb"));
        assert!(v.candidates(2_000).contains(&"b".to_string()));
        assert!(!v.candidates(40_000).contains(&"b".to_string()));
    }

    #[test]
    fn solo_node_is_not_isolated() {
        let mut v = View::new("a", "ai", 30_000);
        v.on_token(rp("ra", "a"), 0);
        v.on_token(rd("ra", "a"), 10);
        let s = v.settle(2_000);
        assert!(!s.isolated_now);
        assert_eq!(s.dropped_rooms, vec!["ra".to_string()]);
    }
}
