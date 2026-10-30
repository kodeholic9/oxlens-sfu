// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-FLR-MEDIA-BIND · 정:IMPL-FLR-LOCK · model: claude-opus-5-5
//! 슬롯의 미디어 결합 — 효과가 판정 번호(`grant_no`)를 든다 · 결합은 본 최댓값보다 클 때만 · 해제 g 는 지금 결합 번호 ≤ g 면 비운다 · 버린 효과는 곁 동작도 없다.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Bind { user: String, g: u64 },
    Release { g: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub bound: Option<(String, u64)>,
    pub max_seen: Option<u64>,
}

impl Slot {
    pub fn apply(&mut self, e: &Effect) -> bool {
        match e {
            Effect::Bind { user, g } => {
                if self.max_seen.is_some_and(|m| *g <= m) {
                    return false;
                }
                self.max_seen = Some(*g);
                self.bound = Some((user.clone(), *g));
                true
            }
            Effect::Release { g } => {
                self.max_seen = Some(self.max_seen.map_or(*g, |m| m.max(*g)));
                match &self.bound {
                    Some((_, b)) if b <= g => {
                        self.bound = None;
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    pub fn speaker(&self) -> Option<&str> {
        self.bound.as_ref().map(|(u, _)| u.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_old_bind_dropped() {
        let mut s = Slot::default();
        assert!(s.apply(&Effect::Bind { user: "B".into(), g: 1 }));
        assert!(!s.apply(&Effect::Bind { user: "A".into(), g: 0 }));
        assert!(!s.apply(&Effect::Release { g: 0 }));
        assert_eq!(s.bound, Some(("B".into(), 1)));
    }
}
