// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:IMPL-LINK-ICE-RESTART · 연:RULE-LINK-SDP-COMMON · model: claude-opus-5-5
//! STUN(순수) — Binding 요청 읽기 · message-integrity 검증(USERNAME 의 ufrag 의 pwd) · 응답 짓기(XOR-MAPPED-ADDRESS · MI · FINGERPRINT · SOFTWARE `ox`) · 첫 바이트 demux.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::net::SocketAddr;

pub const MAGIC: u32 = 0x2112_a442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_PRIORITY: u16 = 0x0024;
pub const ATTR_USE_CANDIDATE: u16 = 0x0025;
pub const ATTR_SOFTWARE: u16 = 0x8022;
pub const ATTR_FINGERPRINT: u16 = 0x8028;
pub const ATTR_ICE_CONTROLLED: u16 = 0x8029;
pub const ATTR_ICE_CONTROLLING: u16 = 0x802a;
pub const SOFTWARE: &str = "ox-sfu";
const FINGERPRINT_XOR: u32 = 0x5354_554e;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Stun,
    Dtls,
    Rtp,
    Other,
}

pub fn classify(b: &[u8]) -> Class {
    match b.first() {
        Some(0..=3) => Class::Stun,
        Some(20..=63) => Class::Dtls,
        Some(128..=191) => Class::Rtp,
        _ => Class::Other,
    }
}

pub fn is_rtcp(b: &[u8]) -> bool {
    b.len() >= 2 && (192..=223).contains(&b[1])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub tid: [u8; 12],
    pub username: String,
    pub use_candidate: bool,
    pub controlling: bool,
    mi_off: Option<usize>,
}

impl Request {
    pub fn local_ufrag(&self) -> &str {
        self.username.split(':').next().unwrap_or("")
    }

    pub fn remote_ufrag(&self) -> &str {
        self.username.split(':').nth(1).unwrap_or("")
    }
}

pub fn parse_request(b: &[u8]) -> Option<Request> {
    if b.len() < 20 {
        return None;
    }
    let typ = u16::from_be_bytes([b[0], b[1]]);
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if typ != BINDING_REQUEST || u32::from_be_bytes([b[4], b[5], b[6], b[7]]) != MAGIC || b.len() < 20 + len {
        return None;
    }
    let mut tid = [0u8; 12];
    tid.copy_from_slice(&b[8..20]);
    let mut r = Request { tid, username: String::new(), use_candidate: false, controlling: false, mi_off: None };
    let mut i = 20;
    while i + 4 <= 20 + len {
        let at = u16::from_be_bytes([b[i], b[i + 1]]);
        let al = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if i + 4 + al > b.len() {
            return None;
        }
        let v = &b[i + 4..i + 4 + al];
        match at {
            ATTR_USERNAME => r.username = String::from_utf8_lossy(v).into_owned(),
            ATTR_USE_CANDIDATE => r.use_candidate = true,
            ATTR_ICE_CONTROLLING => r.controlling = true,
            ATTR_MESSAGE_INTEGRITY => {
                r.mi_off = Some(i);
                break;
            }
            _ => {}
        }
        i += 4 + al.div_ceil(4) * 4;
    }
    Some(r)
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key).unwrap_or_else(|_| <Hmac<Sha1> as Mac>::new_from_slice(&[0]).unwrap_or_else(|_| unreachable!()));
    mac.update(data);
    let out = mac.finalize().into_bytes();
    let mut a = [0u8; 20];
    a.copy_from_slice(&out);
    a
}

