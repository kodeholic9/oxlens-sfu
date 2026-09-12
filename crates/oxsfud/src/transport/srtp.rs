// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · §7-2-1 · model: claude-opus-5

//! SRTP — ★**프로파일 하나**(`AES128-CM-HMAC-SHA1-80`, 정§12).
//!
//! ★**방향마다 열쇠가 다르다**(RFC 5764 §4.2): 클라가 쓰는 것으로 **풀고**, 서버가 쓰는
//! 것으로 **잠근다**. 한 벌로 양쪽을 하면 첫 패킷에서 인증이 깨진다.

use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;

use super::dtls::SrtpKeys;

/// 한 연결의 SRTP 두 벌.
pub struct SrtpPair {
    /// 받는 것을 푼다(클라 → 서버).
    inbound: Context,
    /// 보내는 것을 잠근다(서버 → 클라).
    outbound: Context,
}

impl std::fmt::Debug for SrtpPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ★열쇠를 찍지 않는다 — 로그에 새면 그 세션의 미디어가 통째로 열린다.
        f.write_str("SrtpPair{…}")
    }
}

impl SrtpPair {
    pub fn new(k: &SrtpKeys) -> Result<Self, String> {
        let mk = |key: &[u8], salt: &[u8]| {
            Context::new(key, salt, ProtectionProfile::Aes128CmHmacSha1_80, None, None)
                .map_err(|e| format!("srtp: {e}"))
        };
        Ok(Self {
            inbound: mk(&k.client_key, &k.client_salt)?,
            outbound: mk(&k.server_key, &k.server_salt)?,
        })
    }

    /// ★**인증이 안 맞으면 버린다** — 위조 RTP 가 fan-out 을 타면 남의 화면에 남의 것이 뜬다.
    pub fn open(&mut self, packet: &[u8]) -> Option<bytes::Bytes> {
        self.inbound.decrypt_rtp(packet).ok()
    }

    pub fn seal(&mut self, packet: &[u8]) -> Option<bytes::Bytes> {
        self.outbound.encrypt_rtp(packet).ok()
    }

    pub fn open_rtcp(&mut self, packet: &[u8]) -> Option<bytes::Bytes> {
        self.inbound.decrypt_rtcp(packet).ok()
    }

    pub fn seal_rtcp(&mut self, packet: &[u8]) -> Option<bytes::Bytes> {
        self.outbound.encrypt_rtcp(packet).ok()
    }
}

/// RTP 머리에서 `ssrc` 를 읽는다 — ★**푼 뒤의 평문에서만** 뜻이 있다.
pub fn ssrc_of(rtp: &[u8]) -> Option<u32> {
    if rtp.len() < 12 || rtp[0] >> 6 != 2 {
        return None;
    }
    Some(u32::from_be_bytes([rtp[8], rtp[9], rtp[10], rtp[11]]))
}

/// ★★**PT 를 제자리에서 바꾼다 — 길이 불변**(핫패스 규율 H1).
///
/// 두 번째 바이트의 아래 7비트가 PT 이고 최상위 비트는 marker 다. ★**marker 를 지우면**
/// 수신 지터버퍼가 프레임 경계를 잃는다 — 그래서 통째로 덮지 않고 비트로 갈아 끼운다.
pub fn rewrite_pt(rtp: &mut [u8], pt: u8) -> bool {
    if rtp.len() < 12 {
        return false;
    }
    rtp[1] = (rtp[1] & 0x80) | (pt & 0x7F);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp(marker: bool, pt: u8, ssrc: u32) -> Vec<u8> {
        let mut p = vec![0x80, (u8::from(marker) << 7) | pt, 0, 1, 0, 0, 0, 0];
        p.extend_from_slice(&ssrc.to_be_bytes());
        p.extend_from_slice(b"payload");
        p
    }

    #[test]
    fn ssrc_는_머리에서_읽는다() {
        assert_eq!(ssrc_of(&rtp(false, 111, 0x1234_5678)), Some(0x1234_5678));
        // ★판이 2 가 아니면 RTP 가 아니다.
        assert_eq!(ssrc_of(&[0x00; 12]), None);
        assert_eq!(ssrc_of(&[0x80; 8]), None, "★머리도 못 채운 것");
    }

    #[test]
    fn pt_를_바꿔도_marker_와_길이가_그대로다() {
        let mut p = rtp(true, 111, 1);
        let before = p.len();
        assert!(rewrite_pt(&mut p, 96));
        assert_eq!(p[1] & 0x7F, 96);
        assert_eq!(p[1] & 0x80, 0x80, "★marker 가 살아 있다 — 지우면 프레임 경계를 잃는다");
        assert_eq!(p.len(), before, "★길이 불변");
        assert_eq!(&p[12..], b"payload", "★본문 무접촉");
    }

    #[test]
    fn marker_가_없던_것은_없는_채로_간다() {
        let mut p = rtp(false, 111, 1);
        assert!(rewrite_pt(&mut p, 127));
        assert_eq!(p[1], 127);
    }
}
