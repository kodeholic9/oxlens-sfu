// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-LINK-TRANSPORT · 정:RULE-LINK-PAIR-LIVENESS · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! STUN Binding(ICE-lite) — 읽기 · ★MESSAGE-INTEGRITY 검증(요청 USERNAME 의 ufrag 의 pwd) · 응답 짓기. 검증 전에는 아무것도 바꾸지 않는다.

use std::net::SocketAddr;

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const MAGIC: u32 = 0x2112_a442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_USE_CANDIDATE: u16 = 0x0025;
pub const ATTR_SOFTWARE: u16 = 0x8022;
pub const ATTR_FINGERPRINT: u16 = 0x8028;
pub const SOFTWARE: &str = "oxsfud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub txid: [u8; 12],
    pub local_ufrag: String,
    pub remote_ufrag: String,
    pub use_candidate: bool,
    mi_offset: usize,
    mi: [u8; 20],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StunError {
    NotStun,
    NotBinding,
    Malformed,
    NoUsername,
    NoIntegrity,
    UnknownUfrag,
    BadIntegrity,
}

pub fn is_stun(b: &[u8]) -> bool {
    b.len() >= 20 && b[0] < 4 && u32::from_be_bytes([b[4], b[5], b[6], b[7]]) == MAGIC
}

pub fn parse(b: &[u8]) -> Result<Binding, StunError> {
    if !is_stun(b) {
        return Err(StunError::NotStun);
    }
    let ty = u16::from_be_bytes([b[0], b[1]]);
    if ty != BINDING_REQUEST {
        return Err(StunError::NotBinding);
    }
    let len = u16::from_be_bytes([b[2], b[3]]) as usize;
    if b.len() < 20 + len {
        return Err(StunError::Malformed);
    }
    let mut txid = [0u8; 12];
    txid.copy_from_slice(&b[8..20]);
    let mut i = 20;
    let mut username: Option<String> = None;
    let mut mi: Option<(usize, [u8; 20])> = None;
    let mut use_candidate = false;
    while i + 4 <= 20 + len {
        let at = u16::from_be_bytes([b[i], b[i + 1]]);
        let al = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let v = i + 4;
        if v + al > b.len() {
            return Err(StunError::Malformed);
        }
        match at {
            ATTR_USERNAME => username = Some(String::from_utf8_lossy(&b[v..v + al]).into_owned()),
            ATTR_MESSAGE_INTEGRITY if al == 20 => {
                let mut m = [0u8; 20];
                m.copy_from_slice(&b[v..v + 20]);
                mi = Some((i, m));
            }
            ATTR_USE_CANDIDATE if mi.is_none() => use_candidate = true,
            _ => {}
        }
        if mi.is_some() && at != ATTR_MESSAGE_INTEGRITY {
            break;
        }
        i = v + al.div_ceil(4) * 4;
    }
    let u = username.ok_or(StunError::NoUsername)?;
    let (local, remote) = u.split_once(':').ok_or(StunError::NoUsername)?;
    let (mi_offset, mi) = mi.ok_or(StunError::NoIntegrity)?;
    Ok(Binding { txid, local_ufrag: local.to_string(), remote_ufrag: remote.to_string(), use_candidate, mi_offset, mi })
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 20] {
    let mut m = match <Hmac<Sha1> as Mac>::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return [0; 20],
    };
    for p in parts {
        m.update(p);
    }
    let out = m.finalize().into_bytes();
    let mut r = [0u8; 20];
    r.copy_from_slice(&out);
    r
}

pub fn verify(raw: &[u8], b: &Binding, pwd: &str) -> bool {
    let mut head = raw[..b.mi_offset].to_vec();
    let new_len = (b.mi_offset - 20 + 24) as u16;
    head[2..4].copy_from_slice(&new_len.to_be_bytes());
    let calc = hmac(pwd.as_bytes(), &[&head]);
    calc.iter().zip(b.mi.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn check<'a, F>(raw: &[u8], pwd_of: F) -> Result<Binding, StunError>
where
    F: Fn(&str) -> Option<&'a str>,
{
    let b = parse(raw)?;
    let pwd = pwd_of(&b.local_ufrag).ok_or(StunError::UnknownUfrag)?;
    if !verify(raw, &b, pwd) {
        return Err(StunError::BadIntegrity);
    }
    Ok(b)
}

fn attr(out: &mut Vec<u8>, ty: u16, v: &[u8]) {
    out.extend_from_slice(&ty.to_be_bytes());
    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
    out.extend_from_slice(v);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

pub fn success(txid: &[u8; 12], from: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(txid);
    let mut xa = Vec::with_capacity(20);
    xa.push(0);
    let port = from.port() ^ (MAGIC >> 16) as u16;
    match from {
        SocketAddr::V4(a) => {
            xa.push(1);
            xa.extend_from_slice(&port.to_be_bytes());
            let ip = u32::from(*a.ip()) ^ MAGIC;
            xa.extend_from_slice(&ip.to_be_bytes());
        }
        SocketAddr::V6(a) => {
            xa.push(2);
            xa.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&MAGIC.to_be_bytes());
            key[4..].copy_from_slice(txid);
            let ip = a.ip().octets();
            xa.extend(ip.iter().zip(key.iter()).map(|(x, k)| x ^ k));
        }
    }
    attr(&mut out, ATTR_XOR_MAPPED_ADDRESS, &xa);
    attr(&mut out, ATTR_SOFTWARE, SOFTWARE.as_bytes());
    let mi_len = (out.len() - 20 + 24) as u16;
    out[2..4].copy_from_slice(&mi_len.to_be_bytes());
    let mac = hmac(pwd.as_bytes(), &[&out]);
    attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp_len = (out.len() - 20 + 8) as u16;
    out[2..4].copy_from_slice(&fp_len.to_be_bytes());
    let crc = crc32fast::hash(&out) ^ 0x5354_554e;
    attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

pub fn request(local_ufrag: &str, remote_ufrag: &str, pwd: &str, txid: [u8; 12], use_candidate: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.extend_from_slice(&txid);
    attr(&mut out, ATTR_USERNAME, format!("{local_ufrag}:{remote_ufrag}").as_bytes());
    if use_candidate {
        attr(&mut out, ATTR_USE_CANDIDATE, &[]);
    }
    let mi_len = (out.len() - 20 + 24) as u16;
    out[2..4].copy_from_slice(&mi_len.to_be_bytes());
    let mac = hmac(pwd.as_bytes(), &[&out]);
    attr(&mut out, ATTR_MESSAGE_INTEGRITY, &mac);
    let fp_len = (out.len() - 20 + 8) as u16;
    out[2..4].copy_from_slice(&fp_len.to_be_bytes());
    let crc = crc32fast::hash(&out) ^ 0x5354_554e;
    attr(&mut out, ATTR_FINGERPRINT, &crc.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_verify() {
        let r = request("srv1", "cli1", "pw", [7; 12], true);
        let b = check(&r, |u| if u == "srv1" { Some("pw") } else { None }).unwrap();
        assert!(b.use_candidate);
        assert_eq!(check(&r, |_| Some("other")).unwrap_err(), StunError::BadIntegrity);
        let s = success(&b.txid, "127.0.0.1:5000".parse().unwrap(), "pw");
        assert_eq!(u16::from_be_bytes([s[0], s[1]]), BINDING_SUCCESS);
    }
}
