// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 연:LINK.S.restart · 연:LINK.S.checked_new · 연:LINK.S.checked_current · 연:LINK.S.suspect · 연:RULE-LINK-RECOVERY-UNIT · model: claude-opus-5-5
//! 연결 판정(순수) — 자격 벌(벌마다 ufrag ≤ 2 · 대기분 교체) · 쌍별 생존(UDP 30s · TCP 연결 = 생존) · 경로 = 지명이 실린 마지막 쌍 · PeerState · RFC 4571 틀.

use std::collections::HashMap;

pub const UDP_PAIR_TIMEOUT_MS: u64 = 30_000;
pub const TCP_IDENTIFY_MS: u64 = 10_000;
pub const TCP_KEEPALIVE_MS: u64 = 20_000;
pub const UFRAG_LEN: usize = 8;
pub const PWD_LEN: usize = 22;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cred {
    pub ufrag: String,
    pub pwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creds {
    pub current: Cred,
    pub pending: Option<Cred>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restart {
    Replaced { retired: Option<String> },
}

impl Creds {
    pub fn new(c: Cred) -> Creds {
        Creds { current: c, pending: None }
    }

    pub fn pwd_of(&self, ufrag: &str) -> Option<&str> {
        if self.current.ufrag == ufrag {
            return Some(&self.current.pwd);
        }
        self.pending.as_ref().filter(|p| p.ufrag == ufrag).map(|p| p.pwd.as_str())
    }

    pub fn restart(&mut self, fresh: Cred) -> Option<String> {
        let retired = self.pending.take().map(|p| p.ufrag);
        self.pending = Some(fresh);
        retired
    }

    pub fn checked(&mut self, ufrag: &str) -> Option<String> {
        if self.pending.as_ref().map(|p| p.ufrag == ufrag).unwrap_or(false) {
            let fresh = self.pending.take().unwrap_or_else(|| self.current.clone());
            let old = std::mem::replace(&mut self.current, fresh);
            return Some(old.ufrag);
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
pub enum PathKey {
    Udp(std::net::SocketAddr),
    Tcp(u64),
}

impl PathKey {
    pub fn is_tcp(&self) -> bool {
        matches!(self, PathKey::Tcp(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Alive,
    Suspect,
}

impl PeerState {
    pub fn as_str(self) -> &'static str {
        match self {
            PeerState::Alive => "alive",
            PeerState::Suspect => "suspect",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Pairs {
    last_check: HashMap<PathKey, u64>,
    tcp_open: HashMap<u64, bool>,
    pub path: Option<PathKey>,
    pub state_suspect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Unchanged,
    BecameSuspect,
    BecameAlive,
}

impl Pairs {
    pub fn alive(&self, k: &PathKey, now_ms: u64) -> bool {
        match k {
            PathKey::Tcp(id) => self.tcp_open.get(id).copied().unwrap_or(false),
            PathKey::Udp(_) => self.last_check.get(k).map(|t| now_ms.saturating_sub(*t) < UDP_PAIR_TIMEOUT_MS).unwrap_or(false),
        }
    }

    pub fn checked(&mut self, k: PathKey, nominated: bool, now_ms: u64) -> Liveness {
        self.last_check.insert(k, now_ms);
        if let PathKey::Tcp(id) = k {
            self.tcp_open.insert(id, true);
        }
        if nominated {
            self.path = Some(k);
            if self.state_suspect {
                self.state_suspect = false;
                return Liveness::BecameAlive;
            }
        }
        Liveness::Unchanged
    }

    pub fn tcp_closed(&mut self, id: u64) {
        self.tcp_open.insert(id, false);
    }

    pub fn tick(&mut self, now_ms: u64) -> Liveness {
        let Some(p) = self.path else { return Liveness::Unchanged };
        if !self.alive(&p, now_ms) && !self.state_suspect {
            self.state_suspect = true;
            return Liveness::BecameSuspect;
        }
        Liveness::Unchanged
    }

    pub fn send_path(&self, now_ms: u64) -> Option<PathKey> {
        self.path.filter(|p| self.alive(p, now_ms))
    }

    pub fn tcp_alive(&self) -> Vec<u64> {
        self.tcp_open.iter().filter(|(_, o)| **o).map(|(id, _)| *id).collect()
    }

    pub fn state(&self) -> PeerState {
        if self.state_suspect { PeerState::Suspect } else { PeerState::Alive }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

pub enum Frame {
    Keepalive,
    Packet(Vec<u8>),
}

impl Deframer {
    pub fn push(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        if self.buf.len() < 2 {
            return None;
        }
        let len = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
        if self.buf.len() < 2 + len {
            return None;
        }
        let rest = self.buf.split_off(2 + len);
        let frame = std::mem::replace(&mut self.buf, rest);
        if len == 0 {
            return Some(Frame::Keepalive);
        }
        Some(Frame::Packet(frame[2..].to_vec()))
    }
}

pub fn frame(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + 2);
    out.extend_from_slice(&(b.len().min(u16::MAX as usize) as u16).to_be_bytes());
    out.extend_from_slice(&b[..b.len().min(u16::MAX as usize)]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(u: &str) -> Cred {
        Cred { ufrag: u.into(), pwd: format!("{u}-pwd") }
    }

    #[test]
    fn restart_keeps_at_most_two_and_replaces_pending() {
        let mut k = Creds::new(c("a"));
        assert_eq!(k.restart(c("b")), None);
        assert_eq!(k.restart(c("c")), Some("b".into()));
        assert_eq!(k.restart(c("d")), Some("c".into()));
        assert_eq!(k.ufrags().len(), 2, "S-sym-IMPL-LINK-ICE-RESTART-2 자격 벌마다 ufrag ≤ 2 · 대기분은 마지막 것으로 교체");
        assert!(k.pwd_of("b").is_none(), "S-sym-IMPL-LINK-ICE-RESTART-2 첫 응답 자격은 거절");
        assert!(k.pwd_of("a").is_some(), "새 것이 쓰이기 전에는 옛 ufrag 의 검사도 받는다");
        assert_eq!(k.checked("d"), Some("a".into()));
        assert!(k.pwd_of("a").is_none());
        assert_eq!(k.current.ufrag, "d");
    }

    #[test]
    fn path_follows_nomination_and_forged_check_cannot_move_it() {
        let mut p = Pairs::default();
        let a = PathKey::Udp("1.1.1.1:1".parse().expect("a"));
        let b = PathKey::Udp("2.2.2.2:2".parse().expect("b"));
        p.checked(a, true, 0);
        p.checked(b, false, 10);
        assert_eq!(p.path, Some(a), "S-sym-RULE-LINK-TRANSPORT-3 지명 없는 검사로는 경로가 옮지 않는다");
        assert_eq!(p.tick(UDP_PAIR_TIMEOUT_MS), Liveness::BecameSuspect);
        assert_eq!(p.tick(UDP_PAIR_TIMEOUT_MS + 1), Liveness::Unchanged, "통지를 되풀이하지 않는다");
        assert_eq!(p.checked(b, true, UDP_PAIR_TIMEOUT_MS + 2), Liveness::BecameAlive);
        assert_eq!(p.send_path(UDP_PAIR_TIMEOUT_MS + 3), Some(b));
        let t = PathKey::Tcp(9);
        p.checked(t, true, 0);
        assert!(p.alive(&t, 10 * UDP_PAIR_TIMEOUT_MS), "TCP 는 연결이 있으면 생존");
    }

    #[test]
    fn rfc4571_framing() {
        let mut d = Deframer::default();
        let mut wire = frame(&[1, 2, 3]);
        wire.extend_from_slice(&[0, 0]);
        wire.extend_from_slice(&frame(&[9])[..2]);
        d.push(&wire);
        assert!(matches!(d.next_frame(), Some(Frame::Packet(p)) if p == vec![1, 2, 3]));
        assert!(matches!(d.next_frame(), Some(Frame::Keepalive)), "길이 0 은 keepalive");
        assert!(d.next_frame().is_none(), "부분 수신은 보류");
        d.push(&[9]);
        assert!(matches!(d.next_frame(), Some(Frame::Packet(p)) if p == vec![9]));
    }
}
