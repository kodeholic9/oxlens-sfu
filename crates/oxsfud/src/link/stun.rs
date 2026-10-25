// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-LINK-TRANSPORT 정:RULE-CONC-SANS-IO 정:RULE-DOC-FIXED-PARTS · model: claude-opus-5-5
//! STUN Binding — 파싱 · MESSAGE-INTEGRITY(HMAC-SHA1) 검증 · 성공 응답(XOR-MAPPED-ADDRESS · SOFTWARE · MI · FINGERPRINT). RFC 8489.

use std::net::{IpAddr, SocketAddr};

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const COOKIE: u32 = 0x2112_A442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
const USERNAME: u16 = 0x0006;
const MESSAGE_INTEGRITY: u16 = 0x0008;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;
const USE_CANDIDATE: u16 = 0x0025;
const SOFTWARE: u16 = 0x8022;
const FINGERPRINT: u16 = 0x8028;
const FP_XOR: u32 = 0x5354_554E;
pub const SOFTWARE_NAME: &str = "oxsfud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub txid: [u8; 12],
    pub username: Option<String>,
    pub use_candidate: bool,
    pub mi_at: Option<usize>,
}

impl Request {
    pub fn local_ufrag(&self) -> Option<&str> {
        self.username.as_deref().and_then(|u| u.split(':').next())
    }
}

pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= 20 && b[0] < 4 && u32::from_be_bytes([b[4], b[5], b[6], b[7]]) == COOKIE
}

pub fn parse(b: &[u8]) -> Option<Request> {
    if !is_stun(b) {
        return None;
    }
    let ty = u16::from_be_bytes([b[0], b[1]]);
    if ty != BINDING_REQUEST {
        return None;
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if b.len() < 20 + len {
        return None;
    }
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&b[8..20]);
    let mut r = Request { txid, username: None, use_candidate: false, mi_at: None };
    let mut off = 20;
    while off + 4 <= 20 + len {
        let at = u16::from_be_bytes([b[off], b[off + 1]]);
        let al = u16::from_be_bytes([b[off + 2], b[off + 3]]) as usize;
        let v = off + 4;
        if v + al > b.len() {
            return None;
        }
        match at {
            USERNAME => r.username = std::str::from_utf8(&b[v..v + al]).ok().map(str::to_string),
            USE_CANDIDATE => r.use_candidate = true,
            MESSAGE_INTEGRITY => {
                r.mi_at = Some(off);
                break;
            }
            _ => {}
        }
        off = v + al.div_ceil(4) * 4;
    }
    Some(r)
}

fn hmac(key: &[u8], data: &[u8]) -> Option<[u8; 20]> {
    let mut m = Hmac::<Sha1>::new_from_slice(key).ok()?;
    m.update(data);
    let out = m.finalize().into_bytes();
    let mut a = [0u8; 20];
    a.copy_from_slice(&out);
    Some(a)
}

pub fn integrity_ok(b: &[u8], mi_at: usize, pwd: &str) -> bool {
    if b.len() < mi_at + 24 || mi_at < 20 {
        return false;
    }
    let mut head = b[..mi_at].to_vec();
    let fake_len = (mi_at - 20 + 24) as u16;
    head[2..4].copy_from_slice(&fake_len.to_be_bytes());
    match hmac(pwd.as_bytes(), &head) {
        Some(mac) => mac[..] == b[mi_at + 4..mi_at + 24],
        None => false,
    }
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
    let l = (out.len() - 20 + extra) as u16;
    out[2..4].copy_from_slice(&l.to_be_bytes());
}

pub fn xor_addr(addr: SocketAddr, txid: &[u8; 12]) -> Vec<u8> {
    let port = addr.port() ^ (COOKIE >> 16) as u16;
    let mut v = vec![0u8];
    match addr.ip() {
        IpAddr::V4(ip) => {
            v.push(0x01);
            v.extend_from_slice(&port.to_be_bytes());
            let x = u32::from(ip) ^ COOKIE;
            v.extend_from_slice(&x.to_be_bytes());
        }
        IpAddr::V6(ip) => {
            v.push(0x02);
            v.extend_from_slice(&port.to_be_bytes());
            let mut mask = COOKIE.to_be_bytes().to_vec();
            mask.extend_from_slice(txid);
            for (a, m) in ip.octets().iter().zip(mask.iter()) {
                v.push(a ^ m);
            }
        }
    }
    v
}

pub fn success(txid: &[u8; 12], mapped: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&COOKIE.to_be_bytes());
    out.extend_from_slice(txid);
    attr(&mut out, XOR_MAPPED_ADDRESS, &xor_addr(mapped, txid));
    attr(&mut out, SOFTWARE, SOFTWARE_NAME.as_bytes());
    set_len(&mut out, 24);
    let mac = hmac(pwd.as_bytes(), &out).unwrap_or([0; 20]);
    attr(&mut out, MESSAGE_INTEGRITY, &mac);
    set_len(&mut out, 8);
    let crc = crc32fast::hash(&out) ^ FP_XOR;
    attr(&mut out, FINGERPRINT, &crc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(user: &str, pwd: &str, use_cand: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&COOKIE.to_be_bytes());
        out.extend_from_slice(&[7u8; 12]);
        attr(&mut out, USERNAME, user.as_bytes());
        if use_cand {
            attr(&mut out, USE_CANDIDATE, &[]);
        }
        set_len(&mut out, 24);
        let mac = hmac(pwd.as_bytes(), &out).expect("mac");
        attr(&mut out, MESSAGE_INTEGRITY, &mac);
        set_len(&mut out, 8);
        let crc = crc32fast::hash(&out) ^ FP_XOR;
        attr(&mut out, FINGERPRINT, &crc.to_be_bytes());
        out
    }

    #[test]
    fn verify_before_trust() {
        let b = request("srvufrag:cli", "0123456789012345678901", true);
        let r = parse(&b).expect("parse");
        assert_eq!(r.local_ufrag(), Some("srvufrag"));
        assert!(r.use_candidate);
        let at = r.mi_at.expect("mi");
        assert!(integrity_ok(&b, at, "0123456789012345678901"));
        assert!(!integrity_ok(&b, at, "wrong-password-xxxxxxx"), "S-sym-RULE-LINK-TRANSPORT-3 위조 Binding 은 검증에서 떨어진다");
    }

    #[test]
    fn success_carries_fingerprint() {
        let b = success(&[1; 12], "10.0.0.1:5000".parse().expect("addr"), "pw");
        let n = b.len();
        let crc = crc32fast::hash(&b[..n - 8]) ^ FP_XOR;
        assert_eq!(&b[n - 4..], &crc.to_be_bytes());
        assert_eq!(u16::from_be_bytes([b[2], b[3]]) as usize, n - 20);
    }
}
