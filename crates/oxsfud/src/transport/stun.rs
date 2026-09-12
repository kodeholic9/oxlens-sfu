// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · model: claude-opus-5

//! STUN — ★**ICE-lite 가 받는 유일한 프레임**(RFC 8489).
//!
//! ★★**순서가 곧 방어다**(정§12): ufrag 조회 → ★**message-integrity 검증** → 주소 latch → 응답.
//! 검증 전에 latch 를 옮기면 ★**위조 Binding 하나로 미디어가 엉뚱한 주소로 간다** — ufrag 는
//! `server_config` 를 본 사람이면 다 아는 값이라 ufrag 만으로는 아무것도 못 막는다.
//!
//! 형상 — 머리 20바이트(`type`·`length`·magic·txid 12) + 속성 TLV(값은 4바이트 정렬).

use std::net::SocketAddr;

pub const MAGIC_COOKIE: u32 = 0x2112_A442;
pub const HEADER_SIZE: usize = 20;

pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_RESPONSE: u16 = 0x0101;

pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_USE_CANDIDATE: u16 = 0x0025;
pub const ATTR_FINGERPRINT: u16 = 0x8028;
pub const ATTR_SOFTWARE: u16 = 0x8022;

/// MESSAGE-INTEGRITY 속성 한 벌의 크기(머리 4 + HMAC-SHA1 20).
const MI_ATTR_SIZE: usize = 24;
/// FINGERPRINT 속성 한 벌의 크기(머리 4 + CRC 4).
const FP_ATTR_SIZE: usize = 8;

const ADDR_FAMILY_IPV4: u8 = 0x01;
const ADDR_FAMILY_IPV6: u8 = 0x02;

/// 읽어 낸 메시지. ★**값을 복사하지 않는다** — 원본 버퍼를 가리킨다(핫패스 규율 H1).
#[derive(Debug)]
pub struct Message<'a> {
    pub msg_type: u16,
    pub transaction_id: [u8; 12],
    attrs: Vec<Attr<'a>>,
    /// MESSAGE-INTEGRITY 계산에 쓰는 원문.
    raw: &'a [u8],
}

#[derive(Debug)]
struct Attr<'a> {
    attr_type: u16,
    value: &'a [u8],
}

/// ★**모양이 아니면 `None`** — 여기서 걸러야 그 뒤가 전부 STUN 이라고 믿을 수 있다.
pub fn parse(buf: &[u8]) -> Option<Message<'_>> {
    if buf.len() < HEADER_SIZE || buf[0] & 0xC0 != 0 {
        return None;
    }
    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) != MAGIC_COOKIE || !length.is_multiple_of(4) {
        return None;
    }
    let total = HEADER_SIZE + length;
    if buf.len() < total {
        return None;
    }
    let mut transaction_id = [0u8; 12];
    transaction_id.copy_from_slice(&buf[8..20]);

    let mut attrs = Vec::new();
    let mut at = HEADER_SIZE;
    while at + 4 <= total {
        let attr_type = u16::from_be_bytes([buf[at], buf[at + 1]]);
        let len = u16::from_be_bytes([buf[at + 2], buf[at + 3]]) as usize;
        at += 4;
        if at + len > total {
            break;
        }
        attrs.push(Attr { attr_type, value: &buf[at..at + len] });
        // ★값은 4바이트 정렬이다 — 채움을 건너뛴다.
        at += len.next_multiple_of(4);
    }
    Some(Message { msg_type, transaction_id, attrs, raw: &buf[..total] })
}

