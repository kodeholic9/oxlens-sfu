// author: kodeholic (powered by Claude)
//! 전송 수립 재료 — 정§12. 이 판은 자격과 지문만(ICE-lite host 후보 하나 · DTLS passive 지문). 소켓·SRTP·SCTP 는 뒤 판.

use dtls::crypto::Certificate;
use sha2::{Digest, Sha256};

/// 정§12 — ufrag 8자·pwd 22자, 연결마다 발급, 세션 동안 불변(연§9-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCredentials {
    pub ufrag: String,
    pub pwd: String,
}

impl IceCredentials {
    pub fn generate() -> Self {
        Self { ufrag: random_ice(8), pwd: random_ice(22) }
    }
}

fn random_ice(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("getrandom");
    bytes.iter().map(|b| { let r = b % 36; if r < 10 { char::from(b'0' + r) } else { char::from(b'a' + r - 10) } }).collect()
}

/// 프로세스 인증서 하나 — 지문은 연§4-2 형식 `"sha-256 AB:CD:…"`(해시명 소문자·대문자 hex 콜론).
pub struct ServerCert {
    pub cert: Certificate,
    pub fingerprint: String,
}

impl ServerCert {
    pub fn generate() -> Result<Self, dtls::Error> {
        let cert = Certificate::generate_self_signed(vec!["ox-sfu".to_owned()])?;
        let der = cert.certificate.first().map(|c| c.as_ref()).unwrap_or(&[]);
        Ok(Self { fingerprint: sha256_fingerprint(der), cert })
    }
}

pub fn sha256_fingerprint(der: &[u8]) -> String {
    let hex: Vec<String> = Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creds_and_fingerprint_shape() {
        let c = IceCredentials::generate();
        assert_eq!((c.ufrag.len(), c.pwd.len()), (8, 22));
        assert!(c.ufrag.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
        let f = sha256_fingerprint(b"x");
        assert!(f.starts_with("sha-256 ") && f.len() == 8 + 32 * 3 - 1);
        assert!(f[8..].bytes().all(|b| b == b':' || b.is_ascii_uppercase() || b.is_ascii_digit()));
    }
}
