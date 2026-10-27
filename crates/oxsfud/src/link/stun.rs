// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! STUN(순수) — RFC 5389 머리 · 속성 · MESSAGE-INTEGRITY(HMAC-SHA1) · FINGERPRINT(CRC32) · Binding 성공 응답 짓기. 서버 이름은 `ox` 로 시작한다.

use std::net::SocketAddr;

use hmac::{Hmac, Mac};
use sha1::Sha1;

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
pub const SOFTWARE: &str = "oxsfud";
const FINGERPRINT_XOR: u32 = 0x5354_554e;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub msg_type: u16,
    pub txid: [u8; 12],
    pub attrs: Vec<(u16, usize, usize)>,
    pub raw: Vec<u8>,
}

pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= 20 && b[0] < 4 && u32::from_be_bytes([b[4], b[5], b[6], b[7]]) == MAGIC
}

pub fn parse(b: &[u8]) -> Option<Message> {
    if !is_stun(b) {
        return None;
    }
    let msg_type = u16::from_be_bytes([b[0], b[1]]);
    let len = usize::from(u16::from_be_bytes([b[2], b[3]]));
    if b.len() < 20 + len || !len.is_multiple_of(4) {
        return None;
    }
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&b[8..20]);
    let mut attrs = Vec::new();
    let mut i = 20;
    while i + 4 <= 20 + len {
        let t = u16::from_be_bytes([b[i], b[i + 1]]);
        let l = usize::from(u16::from_be_bytes([b[i + 2], b[i + 3]]));
        if i + 4 + l > 20 + len {
            return None;
        }
        attrs.push((t, i + 4, l));
        i += 4 + l.div_ceil(4) * 4;
    }
    Some(Message { msg_type, txid, attrs, raw: b[..20 + len].to_vec() })
}

impl Message {
    pub fn attr(&self, t: u16) -> Option<&[u8]> {
        self.attrs.iter().find(|a| a.0 == t).map(|a| &self.raw[a.1..a.1 + a.2])
    }

    pub fn has(&self, t: u16) -> bool {
        self.attrs.iter().any(|a| a.0 == t)
    }

    pub fn username(&self) -> Option<&str> {
        self.attr(ATTR_USERNAME).and_then(|v| std::str::from_utf8(v).ok())
    }

    pub fn local_ufrag(&self) -> Option<&str> {
        self.username().and_then(|u| u.split(':').next())
    }

    pub fn use_candidate(&self) -> bool {
        self.has(ATTR_USE_CANDIDATE)
    }

    pub fn verify_integrity(&self, key: &[u8]) -> bool {
        let Some(&(_, off, len)) = self.attrs.iter().find(|a| a.0 == ATTR_MESSAGE_INTEGRITY) else { return false };
        if len != 20 {
            return false;
        }
        let start = off - 4;
        let mut head = self.raw[..start].to_vec();
        let fake_len = (start + 24 - 20) as u16;
        head[2..4].copy_from_slice(&fake_len.to_be_bytes());
        hmac_sha1(key, &head).as_slice() == &self.raw[off..off + 20]
    }

    pub fn verify_fingerprint(&self) -> bool {
        let Some(&(_, off, len)) = self.attrs.iter().find(|a| a.0 == ATTR_FINGERPRINT) else { return true };
        if len != 4 {
            return false;
        }
        let start = off - 4;
        let mut head = self.raw[..start].to_vec();
        let fake_len = (start + 8 - 20) as u16;
        head[2..4].copy_from_slice(&fake_len.to_be_bytes());
        let want = crc32fast::hash(&head) ^ FINGERPRINT_XOR;
        want.to_be_bytes() == self.raw[off..off + 4]
    }
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    let Ok(mut m) = <Hmac<Sha1> as Mac>::new_from_slice(key) else { return Vec::new() };
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn push_attr(out: &mut Vec<u8>, t: u16, v: &[u8]) {
    out.extend_from_slice(&t.to_be_bytes());
    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
    out.extend_from_slice(v);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn set_len(out: &mut [u8], len: usize) {
    out[2..4].copy_from_slice(&(len as u16).to_be_bytes());
}

fn xor_address(addr: SocketAddr, txid: &[u8; 12]) -> Vec<u8> {
    let port = addr.port() ^ (MAGIC >> 16) as u16;
    let mut v = vec![0u8];
    match addr {
        SocketAddr::V4(a) => {
            v.push(1);
            v.extend_from_slice(&port.to_be_bytes());
            let ip = u32::from(*a.ip()) ^ MAGIC;
            v.extend_from_slice(&ip.to_be_bytes());
        }
        SocketAddr::V6(a) => {
            v.push(2);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = MAGIC.to_be_bytes().to_vec();
            key.extend_from_slice(txid);
            for (i, b) in a.ip().octets().iter().enumerate() {
                v.push(b ^ key[i]);
            }
        }
    }
    v
}

pub fn binding_success(txid: &[u8; 12], mapped: SocketAddr, key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(txid);
    push_attr(&mut out, ATTR_XOR_MAPPED_ADDRESS, &xor_address(mapped, txid));
    push_attr(&mut out, ATTR_SOFTWARE, SOFTWARE.as_bytes());
    let mi_len = out.len() + 24 - 20;
    set_len(&mut out, mi_len);
    let mac = hmac_sha1(key, &out);
    push_attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp_len = out.len() + 8 - 20;
    set_len(&mut out, fp_len);
    let crc = crc32fast::hash(&out) ^ FINGERPRINT_XOR;
    push_attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

pub fn binding_request(txid: &[u8; 12], username: &str, key: &[u8], use_candidate: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(txid);
    push_attr(&mut out, ATTR_USERNAME, username.as_bytes());
    push_attr(&mut out, ATTR_ICE_CONTROLLING, &[0u8; 8]);
    push_attr(&mut out, ATTR_PRIORITY, &2_113_937_151u32.to_be_bytes());
    if use_candidate {
        push_attr(&mut out, ATTR_USE_CANDIDATE, &[]);
    }
    let mi_len = out.len() + 24 - 20;
    set_len(&mut out, mi_len);
    let mac = hmac_sha1(key, &out);
    push_attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp_len = out.len() + 8 - 20;
    set_len(&mut out, fp_len);
    let crc = crc32fast::hash(&out) ^ FINGERPRINT_XOR;
    push_attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_and_integrity() {
        let tx = [7u8; 12];
        let b = binding_request(&tx, "srv12345:cli", b"pwd-of-server-22chars-", true);
        let m = parse(&b).expect("parse");
        assert_eq!(m.local_ufrag(), Some("srv12345"));
        assert!(m.use_candidate());
        assert!(m.verify_integrity(b"pwd-of-server-22chars-"));
        assert!(!m.verify_integrity(b"wrong"));
        assert!(m.verify_fingerprint());
        let r = binding_success(&tx, "10.0.0.1:5000".parse().expect("addr"), b"k");
        let p = parse(&r).expect("resp");
        assert_eq!(p.msg_type, BINDING_SUCCESS);
        assert!(p.verify_integrity(b"k"));
    }
}
