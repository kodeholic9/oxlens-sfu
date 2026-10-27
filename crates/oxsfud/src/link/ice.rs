// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:IMPL-LINK-ICE-RESTART · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:RULE-CONC-SANS-IO · 연:LINK.S.restart · 연:LINK.S.checked_new · 연:RULE-LINK-ICE-LITE · 연:RULE-LINK-EVENT-TYPES · model: claude-opus-5-5
//! ICE-lite 장부(순수) — 자격 한 벌마다 최대 둘(지금 · 대기) · 대기분 교체 · 새 ufrag 첫 검증 검사에 옛 것 내림 · 쌍별 생존 · 경로 = 지명이 실린 검증된 검사가 마지막으로 온 쌍. 검사는 MESSAGE-INTEGRITY 검증 뒤에만 무엇이든 바꾼다.

use std::net::SocketAddr;

use super::stun::Message;

pub const UFRAG_LEN: usize = 8;
pub const PWD_LEN: usize = 22;
pub const UDP_LIVENESS_MS: u64 = 30_000;
const ICE_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creds {
    pub ufrag: String,
    pub pwd: String,
}

fn random_string(n: usize) -> String {
    let mut b = vec![0u8; n];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| ICE_CHARS[usize::from(*x) % ICE_CHARS.len()] as char).collect()
}

