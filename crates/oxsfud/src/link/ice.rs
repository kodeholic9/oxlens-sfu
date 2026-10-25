// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-LINK-ICE-RESTART 정:RULE-LINK-TRANSPORT 연:LINK.S.restart 연:LINK.S.checked_new 연:RULE-LINK-ICE-RESTART-PROC · model: claude-opus-5-5
//! ICE 자격 — ufrag 8 · pwd 22 · 연결(자격 한 벌)마다 지금 것 + 대기분 하나(벌마다 최대 둘) · 새 ufrag 로 검증된 첫 검사가 오면 옛 것을 내린다.

const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn token(n: usize) -> String {
    let mut buf = vec![0u8; n];
    let _ = getrandom::fill(&mut buf);
    buf.iter().map(|b| ALPHA[*b as usize % ALPHA.len()] as char).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cred {
    pub ufrag: String,
    pub pwd: String,
}

impl Cred {
    pub fn fresh() -> Cred {
        Cred { ufrag: token(8), pwd: token(22) }
    }
}

#[derive(Debug, Clone)]
pub struct CredSet {
    pub current: Cred,
    pub pending: Option<Cred>,
}

pub enum Checked {
    Current,
    Promoted { dropped: String },
    Unknown,
}

impl CredSet {
    pub fn new() -> CredSet {
        CredSet { current: Cred::fresh(), pending: None }
    }

    pub fn restart(&mut self) -> (Cred, Option<String>) {
        let c = Cred::fresh();
        let replaced = self.pending.replace(c.clone()).map(|p| p.ufrag);
        (c, replaced)
    }

    pub fn pwd_of(&self, ufrag: &str) -> Option<&str> {
        if self.current.ufrag == ufrag {
            return Some(&self.current.pwd);
        }
        self.pending.as_ref().filter(|p| p.ufrag == ufrag).map(|p| p.pwd.as_str())
    }

    pub fn checked(&mut self, ufrag: &str) -> Checked {
        if self.current.ufrag == ufrag {
            return Checked::Current;
        }
        match self.pending.take() {
            Some(p) if p.ufrag == ufrag => {
                let old = std::mem::replace(&mut self.current, p);
                Checked::Promoted { dropped: old.ufrag }
            }
            other => {
                self.pending = other;
                Checked::Unknown
            }
        }
    }

    pub fn ufrags(&self) -> Vec<String> {
        let mut v = vec![self.current.ufrag.clone()];
        if let Some(p) = &self.pending {
            v.push(p.ufrag.clone());
        }
        v
    }
}

impl Default for CredSet {
    fn default() -> Self {
        CredSet::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let c = Cred::fresh();
        assert_eq!(c.ufrag.len(), 8);
        assert_eq!(c.pwd.len(), 22);
    }

    #[test]
    fn at_most_two_per_set() {
        let mut s = CredSet::new();
        let first = s.current.ufrag.clone();
        let (a, _) = s.restart();
        let (b, replaced) = s.restart();
        let (_, replaced2) = s.restart();
        assert_eq!(replaced.as_deref(), Some(a.ufrag.as_str()));
        assert_eq!(replaced2.as_deref(), Some(b.ufrag.as_str()));
        assert!(s.ufrags().len() <= 2, "S-sym-IMPL-LINK-ICE-RESTART-2 재시작 셋 뒤에도 벌마다 ufrag 는 둘 이하");
        assert!(s.pwd_of(&first).is_some(), "새 것이 쓰이기 전엔 옛 것도 받는다");
        let pend = s.pending.clone().expect("pending");
        assert!(matches!(s.checked(&pend.ufrag), Checked::Promoted { .. }));
        assert!(s.pwd_of(&first).is_none(), "새 ufrag 로 검증된 첫 검사 뒤 옛 것을 내린다");
    }
}
