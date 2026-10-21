// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:RULE-LINK-TRANSPORT RFC 8489 RFC 8445 §7.3 · model: claude-opus-5-5
//! STUN Binding — ICE-lite 서버 쪽. message-integrity 를 먼저 검증하고 그 뒤에만 처리한다.

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const MAGIC: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const BINDING_ERROR: u16 = 0x0111;
const ATTR_USERNAME: u16 = 0x0006;
const ATTR_MI: u16 = 0x0008;
const ATTR_ERROR_CODE: u16 = 0x0009;
const ATTR_XOR_MAPPED: u16 = 0x0020;
const ATTR_USE_CANDIDATE: u16 = 0x0025;
const ATTR_SOFTWARE: u16 = 0x8022;
const ATTR_FINGERPRINT: u16 = 0x8028;
const SOFTWARE: &str = "ox-sfu";

pub struct Request<'a> {
    pub raw: &'a [u8],
    pub txid: [u8; 12],
    pub username: Option<String>,
    pub use_candidate: bool,
    mi_at: Option<usize>,
}

pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= 20 && b[0] < 4 && u32::from_be_bytes([b[4], b[5], b[6], b[7]]) == MAGIC
}

pub fn parse(b: &[u8]) -> Option<Request<'_>> {
    if !is_stun(b) {
        return None;
    }
    let typ = u16::from_be_bytes([b[0], b[1]]);
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if typ != BINDING_REQUEST || b.len() < 20 + len {
        return None;
    }
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&b[8..20]);
    let mut r = Request { raw: &b[..20 + len], txid, username: None, use_candidate: false, mi_at: None };
    let mut i = 20;
    while i + 4 <= 20 + len {
        let t = u16::from_be_bytes([b[i], b[i + 1]]);
        let l = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let v0 = i + 4;
        if v0 + l > b.len() {
            return None;
        }
        match t {
            ATTR_USERNAME => r.username = String::from_utf8(b[v0..v0 + l].to_vec()).ok(),
            ATTR_USE_CANDIDATE => r.use_candidate = true,
            ATTR_MI => {
                r.mi_at = Some(i);
                break;
            }
            _ => {}
        }
        i = v0 + l.div_ceil(4) * 4;
    }
    Some(r)
}

impl Request<'_> {
    pub fn local_ufrag(&self) -> Option<&str> {
        self.username.as_deref().and_then(|u| u.split(':').next())
    }

    pub fn verify(&self, pwd: &str) -> bool {
        let Some(at) = self.mi_at else { return false };
        if self.raw.len() < at + 24 {
            return false;
        }
        let mut head = self.raw[..at].to_vec();
        let mlen = (at + 24 - 20) as u16;
        head[2..4].copy_from_slice(&mlen.to_be_bytes());
        let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(pwd.as_bytes()) else { return false };
        mac.update(&head);
        mac.verify_slice(&self.raw[at + 4..at + 24]).is_ok()
    }
}

fn attr(out: &mut Vec<u8>, t: u16, v: &[u8]) {
    out.extend_from_slice(&t.to_be_bytes());
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

fn finish(mut out: Vec<u8>, pwd: Option<&str>) -> Vec<u8> {
    if let Some(p) = pwd
        && let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(p.as_bytes())
    {
        set_len(&mut out, 24);
        mac.update(&out);
        let tag = mac.finalize().into_bytes();
        attr(&mut out, ATTR_MI, &tag);
    }
    set_len(&mut out, 8);
    let crc = crc32fast::hash(&out) ^ 0x5354_554e;
    attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

fn head(typ: u16, txid: &[u8; 12]) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&typ.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(txid);
    out
}

pub fn success(txid: &[u8; 12], from: std::net::SocketAddr, pwd: &str) -> Vec<u8> {
    let mut out = head(BINDING_SUCCESS, txid);
    let mut v = Vec::with_capacity(20);
    let port = from.port() ^ (MAGIC >> 16) as u16;
    match from.ip() {
        std::net::IpAddr::V4(ip) => {
            v.extend_from_slice(&[0, 1]);
            v.extend_from_slice(&port.to_be_bytes());
            let x = u32::from(ip) ^ MAGIC;
            v.extend_from_slice(&x.to_be_bytes());
        }
        std::net::IpAddr::V6(ip) => {
            v.extend_from_slice(&[0, 2]);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&MAGIC.to_be_bytes());
            key[4..].copy_from_slice(txid);
            let o = ip.octets();
            for i in 0..16 {
                v.push(o[i] ^ key[i]);
            }
        }
    }
    attr(&mut out, ATTR_XOR_MAPPED, &v);
    attr(&mut out, ATTR_SOFTWARE, SOFTWARE.as_bytes());
    finish(out, Some(pwd))
}

pub fn unauthorized(txid: &[u8; 12]) -> Vec<u8> {
    let mut out = head(BINDING_ERROR, txid);
    let mut v = vec![0, 0, 4, 1];
    v.extend_from_slice(b"Unauthorized");
    attr(&mut out, ATTR_ERROR_CODE, &v);
    attr(&mut out, ATTR_SOFTWARE, SOFTWARE.as_bytes());
    finish(out, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(user: &str, pwd: &str, use_cand: bool) -> Vec<u8> {
        let mut out = head(BINDING_REQUEST, &[7u8; 12]);
        attr(&mut out, ATTR_USERNAME, user.as_bytes());
        if use_cand {
            attr(&mut out, ATTR_USE_CANDIDATE, &[]);
        }
        finish(out, Some(pwd))
    }

    #[test]
    fn verify_and_answer() {
        let req = request("srvufrag:cli", "secretpwd", true);
        let r = parse(&req).expect("parse");
        assert_eq!(r.local_ufrag(), Some("srvufrag"));
        assert!(r.use_candidate);
        assert!(r.verify("secretpwd"));
        assert!(!r.verify("wrong"));
        let res = success(&r.txid, "127.0.0.1:5000".parse().expect("addr"), "secretpwd");
        assert_eq!(u16::from_be_bytes([res[0], res[1]]), BINDING_SUCCESS);
        assert!(is_stun(&res));
    }
}
