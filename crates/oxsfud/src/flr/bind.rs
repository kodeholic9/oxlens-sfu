// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-LOCK · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 미디어 결합(순수) — 효과가 판정 번호(`grant_no`)를 든다. 결합은 본 최댓값보다 클 때만 · 해제는 지금 결합의 번호일 때만 비운다 · 본 최댓값은 늘 올린다. 버린 효과는 곁 동작을 하지 않는다.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Bind(String, u64),
    Release(u64),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub bound: Option<(String, u64)>,
    pub max_seen: Option<u64>,
}

impl Slot {
    pub fn new() -> Slot {
        Slot::default()
    }

    fn newer(&self, g: u64) -> bool {
        self.max_seen.is_none_or(|m| g > m)
    }

    fn see(&mut self, g: u64) {
        self.max_seen = Some(self.max_seen.map_or(g, |m| m.max(g)));
    }

    pub fn apply(&mut self, e: &Effect) -> bool {
        match e {
            Effect::Bind(u, g) => {
                let ok = self.newer(*g);
                if ok {
                    self.bound = Some((u.clone(), *g));
                }
                self.see(*g);
                ok
            }
            Effect::Release(g) => {
                let ok = self.bound.as_ref().is_some_and(|(_, cur)| cur == g);
                if ok {
                    self.bound = None;
                }
                self.see(*g);
                ok
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_release_does_not_clear_new_bind() {
        let mut s = Slot::new();
        assert!(s.apply(&Effect::Bind("A".into(), 0)));
        assert!(s.apply(&Effect::Bind("B".into(), 1)));
        assert!(!s.apply(&Effect::Release(0)), "S-sym-IMPL-FLR-MEDIA-BIND-3 남의 결합을 비우지 않는다");
        assert_eq!(s.bound, Some(("B".into(), 1)));
    }

    #[test]
    fn late_bind_after_release_is_dropped() {
        let mut s = Slot::new();
        assert!(!s.apply(&Effect::Release(0)));
        assert!(!s.apply(&Effect::Bind("A".into(), 0)), "S-sym-IMPL-FLR-MEDIA-BIND-4 늦게 닿은 옛 결합은 버린다");
        assert_eq!(s.bound, None);
    }
}
