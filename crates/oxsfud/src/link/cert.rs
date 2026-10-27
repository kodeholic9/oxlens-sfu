// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-LINK-TRANSPORT · 정:IMPL-ROOM-SERVER-CONFIG · 연:LINK.type.ServerConfig · model: claude-opus-5-5
//! 프로세스 인증서 — 기동마다 한 장(ECDSA P-256 · CN `oxsfud`) · 지문은 `sha-256 AB:CD:…`(해시명 소문자 + 공백 + 대문자 hex 콜론).

use dtls::crypto::{Certificate, CryptoPrivateKey};
use sha2::{Digest, Sha256};

pub const CN: &str = "oxsfud";

pub fn generate() -> Result<Certificate, String> {
    let mut params = rcgen::CertificateParams::new(vec![CN.to_string()]).map_err(|e| e.to_string())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, CN);
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let private_key = CryptoPrivateKey::try_from(&key).map_err(|e| e.to_string())?;
    Ok(Certificate { certificate: vec![cert.der().clone()], private_key })
}

pub fn fingerprint(der: &[u8]) -> String {
    let d = Sha256::digest(der);
    let hex: Vec<String> = d.iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_form() {
        let c = generate().expect("cert");
        let f = fingerprint(c.certificate[0].as_ref());
        assert!(f.starts_with("sha-256 "));
        assert_eq!(f.len(), "sha-256 ".len() + 32 * 3 - 1);
        assert!(f[8..].chars().all(|c| c == ':' || c.is_ascii_digit() || ('A'..='F').contains(&c)));
    }
}
