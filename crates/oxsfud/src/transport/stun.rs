// author: kodeholic (powered by Claude)
//! STUN Binding — 정§12 ICE-lite 행. 서버가 받는 것(Binding Request)과 내는 것(Success Response)만(RFC 8489).
//! 판정 순서(integrity → latch → 응답)는 `udp.rs` 가 지킨다 — 여기는 파싱·검증·조립뿐이다.

use std::net::SocketAddr;

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const MAGIC_COOKIE: u32 = 0x2112_A442;
pub const HEADER_LEN: usize = 20;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_RESPONSE: u16 = 0x0101;

const ATTR_USERNAME: u16 = 0x0006;
const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_USE_CANDIDATE: u16 = 0x0025;
const ATTR_FINGERPRINT: u16 = 0x8028;
const ATTR_SOFTWARE: u16 = 0x8022;

const FAMILY_V4: u8 = 0x01;
const FAMILY_V6: u8 = 0x02;

/// 속성 헤더 4B + HMAC-SHA1 20B (RFC 8489 §14.5 — 길이 필드가 이것을 포함해야 한다).
const MI_ATTR_LEN: usize = 24;
/// 속성 헤더 4B + CRC32 4B (RFC 8489 §14.7).
const FP_ATTR_LEN: usize = 8;
const FINGERPRINT_XOR: u32 = 0x5354_554E;

/// 정 부록 C-15 — 관측에 남는 이름은 `ox-` 접두.
const SOFTWARE: &[u8] = b"ox-sfu";

#[derive(Debug)]
pub struct Message<'a> {
    pub msg_type: u16,
    pub transaction_id: [u8; 12],
    attrs: Vec<(u16, &'a [u8])>,
    raw: &'a [u8],
}

/// `None` = STUN 이 아니거나 형식이 깨졌다. 예외를 던지지 않는다.
pub fn parse(buf: &[u8]) -> Option<Message<'_>> {
    if buf.len() < HEADER_LEN || buf[0] & 0xC0 != 0 {
        return None;
    }
    if u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) != MAGIC_COOKIE {
        return None;
    }
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if !len.is_multiple_of(4) || buf.len() < HEADER_LEN + len {
        return None;
    }
    let total = HEADER_LEN + len;
    let mut attrs = Vec::new();
    let mut at = HEADER_LEN;
    while at + 4 <= total {
        let t = u16::from_be_bytes([buf[at], buf[at + 1]]);
        let l = u16::from_be_bytes([buf[at + 2], buf[at + 3]]) as usize;
        at += 4;
        if at + l > total {
            break;
        }
        attrs.push((t, &buf[at..at + l]));
        at += l.next_multiple_of(4);
    }
    let mut transaction_id = [0u8; 12];
    transaction_id.copy_from_slice(&buf[8..20]);
    Some(Message { msg_type: u16::from_be_bytes([buf[0], buf[1]]), transaction_id, attrs, raw: &buf[..total] })
}

impl Message<'_> {
    fn attr(&self, t: u16) -> Option<&[u8]> {
        self.attrs.iter().find(|(k, _)| *k == t).map(|(_, v)| *v)
    }

    /// ICE USERNAME 은 `{서버 ufrag}:{클라 ufrag}` — 앞자리가 조회 키다(연§9-3).
    pub fn server_ufrag(&self) -> Option<&str> {
        let u = std::str::from_utf8(self.attr(ATTR_USERNAME)?).ok()?;
        u.split(':').next().filter(|s| !s.is_empty())
    }

    /// 클라가 이 후보를 지명했다 = DTLS 를 시작할 때다.
    pub fn has_use_candidate(&self) -> bool {
        self.attr(ATTR_USE_CANDIDATE).is_some()
    }

    /// 정§12 — 이것이 참일 때에만 latch·응답·`last_seen` 으로 넘어간다. 순서가 곧 방어다.
    pub fn verify_integrity(&self, pwd: &str) -> bool {
        let Some(got) = self.attr(ATTR_MESSAGE_INTEGRITY) else { return false };
        let Some(at) = attr_offset(self.raw, ATTR_MESSAGE_INTEGRITY) else { return false };
        let mut head = self.raw[..at].to_vec();
        let claimed = (at - HEADER_LEN + MI_ATTR_LEN) as u16;
        head[2..4].copy_from_slice(&claimed.to_be_bytes());
        let want = hmac_sha1(&head, pwd.as_bytes());
        want.len() == got.len() && want.iter().zip(got).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
    }
}

/// Success Response — XOR-MAPPED-ADDRESS · SOFTWARE · MESSAGE-INTEGRITY · FINGERPRINT.
pub fn binding_response(transaction_id: &[u8; 12], remote: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(transaction_id);
    push_attr(&mut buf, ATTR_XOR_MAPPED_ADDRESS, &xor_mapped_address(remote, transaction_id));
    push_attr(&mut buf, ATTR_SOFTWARE, SOFTWARE);

    set_len(&mut buf, MI_ATTR_LEN);
    let mac = hmac_sha1(&buf, pwd.as_bytes());
    push_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, &mac);

    set_len(&mut buf, FP_ATTR_LEN);
    let crc = crc32fast::hash(&buf) ^ FINGERPRINT_XOR;
    push_attr(&mut buf, ATTR_FINGERPRINT, &crc.to_be_bytes());

    set_len(&mut buf, 0);
    buf
}

