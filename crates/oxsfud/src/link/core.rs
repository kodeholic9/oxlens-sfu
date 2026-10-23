// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-PEER-STATE · 정:IMPL-LINK-ICE-RESTART · 정:RULE-CONC-SANS-IO · 연:RULE-LINK-SDP-COMMON · model: claude-opus-5-5
//! 연결 산수(sans-IO) — 첫 바이트 가르기 · STUN Binding(무결성 검증 · 응답) · RFC 4571 틀 · ICE 자격 · 쌍별 생존과 PeerState 판정.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const UFRAG_LEN: usize = 8;
pub const PWD_LEN: usize = 22;
pub const UDP_PAIR_TTL_MS: u64 = 30_000;
pub const TCP_UNVERIFIED_MS: u64 = 10_000;
pub const TCP_KEEPALIVE_MS: u64 = 20_000;
pub const TCP_FRAME_MAX: usize = 65_535;
pub const SOFTWARE: &str = "oxlens-sfu";

const MAGIC: u32 = 0x2112_a442;
const FP_XOR: u32 = 0x5354_554e;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const A_USERNAME: u16 = 0x0006;
const A_MI: u16 = 0x0008;
const A_XOR_MAPPED: u16 = 0x0020;
const A_PRIORITY: u16 = 0x0024;
const A_USE_CANDIDATE: u16 = 0x0025;
const A_SOFTWARE: u16 = 0x8022;
const A_FINGERPRINT: u16 = 0x8028;
const A_ICE_CONTROLLED: u16 = 0x8029;
const A_ICE_CONTROLLING: u16 = 0x802a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Stun,
    Dtls,
    Rtp,
    Rtcp,
    Other,
}

pub fn classify(b: &[u8]) -> Kind {
    match b.first().copied() {
        Some(0..=3) => Kind::Stun,
        Some(20..=63) => Kind::Dtls,
        Some(128..=191) if b.len() >= 2 => {
            if (192..=223).contains(&b[1]) {
                Kind::Rtcp
            } else {
                Kind::Rtp
            }
        }
        _ => Kind::Other,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub tid: [u8; 12],
    pub username: String,
    pub use_candidate: bool,
    pub controlling: bool,
    pub priority: Option<u32>,
    mi_at: usize,
    raw: Vec<u8>,
}

impl Binding {
    pub fn local_ufrag(&self) -> &str {
        self.username.split(':').next().unwrap_or("")
    }

    pub fn verify(&self, pwd: &str) -> bool {
        let Some(mi) = self.raw.get(self.mi_at + 4..self.mi_at + 24) else { return false };
        let mut head = self.raw[..self.mi_at].to_vec();
        let len = (self.mi_at + 24 - 20) as u16;
        head[2..4].copy_from_slice(&len.to_be_bytes());
        hmac_sha1(pwd.as_bytes(), &head).as_slice() == mi
    }
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    let Ok(mut m) = <Hmac<Sha1> as Mac>::new_from_slice(key) else { return Vec::new() };
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub fn parse_binding(b: &[u8]) -> Option<Binding> {
    if b.len() < 20 {
        return None;
    }
    let ty = u16::from_be_bytes([b[0], b[1]]);
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    let magic = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    if ty != BINDING_REQUEST || magic != MAGIC || 20 + len > b.len() {
        return None;
    }
    let mut tid = [0u8; 12];
    tid.copy_from_slice(&b[8..20]);
    let mut out = Binding { tid, username: String::new(), use_candidate: false, controlling: false, priority: None, mi_at: 0, raw: b[..20 + len].to_vec() };
    let mut i = 20;
    let mut seen_mi = false;
    while i + 4 <= 20 + len {
        let at = u16::from_be_bytes([b[i], b[i + 1]]);
        let al = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let v = b.get(i + 4..i + 4 + al)?;
        match at {
            A_USERNAME if !seen_mi => out.username = String::from_utf8_lossy(v).to_string(),
            A_USE_CANDIDATE if !seen_mi => out.use_candidate = true,
            A_ICE_CONTROLLING if !seen_mi => out.controlling = true,
            A_PRIORITY if !seen_mi && al == 4 => out.priority = Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]])),
            A_MI if !seen_mi => {
                out.mi_at = i;
                seen_mi = true;
            }
            _ => {}
        }
        i += 4 + al.div_ceil(4) * 4;
    }
    if !seen_mi || out.username.is_empty() {
        return None;
    }
    Some(out)
}