impl<'a> Message<'a> {
    fn attr(&self, t: u16) -> Option<&Attr<'a>> {
        self.attrs.iter().find(|a| a.attr_type == t)
    }

    /// `USERNAME` = `{서버 ufrag}:{클라 ufrag}` — ★**앞쪽이 우리를 가리킨다**(RFC 8445 §7.2.2).
    pub fn username(&self) -> Option<&'a str> {
        self.attr(ATTR_USERNAME).and_then(|a| std::str::from_utf8(a.value).ok())
    }

    /// 그 요청이 가리키는 **우리** ufrag.
    pub fn local_ufrag(&self) -> Option<&'a str> {
        self.username().map(|u| u.split(':').next().unwrap_or(u))
    }

    pub fn has_use_candidate(&self) -> bool {
        self.attr(ATTR_USE_CANDIDATE).is_some()
    }

    pub fn is_binding_request(&self) -> bool {
        self.msg_type == BINDING_REQUEST
    }

    /// ★**이것이 방어다** — 참이 아니면 latch 를 건드리지 않는다.
    pub fn integrity_ok(&self, pwd: &str) -> bool {
        let Some(got) = self.attr(ATTR_MESSAGE_INTEGRITY).map(|a| a.value) else {
            // ★없으면 실패다 — *"안 실었으니 통과"* 로 두면 방어가 통째로 없는 것과 같다.
            return false;
        };
        let Some(at) = attr_offset(self.raw, ATTR_MESSAGE_INTEGRITY) else {
            return false;
        };
        // ★HMAC 의 입력은 **그 속성 앞까지**이고, 머리의 길이는 ★**그 속성까지 실은 값**으로 바꾼다.
        let mut tmp = self.raw[..at].to_vec();
        let adjusted = (at - HEADER_SIZE + MI_ATTR_SIZE) as u16;
        tmp[2..4].copy_from_slice(&adjusted.to_be_bytes());
        let want = hmac_sha1(&tmp, pwd.as_bytes());
        // ★**시간 불변 비교** — 바이트마다 일찍 끊으면 HMAC 을 한 바이트씩 맞춰 갈 수 있다.
        want.len() == got.len() && want.iter().zip(got).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
    }
}

/// Binding 성공 응답. ★**관측된 출발 주소를 그대로 돌려준다** — 그것이 latch 의 근거다.
pub fn binding_response(transaction_id: &[u8; 12], remote: SocketAddr, pwd: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    buf.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // 길이는 아래에서 세 번 고쳐 쓴다
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(transaction_id);

    let xma = xor_mapped_address(remote, transaction_id);
    write_attr(&mut buf, ATTR_XOR_MAPPED_ADDRESS, &xma);
    write_attr(&mut buf, ATTR_SOFTWARE, b"oxlens-sfu");

    // ★길이를 **먼저** 고쳐야 한다 — HMAC 이 머리까지 덮으므로 나중에 고치면 서명이 깨진다.
    set_len(&mut buf, MI_ATTR_SIZE);
    let mac = hmac_sha1(&buf, pwd.as_bytes());
    write_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, &mac);

    set_len(&mut buf, FP_ATTR_SIZE);
    let crc = crc32fast::hash(&buf) ^ 0x5354_554E;
    write_attr(&mut buf, ATTR_FINGERPRINT, &crc.to_be_bytes());

    set_len(&mut buf, 0);
    buf
}

/// 머리의 길이 칸 = 지금 몸통 + `extra`(아직 안 쓴 속성 한 벌).
fn set_len(buf: &mut [u8], extra: usize) {
    let v = (buf.len() - HEADER_SIZE + extra) as u16;
    buf[2..4].copy_from_slice(&v.to_be_bytes());
}

fn write_attr(buf: &mut Vec<u8>, attr_type: u16, value: &[u8]) {
    buf.extend_from_slice(&attr_type.to_be_bytes());
    buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buf.extend_from_slice(value);
    buf.extend(std::iter::repeat_n(0u8, value.len().next_multiple_of(4) - value.len()));
}

fn xor_mapped_address(addr: SocketAddr, transaction_id: &[u8; 12]) -> Vec<u8> {
    let mut v = Vec::with_capacity(20);
    v.push(0x00);
    let port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
    match addr {
        SocketAddr::V4(a) => {
            v.push(ADDR_FAMILY_IPV4);
            v.extend_from_slice(&port.to_be_bytes());
            let cookie = MAGIC_COOKIE.to_be_bytes();
            v.extend(a.ip().octets().iter().zip(cookie).map(|(b, c)| b ^ c));
        }
        SocketAddr::V6(a) => {
            v.push(ADDR_FAMILY_IPV6);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
            key[4..].copy_from_slice(transaction_id);
            v.extend(a.ip().octets().iter().zip(key).map(|(b, c)| b ^ c));
        }
    }
    v
}