pub fn integrity_ok(b: &[u8], req: &Request, pwd: &str) -> bool {
    let Some(off) = req.mi_off else { return false };
    if b.len() < off + 24 {
        return false;
    }
    let mut head = b[..off].to_vec();
    let fake_len = (off + 24 - 20) as u16;
    head[2..4].copy_from_slice(&fake_len.to_be_bytes());
    let want = hmac(pwd.as_bytes(), &head);
    let got = &b[off + 4..off + 24];
    want.iter().zip(got).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn put_attr(out: &mut Vec<u8>, t: u16, v: &[u8]) {
    out.extend_from_slice(&t.to_be_bytes());
    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
    out.extend_from_slice(v);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn set_len(out: &mut [u8], len: usize) {
    out[2..4].copy_from_slice(&((len - 20) as u16).to_be_bytes());
}

pub fn xor_addr(addr: SocketAddr, tid: &[u8; 12]) -> Vec<u8> {
    let port = addr.port() ^ (MAGIC >> 16) as u16;
    let mut v = vec![0u8];
    match addr {
        SocketAddr::V4(a) => {
            v.push(0x01);
            v.extend_from_slice(&port.to_be_bytes());
            let ip = u32::from_be_bytes(a.ip().octets()) ^ MAGIC;
            v.extend_from_slice(&ip.to_be_bytes());
        }
        SocketAddr::V6(a) => {
            v.push(0x02);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&MAGIC.to_be_bytes());
            key[4..].copy_from_slice(tid);
            for (o, k) in a.ip().octets().iter().zip(key.iter()) {
                v.push(o ^ k);
            }
        }
    }
    v
}

pub fn success(req: &Request, from: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&req.tid);
    put_attr(&mut out, ATTR_XOR_MAPPED_ADDRESS, &xor_addr(from, &req.tid));
    put_attr(&mut out, ATTR_SOFTWARE, SOFTWARE.as_bytes());
    let mi_at = out.len();
    set_len(&mut out, mi_at + 24);
    let mac = hmac(pwd.as_bytes(), &out);
    put_attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp_at = out.len();
    set_len(&mut out, fp_at + 8);
    let crc = crc32fast::hash(&out) ^ FINGERPRINT_XOR;
    put_attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

pub fn fingerprint_ok(b: &[u8]) -> bool {
    if b.len() < 28 {
        return true;
    }
    let at = b.len() - 8;
    if u16::from_be_bytes([b[at], b[at + 1]]) != ATTR_FINGERPRINT {
        return true;
    }
    let want = u32::from_be_bytes([b[at + 4], b[at + 5], b[at + 6], b[at + 7]]);
    crc32fast::hash(&b[..at]) ^ FINGERPRINT_XOR == want
}

#[cfg(test)]
pub fn request_for_test(username: &str, pwd: &str, use_candidate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&[7u8; 12]);
    put_attr(&mut out, ATTR_USERNAME, username.as_bytes());
    put_attr(&mut out, ATTR_ICE_CONTROLLING, &[0; 8]);
    if use_candidate {
        put_attr(&mut out, ATTR_USE_CANDIDATE, &[]);
    }
    let at = out.len();
    set_len(&mut out, at + 24);
    let mac = hmac(pwd.as_bytes(), &out);
    put_attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp = out.len();
    set_len(&mut out, fp + 8);
    let crc = crc32fast::hash(&out) ^ FINGERPRINT_XOR;
    put_attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_round_trip_and_integrity() {
        let req = request_for_test("srvUfrag:cli", "server-password-22chars", true);
        assert_eq!(classify(&req), Class::Stun);
        let r = parse_request(&req).expect("req");
        assert_eq!(r.local_ufrag(), "srvUfrag");
        assert!(r.use_candidate && r.controlling);
        assert!(integrity_ok(&req, &r, "server-password-22chars"));
        assert!(!integrity_ok(&req, &r, "wrong"), "S-sym-RULE-LINK-TRANSPORT-3 MI 가 틀리면 처리하지 않는다");
        assert!(fingerprint_ok(&req));
        let res = success(&r, "10.0.0.1:5000".parse().expect("addr"), "server-password-22chars");
        assert!(fingerprint_ok(&res));
        assert_eq!(u16::from_be_bytes([res[0], res[1]]), BINDING_SUCCESS);
    }

    #[test]
    fn demux_ranges() {
        assert_eq!(classify(&[22]), Class::Dtls);
        assert_eq!(classify(&[0x80, 96]), Class::Rtp);
        assert!(is_rtcp(&[0x80, 200]));
        assert_eq!(classify(&[200]), Class::Other);
    }
}