fn set_len(buf: &mut [u8], extra: usize) {
    let len = (buf.len() - HEADER_LEN + extra) as u16;
    buf[2..4].copy_from_slice(&len.to_be_bytes());
}

fn push_attr(buf: &mut Vec<u8>, t: u16, v: &[u8]) {
    buf.extend_from_slice(&t.to_be_bytes());
    buf.extend_from_slice(&(v.len() as u16).to_be_bytes());
    buf.extend_from_slice(v);
    buf.extend(std::iter::repeat_n(0u8, v.len().next_multiple_of(4) - v.len()));
}

fn attr_offset(raw: &[u8], target: u16) -> Option<usize> {
    let mut at = HEADER_LEN;
    while at + 4 <= raw.len() {
        if u16::from_be_bytes([raw[at], raw[at + 1]]) == target {
            return Some(at);
        }
        at += 4 + (u16::from_be_bytes([raw[at + 2], raw[at + 3]]) as usize).next_multiple_of(4);
    }
    None
}

fn xor_mapped_address(addr: SocketAddr, transaction_id: &[u8; 12]) -> Vec<u8> {
    let cookie = MAGIC_COOKIE.to_be_bytes();
    let mut out = vec![0x00];
    out.push(if addr.is_ipv4() { FAMILY_V4 } else { FAMILY_V6 });
    out.extend_from_slice(&(addr.port() ^ (MAGIC_COOKIE >> 16) as u16).to_be_bytes());
    match addr {
        SocketAddr::V4(v4) => out.extend(v4.ip().octets().iter().zip(cookie).map(|(b, c)| b ^ c)),
        SocketAddr::V6(v6) => {
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&cookie);
            key[4..].copy_from_slice(transaction_id);
            out.extend(v6.ip().octets().iter().zip(key).map(|(b, c)| b ^ c));
        }
    }
    out
}

fn hmac_sha1(data: &[u8], key: &[u8]) -> [u8; 20] {
    let mut mac = <Hmac<Sha1>>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(username: &str, pwd: &str, use_candidate: bool) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        buf.extend_from_slice(&[7u8; 12]);
        push_attr(&mut buf, ATTR_USERNAME, username.as_bytes());
        if use_candidate {
            push_attr(&mut buf, ATTR_USE_CANDIDATE, &[]);
        }
        set_len(&mut buf, MI_ATTR_LEN);
        let mac = hmac_sha1(&buf, pwd.as_bytes());
        push_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, &mac);
        set_len(&mut buf, 0);
        buf
    }

    #[test]
    fn request_parse_and_integrity_gate() {
        let raw = request("srv0001:cli", "s3cret", true);
        let m = parse(&raw).unwrap();
        assert_eq!((m.msg_type, m.server_ufrag(), m.has_use_candidate()), (BINDING_REQUEST, Some("srv0001"), true));
        assert!(m.verify_integrity("s3cret"));
        assert!(!m.verify_integrity("other"), "ufrag 를 아는 제3자의 위조는 여기서 멈춘다");
        assert!(!parse(&request("u:c", "p", false)).unwrap().has_use_candidate());
        assert!(parse(&raw[..12]).is_none());
        let mut bad = raw.clone();
        bad[4] = 0;
        assert!(parse(&bad).is_none(), "magic cookie 가 아니면 STUN 이 아니다");
    }

    #[test]
    fn response_carries_xor_address_and_verifies() {
        let addr: SocketAddr = "203.0.113.9:41234".parse().unwrap();
        let res = binding_response(&[7u8; 12], addr, "s3cret");
        let m = parse(&res).unwrap();
        assert_eq!(m.msg_type, BINDING_RESPONSE);
        assert!(m.verify_integrity("s3cret"));
        let xma = m.attr(ATTR_XOR_MAPPED_ADDRESS).unwrap();
        assert_eq!((xma[1], u16::from_be_bytes([xma[2], xma[3]])), (FAMILY_V4, 41234 ^ 0x2112));
        assert_eq!(&xma[4..], &[203 ^ 0x21, 0x12, 113 ^ 0xA4, 9 ^ 0x42]);
        assert_eq!(m.attr(ATTR_SOFTWARE), Some(&b"ox-sfu"[..]));
        let at = attr_offset(&res, ATTR_FINGERPRINT).unwrap();
        let mut head = res[..at].to_vec();
        head[2..4].copy_from_slice(&((at - HEADER_LEN + FP_ATTR_LEN) as u16).to_be_bytes());
        assert_eq!(m.attr(ATTR_FINGERPRINT), Some(&(crc32fast::hash(&head) ^ FINGERPRINT_XOR).to_be_bytes()[..]));
    }

    #[test]
    fn v6_address_uses_transaction_id_in_key() {
        let addr: SocketAddr = "[2001:db8::1]:5000".parse().unwrap();
        let v = xor_mapped_address(addr, &[9u8; 12]);
        assert_eq!((v[1], v.len()), (FAMILY_V6, 20));
        assert_eq!(v[8], 9);
    }
}