fn hmac_sha1(data: &[u8], key: &[u8]) -> [u8; 20] {
    use hmac::{Hmac, Mac};
    let mut mac = <Hmac<sha1::Sha1>>::new_from_slice(key).expect("HMAC 은 어떤 길이의 키도 받는다");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// ★**시험용 요청 지음꾼** — 우리 검증기를 우리 형상으로 먼저 때린다.
/// 진짜 교차 검증은 2층이 남의 구현(aiortc/aioice)으로 한다 — ★**자기 서명을 자기가 맞히는
/// 것만으로는 계약이 섰다고 말할 수 없다.**
#[cfg(test)]
pub fn binding_request(transaction_id: &[u8; 12], username: &str, pwd: &str, use_candidate: bool) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    buf.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    buf.extend_from_slice(transaction_id);
    write_attr(&mut buf, ATTR_USERNAME, username.as_bytes());
    if use_candidate {
        write_attr(&mut buf, ATTR_USE_CANDIDATE, &[]);
    }
    set_len(&mut buf, MI_ATTR_SIZE);
    let mac = hmac_sha1(&buf, pwd.as_bytes());
    write_attr(&mut buf, ATTR_MESSAGE_INTEGRITY, &mac);
    set_len(&mut buf, FP_ATTR_SIZE);
    let crc = crc32fast::hash(&buf) ^ 0x5354_554E;
    write_attr(&mut buf, ATTR_FINGERPRINT, &crc.to_be_bytes());
    set_len(&mut buf, 0);
    buf
}

fn attr_offset(raw: &[u8], target: u16) -> Option<usize> {
    let mut at = HEADER_SIZE;
    while at + 4 <= raw.len() {
        let t = u16::from_be_bytes([raw[at], raw[at + 1]]);
        let len = u16::from_be_bytes([raw[at + 2], raw[at + 3]]) as usize;
        if t == target {
            return Some(at);
        }
        at += 4 + len.next_multiple_of(4);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PWD: &str = "zxcvbnmasdfghjklqwerty";

    /// 응답을 지어 되읽는다 — 우리 서명이 우리 검증을 통과해야 한다.
    fn round_trip(pwd_sign: &str, pwd_check: &str) -> bool {
        let txid = [7u8; 12];
        let wire = binding_response(&txid, "203.0.113.9:51234".parse().expect("주소"), pwd_sign);
        let m = parse(&wire).expect("우리가 지은 것은 읽힌다");
        m.integrity_ok(pwd_check)
    }

    #[test]
    fn 서명은_비밀번호가_맞을_때만_선다() {
        assert!(round_trip(PWD, PWD));
        // ★**ufrag 를 알아도 pwd 가 틀리면 못 지나간다** — 그것이 위조 Binding 방어의 전부다.
        assert!(!round_trip(PWD, "wrong-password-000000"));
    }

    #[test]
    fn 응답은_길이_세_번을_제대로_적는다() {
        let wire = binding_response(&[1u8; 12], "10.0.0.1:5000".parse().expect("주소"), PWD);
        let body = u16::from_be_bytes([wire[2], wire[3]]) as usize;
        // ★마지막 길이는 FINGERPRINT 까지 **다** 센 값이다 — 아니면 읽는 쪽이 끝을 못 찾는다.
        assert_eq!(body, wire.len() - HEADER_SIZE);
        assert!(parse(&wire).expect("읽힌다").attr(ATTR_FINGERPRINT).is_some());
    }

    #[test]
    fn 서명이_없으면_실패다() {
        // 속성 없는 맨 Binding 요청.
        let mut wire = Vec::new();
        wire.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        wire.extend_from_slice(&[3u8; 12]);
        let m = parse(&wire).expect("읽힌다");
        assert!(m.is_binding_request());
        // ★*"안 실었으니 통과"* 로 두면 방어가 통째로 없다.
        assert!(!m.integrity_ok(PWD));
    }

    #[test]
    fn username_의_앞쪽이_우리_ufrag_다() {
        let wire = binding_request(&[3u8; 12], "srvufrag:botufrag", PWD, true);
        let m = parse(&wire).expect("읽힌다");
        assert_eq!(m.local_ufrag(), Some("srvufrag"));
        assert!(m.is_binding_request() && m.has_use_candidate());
        // ★USERNAME 이 서명 **앞**에 있어야 한다 — 뒤에 있으면 서명이 그것을 안 덮는다.
        assert!(m.integrity_ok(PWD));
    }

    #[test]
    fn 매직이_아니면_stun_이_아니다() {
        let mut wire = vec![0u8; 20];
        wire[4..8].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        assert!(parse(&wire).is_none());
    }
}
