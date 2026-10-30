// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-LINK-ICE-RESTART · 정:RULE-LINK-TRANSPORT · 연:LINK.S.restart · 연:LINK.S.checked_new · model: claude-opus-5-5
//! ICE 자격 장부 — 연결마다 한 벌(ufrag 8 · pwd 22) · ★벌마다 최대 둘(쓰는 것 + 대기) · 새 ufrag 첫 검증 검사에 옛 것을 내린다.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cred {
    pub ufrag: String,
    pub pwd: String,
}

const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn token(n: usize) -> String {
    common::random_bytes(n).iter().map(|b| ALNUM[(*b as usize) % ALNUM.len()] as char).collect()
}

impl Cred {
    pub fn fresh() -> Cred {
        Cred { ufrag: token(8), pwd: token(22) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creds {
    pub active: Cred,
    pub pending: Option<Cred>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    Current,
    Promoted { retired: String },
}

impl Creds {
    pub fn new() -> Creds {
        Creds { active: Cred::fresh(), pending: None }
    }

    pub fn restart(&mut self, next: Cred) -> Option<String> {
        self.pending.replace(next).map(|old| old.ufrag)
    }

    pub fn pwd(&self, ufrag: &str) -> Option<&str> {
        if self.active.ufrag == ufrag {
            return Some(&self.active.pwd);
        }
        self.pending.as_ref().filter(|p| p.ufrag == ufrag).map(|p| p.pwd.as_str())
    }

    pub fn checked(&mut self, ufrag: &str) -> Checked {
        match &self.pending {
            Some(p) if p.ufrag == ufrag => {
                let p = self.pending.take().unwrap_or_else(Cred::fresh);
                let old = std::mem::replace(&mut self.active, p);
                Checked::Promoted { retired: old.ufrag }
            }
            _ => Checked::Current,
        }
    }

    pub fn ufrags(&self) -> Vec<String> {
        let mut v = vec![self.active.ufrag.clone()];
        if let Some(p) = &self.pending {
            v.push(p.ufrag.clone());
        }
        v
    }

    pub fn use_ufrag(&self) -> &str {
        self.pending.as_ref().map(|p| p.ufrag.as_str()).unwrap_or(&self.active.ufrag)
    }
}

impl Default for Creds {
    fn default() -> Self {
        Creds::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_two_and_promote() {
        let mut c = Creds::new();
        let a = c.active.ufrag.clone();
        let p1 = Cred::fresh();
        assert_eq!(c.restart(p1.clone()), None);
        let p2 = Cred::fresh();
        assert_eq!(c.restart(p2.clone()), Some(p1.ufrag.clone()));
        assert_eq!(c.ufrags().len(), 2);
        assert!(c.pwd(&p1.ufrag).is_none());
        assert_eq!(c.checked(&a), Checked::Current);
        assert_eq!(c.checked(&p2.ufrag), Checked::Promoted { retired: a.clone() });
        assert!(c.pwd(&a).is_none());
    }
}
