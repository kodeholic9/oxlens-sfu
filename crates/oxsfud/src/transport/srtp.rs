// author: kodeholic (powered by Claude)
//! SRTP 문맥 — 정§12 SRTP 행. 방향마다 하나(inbound 복호·outbound 암호), 키는 DTLS 완료 뒤 한 번 설치한다.

use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;

pub struct SrtpContext(Option<Context>);

impl SrtpContext {
    pub fn install(key: &[u8], salt: &[u8]) -> Result<Self, String> {
        Context::new(key, salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None)
            .map(|c| Self(Some(c)))
            .map_err(|e| format!("srtp context: {e}"))
    }

    pub fn decrypt_rtp(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.0.as_mut().ok_or_else(|| "no key".to_owned())?.decrypt_rtp(packet).map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    pub fn decrypt_rtcp(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.0.as_mut().ok_or_else(|| "no key".to_owned())?.decrypt_rtcp(packet).map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    pub fn encrypt_rtp(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.0.as_mut().ok_or_else(|| "no key".to_owned())?.encrypt_rtp(packet).map(|b| b.to_vec()).map_err(|e| e.to_string())
    }

    pub fn encrypt_rtcp(&mut self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.0.as_mut().ok_or_else(|| "no key".to_owned())?.encrypt_rtcp(packet).map(|b| b.to_vec()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_same_material() {
        let (key, salt) = ([1u8; 16], [2u8; 14]);
        let plain = vec![0x80, 0x78, 0x00, 0x01, 0, 0, 0, 0, 0, 1, 0xE2, 0x40, 0xDE, 0xAD];
        let mut out = SrtpContext::install(&key, &salt).unwrap();
        let mut inn = SrtpContext::install(&key, &salt).unwrap();
        let sealed = out.encrypt_rtp(&plain).unwrap();
        assert_ne!(sealed, plain);
        assert_eq!(inn.decrypt_rtp(&sealed).unwrap(), plain);
        assert!(SrtpContext(None).decrypt_rtp(&plain).is_err());
    }
}
