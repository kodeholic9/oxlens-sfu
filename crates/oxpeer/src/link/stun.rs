// author: kodeholic (powered by Claude)
//! STUN Binding(RFC 5389 · RFC 8445) — 서버는 ICE-lite(controlled) · 클라는 controlling · 지명은 USE-CANDIDATE.
//! USERNAME = `서버 ufrag:클라 ufrag` · MESSAGE-INTEGRITY 는 서버 pwd · 같은 요청을 주기로 다시 보내는 것이 consent 다(RFC 7675).

const MAGIC_COOKIE: u32 = 0x2112_A442;
const HEADER: usize = 20;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_USERNAME: u16 = 0x0006;
const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
const ATTR_PRIORITY: u16 = 0x0024;
const ATTR_USE_CANDIDATE: u16 = 0x0025;
const ATTR_ICE_CONTROLLING: u16 = 0x802A;
const ATTR_FINGERPRINT: u16 = 0x8028;
/// 호스트 후보 우선순위(연:RULE-LINK-SDP-COMMON `a=candidate` 고정값과 같은 꼴)
const PRIORITY: u32 = 2_113_937_151;

/// 요청 하나 — (바이트, 트랜잭션 id)
pub(crate) fn binding_request(server_ufrag: &str, local_ufrag: &str, server_pwd: &str, tie: u64) -> (Vec<u8>, [u8; 12]) {
    let mut tid = [0u8; 12];
    let _ = getrandom::fill(&mut tid);
    let mut b = Vec::with_capacity(128);
    b.extend_from_slice(&BINDING_REQUEST.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    b.extend_from_slice(&tid);
    attr(&mut b, ATTR_USERNAME, format!("{server_ufrag}:{local_ufrag}").as_bytes());
    attr(&mut b, ATTR_PRIORITY, &PRIORITY.to_be_bytes());
    attr(&mut b, ATTR_ICE_CONTROLLING, &tie.to_be_bytes());
    attr(&mut b, ATTR_USE_CANDIDATE, &[]);
    let len_mi = (b.len() - HEADER + 24) as u16;
    b[2..4].copy_from_slice(&len_mi.to_be_bytes());
    let mac = hmac_sha1(&b, server_pwd.as_bytes());
    attr(&mut b, ATTR_MESSAGE_INTEGRITY, &mac);
    let len_fp = (b.len() - HEADER + 8) as u16;
    b[2..4].copy_from_slice(&len_fp.to_be_bytes());
    let crc = crc32fast::hash(&b) ^ 0x5354_554E;
    attr(&mut b, ATTR_FINGERPRINT, &crc.to_be_bytes());
    let fin = (b.len() - HEADER) as u16;
    b[2..4].copy_from_slice(&fin.to_be_bytes());
    (b, tid)
}

/// 성공 응답이면 그 트랜잭션 id
pub(crate) fn binding_success(b: &[u8]) -> Option<[u8; 12]> {
    if b.len() < HEADER || u16::from_be_bytes([b[0], b[1]]) != BINDING_SUCCESS || u32::from_be_bytes([b[4], b[5], b[6], b[7]]) != MAGIC_COOKIE {
        return None;
    }
    b[8..20].try_into().ok()
}

pub(crate) fn random_ice_string(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| (b % 36) as u32).map(|r| char::from_digit(r, 36).unwrap_or('0')).collect()
}

fn attr(b: &mut Vec<u8>, t: u16, v: &[u8]) {
    b.extend_from_slice(&t.to_be_bytes());
    b.extend_from_slice(&(v.len() as u16).to_be_bytes());
    b.extend_from_slice(v);
    b.extend(std::iter::repeat_n(0u8, (4 - v.len() % 4) % 4));
}

fn hmac_sha1(data: &[u8], key: &[u8]) -> [u8; 20] {
    use hmac::{Hmac, Mac};
    let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(key) else { return [0; 20] };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5769 의 꼴 검사 — 길이 · 지문 · 무결성 자리
    #[test]
    fn request_shape() {
        let (b, tid) = binding_request("srv", "cli", "pwd", 7);
        assert_eq!(u16::from_be_bytes([b[0], b[1]]), BINDING_REQUEST);
        assert_eq!(usize::from(u16::from_be_bytes([b[2], b[3]])), b.len() - HEADER);
        assert_eq!(&b[8..20], &tid);
        let n = b.len();
        assert_eq!(u16::from_be_bytes([b[n - 8], b[n - 7]]), ATTR_FINGERPRINT);
        let crc = crc32fast::hash(&b[..n - 8]) ^ 0x5354_554E;
        assert_eq!(&b[n - 4..], &crc.to_be_bytes());
        let mut ok = b.clone();
        ok[0..2].copy_from_slice(&BINDING_SUCCESS.to_be_bytes());
        assert_eq!(binding_success(&ok), Some(tid));
        assert_eq!(binding_success(&b), None);
    }
}