fn attr(out: &mut Vec<u8>, ty: u16, v: &[u8]) {
    out.extend_from_slice(&ty.to_be_bytes());
    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
    out.extend_from_slice(v);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn set_len(out: &mut [u8], extra: usize) {
    let len = (out.len() - 20 + extra) as u16;
    out[2..4].copy_from_slice(&len.to_be_bytes());
}

pub fn success(tid: &[u8; 12], from: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(tid);
    let port = from.port() ^ (MAGIC >> 16) as u16;
    let mut x = vec![0u8];
    match from.ip() {
        IpAddr::V4(v4) => {
            x.push(1);
            x.extend_from_slice(&port.to_be_bytes());
            let ip = u32::from_be_bytes(v4.octets()) ^ MAGIC;
            x.extend_from_slice(&ip.to_be_bytes());
        }
        IpAddr::V6(v6) => {
            x.push(2);
            x.extend_from_slice(&port.to_be_bytes());
            let mut key = MAGIC.to_be_bytes().to_vec();
            key.extend_from_slice(tid);
            for (i, o) in v6.octets().iter().enumerate() {
                x.push(o ^ key[i]);
            }
        }
    }
    attr(&mut out, A_XOR_MAPPED, &x);
    attr(&mut out, A_SOFTWARE, SOFTWARE.as_bytes());
    set_len(&mut out, 24);
    let mi = hmac_sha1(pwd.as_bytes(), &out);
    attr(&mut out, A_MI, &mi);
    set_len(&mut out, 8);
    let fp = crc32fast::hash(&out) ^ FP_XOR;
    attr(&mut out, A_FINGERPRINT, &fp.to_be_bytes());
    out
}

pub fn binding_request(local: &str, remote: &str, pwd: &str, use_candidate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&[7u8; 12]);
    attr(&mut out, A_USERNAME, format!("{local}:{remote}").as_bytes());
    if use_candidate {
        attr(&mut out, A_USE_CANDIDATE, &[]);
    }
    attr(&mut out, A_ICE_CONTROLLED, &[0; 8]);
    set_len(&mut out, 24);
    let mi = hmac_sha1(pwd.as_bytes(), &out);
    attr(&mut out, A_MI, &mi);
    set_len(&mut out, 0);
    out
}

#[derive(Default)]
pub struct Deframer {
    buf: Vec<u8>,
}

impl Deframer {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 2 {
                break;
            }
            let n = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
            if self.buf.len() < 2 + n {
                break;
            }
            out.push(self.buf[2..2 + n].to_vec());
            self.buf.drain(..2 + n);
        }
        out
    }
}