impl Creds {
    pub fn generate() -> Creds {
        Creds { ufrag: random_string(UFRAG_LEN), pwd: random_string(PWD_LEN) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    current: Creds,
    pending: Option<Creds>,
}

impl Ledger {
    pub fn new(c: Creds) -> Ledger {
        Ledger { current: c, pending: None }
    }

    pub fn current(&self) -> &Creds {
        &self.current
    }

    pub fn pending(&self) -> Option<&Creds> {
        self.pending.as_ref()
    }

    pub fn restart(&mut self, next: Creds) -> Option<Creds> {
        self.pending.replace(next)
    }

    pub fn pwd_for(&self, ufrag: &str) -> Option<&str> {
        if self.current.ufrag == ufrag {
            return Some(&self.current.pwd);
        }
        self.pending.as_ref().filter(|p| p.ufrag == ufrag).map(|p| p.pwd.as_str())
    }

    pub fn verified(&mut self, ufrag: &str) -> Option<Creds> {
        if self.pending.as_ref().is_some_and(|p| p.ufrag == ufrag)
            && let Some(p) = self.pending.take()
        {
            return Some(std::mem::replace(&mut self.current, p));
        }
        None
    }

    pub fn ufrags(&self) -> Vec<String> {
        let mut v = vec![self.current.ufrag.clone()];
        if let Some(p) = &self.pending {
            v.push(p.ufrag.clone());
        }
        v
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    Udp(SocketAddr),
    Tcp(u64, SocketAddr),
}

impl Via {
    pub fn addr(&self) -> SocketAddr {
        match self {
            Via::Udp(a) | Via::Tcp(_, a) => *a,
        }
    }

    pub fn is_tcp(&self) -> bool {
        matches!(self, Via::Tcp(..))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pair {
    via: Via,
    last_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pairs {
    pairs: Vec<Pair>,
    path: Option<Via>,
}

impl Pairs {
    pub fn new() -> Pairs {
        Pairs::default()
    }

    pub fn path(&self) -> Option<Via> {
        self.path
    }

    pub fn knows(&self, via: &Via) -> bool {
        self.pairs.iter().any(|p| p.via == *via)
    }

    pub fn alive(&self, via: &Via, now_ms: u64) -> bool {
        self.pairs.iter().any(|p| p.via == *via && (via.is_tcp() || now_ms.saturating_sub(p.last_ms) < UDP_LIVENESS_MS))
    }

    pub fn path_alive(&self, now_ms: u64) -> bool {
        self.path.is_some_and(|v| self.alive(&v, now_ms))
    }

    pub fn on_verified(&mut self, via: Via, nominate: bool, now_ms: u64) -> bool {
        match self.pairs.iter_mut().find(|p| p.via == via) {
            Some(p) => p.last_ms = now_ms,
            None => self.pairs.push(Pair { via, last_ms: now_ms }),
        }
        if nominate && self.path != Some(via) {
            self.path = Some(via);
            return true;
        }
        false
    }

    pub fn tcp_closed(&mut self, conn: u64) {
        self.pairs.retain(|p| !matches!(p.via, Via::Tcp(c, _) if c == conn));
        if matches!(self.path, Some(Via::Tcp(c, _)) if c == conn) {
            self.path = None;
        }
    }

    pub fn tcp_conns(&self) -> Vec<u64> {
        self.pairs.iter().filter_map(|p| if let Via::Tcp(c, _) = p.via { Some(c) } else { None }).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Unverified,
    Accepted { key: String, path_moved: bool, promoted: Option<Creds> },
}

pub fn check(ledger: &mut Ledger, pairs: &mut Pairs, msg: &Message, via: Via, now_ms: u64) -> Verdict {
    let Some(ufrag) = msg.local_ufrag().map(str::to_string) else { return Verdict::Unverified };
    let Some(key) = ledger.pwd_for(&ufrag).map(str::to_string) else { return Verdict::Unverified };
    if !msg.verify_integrity(key.as_bytes()) || !msg.verify_fingerprint() {
        return Verdict::Unverified;
    }
    let promoted = ledger.verified(&ufrag);
    let path_moved = pairs.on_verified(via, msg.use_candidate(), now_ms);
    Verdict::Accepted { key, path_moved, promoted }
}

#[cfg(test)]
mod tests {
    use super::super::stun::{binding_request, parse};
    use super::*;

    fn creds(u: &str, p: &str) -> Creds {
        Creds { ufrag: u.into(), pwd: p.into() }
    }

    #[test]
    fn generated_lengths() {
        let c = Creds::generate();
        assert_eq!((c.ufrag.len(), c.pwd.len()), (UFRAG_LEN, PWD_LEN));
    }

    #[test]
    fn ledger_holds_two_and_replaces_pending() {
        let mut l = Ledger::new(creds("aaaa0000", "p0"));
        assert_eq!(l.restart(creds("bbbb1111", "p1")), None);
        let dropped = l.restart(creds("cccc2222", "p2"));
        assert_eq!(dropped.map(|c| c.ufrag), Some("bbbb1111".to_string()));
        assert_eq!(l.ufrags().len(), 2, "S-sym-IMPL-LINK-ICE-RESTART-2 자격 벌마다 최대 둘");
        assert!(l.pwd_for("bbbb1111").is_none(), "S-sym-IMPL-LINK-ICE-RESTART-2 교체된 대기분은 통과하지 않는다");
        assert!(l.pwd_for("aaaa0000").is_some(), "S-sym-IMPL-LINK-ICE-RESTART-1 새 것이 쓰이기 전엔 옛 것도 받는다");
        let old = l.verified("cccc2222");
        assert_eq!(old.map(|c| c.ufrag), Some("aaaa0000".to_string()));
        assert!(l.pwd_for("aaaa0000").is_none());
    }

    #[test]
    fn forged_binding_does_not_move_path() {
        let mut l = Ledger::new(creds("srvufrag", "rightpasswordrightpass"));
        let mut p = Pairs::new();
        let a: SocketAddr = "10.0.0.1:1000".parse().expect("a");
        let good = parse(&binding_request(&[1; 12], "srvufrag:cli", b"rightpasswordrightpass", true)).expect("good");
        assert!(matches!(check(&mut l, &mut p, &good, Via::Udp(a), 0), Verdict::Accepted { path_moved: true, .. }));
        let evil: SocketAddr = "6.6.6.6:666".parse().expect("e");
        let forged = parse(&binding_request(&[2; 12], "srvufrag:cli", b"forged-key", true)).expect("forged");
        assert_eq!(check(&mut l, &mut p, &forged, Via::Udp(evil), 10), Verdict::Unverified);
        assert_eq!(p.path(), Some(Via::Udp(a)), "S-sym-RULE-LINK-TRANSPORT-3 검증 전 STUN 은 경로를 옮기지 못한다");
        assert!(!p.knows(&Via::Udp(evil)));
    }

    #[test]
    fn backup_check_without_nomination_keeps_path() {
        let mut l = Ledger::new(creds("srvufrag", "rightpasswordrightpass"));
        let mut p = Pairs::new();
        let a: SocketAddr = "10.0.0.1:1000".parse().expect("a");
        let b: SocketAddr = "10.0.0.2:2000".parse().expect("b");
        let nom = parse(&binding_request(&[1; 12], "srvufrag:cli", b"rightpasswordrightpass", true)).expect("n");
        let plain = parse(&binding_request(&[3; 12], "srvufrag:cli", b"rightpasswordrightpass", false)).expect("p");
        check(&mut l, &mut p, &nom, Via::Udp(a), 0);
        check(&mut l, &mut p, &plain, Via::Udp(b), 5);
        assert_eq!(p.path(), Some(Via::Udp(a)), "S-sym-RULE-LINK-TRANSPORT-5 지명 없는 검사에 끌려가지 않는다");
        assert!(p.path_alive(29_999));
        assert!(!p.path_alive(30_000));
    }
}
