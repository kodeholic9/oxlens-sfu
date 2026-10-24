// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-BUS-HOME-LOST · 정:CAND-BUS-HOME-INST · 정:RULE-BUS-PRESENCE · model: claude-opus-5-5
//! 홈 판정(순수) — node 토큰 집합을 보고 사라진 홈을 3s 뒤 판정한다: 같은 node 의 새 inst 면 옮겨 달고 · 없으면 거둔다 · 고립(타 node 일괄 소멸)이거나 소멸 전 node ≤ 2 면 거두지 않는다.

use std::collections::BTreeMap;

pub const HOME_JUDGE_MS: u64 = 3_000;
pub const BULK_MS: u64 = 1_000;

#[derive(Debug, Clone, Default)]
pub struct Nodes {
    pub me: String,
    pub live: BTreeMap<String, String>,
    gone: Vec<(String, String, u64, usize)>,
    recent: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Rehome { node: String, from: String, to: String },
    Reap { node: String, inst: String },
    Keep { node: String, inst: String, why: &'static str },
}

impl Nodes {
    pub fn new(me: &str) -> Nodes {
        Nodes { me: me.to_string(), ..Nodes::default() }
    }

    pub fn put(&mut self, node: &str, inst: &str) {
        self.live.insert(node.to_string(), inst.to_string());
    }

    pub fn delete(&mut self, node: &str, inst: &str, now_ms: u64) {
        if self.live.get(node).map(|i| i == inst).unwrap_or(false) {
            let before = self.live.len() + usize::from(!self.live.contains_key(&self.me));
            self.live.remove(node);
            self.recent.retain(|t| now_ms.saturating_sub(*t) <= BULK_MS);
            self.recent.push(now_ms);
            self.gone.push((node.to_string(), inst.to_string(), now_ms, before));
        }
    }

    fn isolated(&self) -> bool {
        !self.live.keys().any(|n| n != &self.me) && self.recent.len() >= 2
    }

    pub fn judge(&mut self, now_ms: u64) -> Vec<Verdict> {
        let mut out = Vec::new();
        let isolated = self.isolated();
        let mut keep = Vec::new();
        for (node, inst, at, before) in std::mem::take(&mut self.gone) {
            if now_ms.saturating_sub(at) < HOME_JUDGE_MS {
                keep.push((node, inst, at, before));
                continue;
            }
            if let Some(fresh) = self.live.get(&node) {
                out.push(Verdict::Rehome { node: node.clone(), from: inst, to: fresh.clone() });
            } else if isolated {
                out.push(Verdict::Keep { node, inst, why: "isolated" });
            } else if before <= 2 {
                out.push(Verdict::Keep { node, inst, why: "two-nodes" });
            } else {
                out.push(Verdict::Reap { node, inst });
            }
        }
        self.gone = keep;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_lost_rules() {
        let mut n = Nodes::new("b");
        for (k, i) in [("a", "a1"), ("b", "b1"), ("c", "c1")] {
            n.put(k, i);
        }
        n.delete("a", "a1", 0);
        assert!(n.judge(1000).is_empty(), "3s 뒤 판정");
        assert_eq!(n.judge(3000), vec![Verdict::Reap { node: "a".into(), inst: "a1".into() }], "S-sym-IMPL-BUS-HOME-LOST-1 홈을 잃은 Peer 를 거둔다");
        n.delete("c", "c1", 5000);
        n.put("c", "c2");
        assert_eq!(n.judge(9000), vec![Verdict::Rehome { node: "c".into(), from: "c1".into(), to: "c2".into() }]);
        let mut two = Nodes::new("b");
        two.put("a", "a1");
        two.put("b", "b1");
        two.delete("a", "a1", 0);
        assert!(matches!(two.judge(3000)[0], Verdict::Keep { why: "two-nodes", .. }));
    }
}