pub fn frame(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() > TCP_FRAME_MAX {
        return None;
    }
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cred {
    pub ufrag: String,
    pub pwd: String,
}

impl Cred {
    pub fn fresh() -> Cred {
        Cred { ufrag: common::ident::random_alnum(UFRAG_LEN), pwd: common::ident::random_alnum(PWD_LEN) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Creds {
    pub cur: Cred,
    pub pend: Option<Cred>,
}

impl Creds {
    pub fn new() -> Creds {
        Creds { cur: Cred::fresh(), pend: None }
    }

    pub fn restart(&mut self) -> (Cred, Option<String>) {
        let n = Cred::fresh();
        let dropped = self.pend.replace(n.clone()).map(|c| c.ufrag);
        (n, dropped)
    }

    pub fn pwd_of(&self, ufrag: &str) -> Option<&str> {
        if self.cur.ufrag == ufrag {
            return Some(&self.cur.pwd);
        }
        self.pend.as_ref().filter(|c| c.ufrag == ufrag).map(|c| c.pwd.as_str())
    }

    pub fn checked(&mut self, ufrag: &str) -> Option<String> {
        if self.pend.as_ref().map(|c| c.ufrag.as_str()) != Some(ufrag) {
            return None;
        }
        let n = self.pend.take()?;
        let old = std::mem::replace(&mut self.cur, n);
        Some(old.ufrag)
    }

    pub fn generation(&self) -> &str {
        &self.cur.ufrag
    }
}

impl Default for Creds {
    fn default() -> Self {
        Creds::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Udp,
    Tcp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerState {
    Alive,
    Suspect,
}

impl PeerState {
    pub fn name(self) -> &'static str {
        match self {
            PeerState::Alive => "alive",
            PeerState::Suspect => "suspect",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub proto: Proto,
    pub last_ms: u64,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pairs {
    pub map: BTreeMap<SocketAddr, Pair>,
    pub route: Option<SocketAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edge {
    None,
    Suspect,
    Alive,
}

impl Pairs {
    pub fn alive(&self, a: &SocketAddr, now: u64) -> bool {
        match self.map.get(a) {
            Some(p) if p.proto == Proto::Tcp => p.connected,
            Some(p) => now.saturating_sub(p.last_ms) < UDP_PAIR_TTL_MS,
            None => false,
        }
    }

    pub fn checked(&mut self, a: SocketAddr, proto: Proto, nominate: bool, now: u64, state: PeerState) -> Edge {
        let e = self.map.entry(a).or_insert(Pair { proto, last_ms: now, connected: proto == Proto::Tcp });
        e.last_ms = now;
        if proto == Proto::Tcp {
            e.connected = true;
        }
        if nominate {
            self.route = Some(a);
            if state == PeerState::Suspect {
                return Edge::Alive;
            }
        }
        Edge::None
    }

    pub fn tcp_gone(&mut self, a: &SocketAddr) {
        if let Some(p) = self.map.get_mut(a) {
            p.connected = false;
        }
    }

    pub fn sweep(&mut self, now: u64, state: PeerState) -> Edge {
        let dead: Vec<SocketAddr> = self.map.iter().filter(|(a, _)| !self.alive(a, now)).map(|(a, _)| *a).collect();
        let route_dead = self.route.map(|r| dead.contains(&r)).unwrap_or(false);
        for a in &dead {
            if Some(*a) != self.route {
                self.map.remove(a);
            }
        }
        if route_dead && state == PeerState::Alive {
            return Edge::Suspect;
        }
        Edge::None
    }

    pub fn send_to(&self, now: u64) -> Option<(SocketAddr, Proto)> {
        let r = self.route?;
        if !self.alive(&r, now) {
            return None;
        }
        self.map.get(&r).map(|p| (r, p.proto))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demux_by_first_byte() {
        assert_eq!(classify(&[0, 1]), Kind::Stun);
        assert_eq!(classify(&[22, 0]), Kind::Dtls);
        assert_eq!(classify(&[0x80, 96]), Kind::Rtp);
        assert_eq!(classify(&[0x80, 200]), Kind::Rtcp);
    }

    #[test]
    fn binding_round_trip_verifies_integrity() {
        let req = binding_request("srv12345", "cli", "pppppppppppppppppppppp", true);
        let b = parse_binding(&req).unwrap_or_else(|| panic!("parse"));
        assert_eq!(b.local_ufrag(), "srv12345");
        assert!(b.use_candidate);
        assert!(b.verify("pppppppppppppppppppppp"), "S-sym-RULE-LINK-TRANSPORT-3");
        assert!(!b.verify("wrong"));
        let r = success(&b.tid, "127.0.0.1:5000".parse().unwrap_or_else(|_| panic!()), "pppppppppppppppppppppp");
        assert_eq!(u16::from_be_bytes([r[0], r[1]]), BINDING_SUCCESS);
    }

    #[test]
    fn rfc4571_partial_frames_wait() {
        let mut d = Deframer::default();
        assert!(d.push(&[0, 3, 1]).is_empty());
        assert_eq!(d.push(&[2, 3, 0, 0]), vec![vec![1, 2, 3], vec![]]);
    }

    #[test]
    fn restart_keeps_old_until_checked() {
        let mut c = Creds::new();
        let old = c.cur.ufrag.clone();
        let (n, _) = c.restart();
        assert!(c.pwd_of(&old).is_some() && c.pwd_of(&n.ufrag).is_some());
        assert_eq!(c.checked(&n.ufrag), Some(old.clone()));
        assert!(c.pwd_of(&old).is_none(), "S-sym-IMPL-LINK-ICE-RESTART-1");
    }

    #[test]
    fn route_death_is_suspect_once() {
        let mut p = Pairs::default();
        let a: SocketAddr = "1.2.3.4:5".parse().unwrap_or_else(|_| panic!());
        p.checked(a, Proto::Udp, true, 0, PeerState::Alive);
        assert_eq!(p.sweep(10_000, PeerState::Alive), Edge::None);
        assert_eq!(p.sweep(30_001, PeerState::Alive), Edge::Suspect);
        assert_eq!(p.sweep(40_000, PeerState::Suspect), Edge::None);
        assert_eq!(p.checked(a, Proto::Udp, true, 41_000, PeerState::Suspect), Edge::Alive);
    }
}
