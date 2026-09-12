// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · 연§4-2 · §6-2 · model: claude-opus-5

//! 미디어를 붙이는 재료 — ★**ICE 자격과 DTLS 지문.**
//!
//! ★**지문은 프로세스 것이고 자격은 연결 것이다.** 인증서 하나를 기동 때 굽고(그래야
//! 재협상마다 지문이 바뀌지 않는다), ufrag/pwd 는 ★**연결마다 새로 발급해 세션 동안 불변**이다
//! (연§9-3 — 바꾸면 브라우저가 ICE 재시작으로 오인한다).

use oxsig::body::room::{CodecCap, DtlsConfig, ExtmapEntry, IceConfig};
use oxsig::types::Kind;

/// 정§12 — ufrag 8자 · pwd 22자.
const UFRAG_LEN: usize = 8;
const PWD_LEN: usize = 22;

/// RFC 5245 가 허용하는 ice-char. ★**혼동하는 글자를 빼지 않는다** — 사람이 읽는 값이 아니다.
const ICE_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn rand_ice(n: usize) -> String {
    let mut raw = vec![0u8; n];
    getrandom::fill(&mut raw).expect("OS 난수 — 없으면 자격을 지어낼 수 없다");
    raw.iter().map(|b| ICE_CHARS[*b as usize % ICE_CHARS.len()] as char).collect()
}

/// 한 Peer 의 ICE 자격 넷. ★**보내기와 받기를 가른다**(연결이 둘이다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCreds {
    pub publish_ufrag: String,
    pub publish_pwd: String,
    pub subscribe_ufrag: String,
    pub subscribe_pwd: String,
}

impl IceCreds {
    pub fn fresh() -> Self {
        Self {
            publish_ufrag: rand_ice(UFRAG_LEN),
            publish_pwd: rand_ice(PWD_LEN),
            subscribe_ufrag: rand_ice(UFRAG_LEN),
            subscribe_pwd: rand_ice(PWD_LEN),
        }
    }

    pub fn config(&self, ip: &str, port: u16) -> IceConfig {
        IceConfig {
            ip: ip.to_string(),
            port,
            publish_ufrag: self.publish_ufrag.clone(),
            publish_pwd: self.publish_pwd.clone(),
            subscribe_ufrag: self.subscribe_ufrag.clone(),
            subscribe_pwd: self.subscribe_pwd.clone(),
        }
    }
}

/// 이 프로세스의 DTLS 신원. ★**기동 때 한 번 굽는다.**
#[derive(Debug, Clone)]
pub struct Dtls {
    /// `"sha-256 AB:CD:…"` — 클라는 가공 없이 `a=fingerprint:` 뒤에 붙인다.
    pub fingerprint: String,
    /// DER 인증서와 키 — 전송이 설 때 쓴다.
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

impl Dtls {
    /// ★**자가서명 하나** — WebRTC 의 신뢰 기준은 CA 가 아니라 SDP 가 나른 지문이다(RFC 8122).
    pub fn bake() -> Result<Self, String> {
        let kp = rcgen::KeyPair::generate().map_err(|e| format!("dtls: 키 생성 {e}"))?;
        let cert = rcgen::CertificateParams::new(vec!["oxlens-sfu".to_string()])
            .map_err(|e| format!("dtls: 파라미터 {e}"))?
            .self_signed(&kp)
            .map_err(|e| format!("dtls: 자가서명 {e}"))?;
        let der = cert.der().to_vec();
        Ok(Self { fingerprint: fingerprint_of(&der), cert_der: der, key_der: kp.serialize_der() })
    }

    pub fn config(&self) -> DtlsConfig {
        DtlsConfig {
            fingerprint: self.fingerprint.clone(),
            // ★항상 `passive` — 그래서 클라가 `active` 다.
            setup: "passive".to_string(),
        }
    }
}

/// DER 인증서의 sha-256 지문 — ★**대문자 16진 + 콜론**이 SDP 표기다(RFC 8122 §5).
fn fingerprint_of(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let sum = Sha256::digest(der);
    let hex: Vec<String> = sum.iter().map(|b| format!("{b:02X}")).collect();
    format!("sha-256 {}", hex.join(":"))
}

/// 서버가 선언하는 확장 번호(연§6-2 표). ★**여섯 고정**이다.
pub fn extmap() -> Vec<ExtmapEntry> {
    [
        (1u8, "urn:ietf:params:rtp-hdrext:sdes:mid"),
        (4, "urn:ietf:params:rtp-hdrext:ssrc-audio-level"),
        (5, "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time"),
        (6, "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"),
        (10, "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id"),
        (11, "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id"),
    ]
    .into_iter()
    .map(|(id, uri)| ExtmapEntry { id, uri: uri.to_string() })
    .collect()
}

/// 서버가 받고 싶은 코덱. ★**PT·클럭·fmtp 는 없다** — 그것은 협상이 정한다(연§6-2).
pub fn codecs() -> Vec<CodecCap> {
    vec![
        CodecCap {
            kind: Kind::Audio,
            name: "opus".into(),
            rtcp_fb: vec!["transport-cc".into()],
        },
        CodecCap {
            kind: Kind::Video,
            name: "VP8".into(),
            rtcp_fb: vec!["nack".into(), "nack pli".into(), "ccm fir".into(), "transport-cc".into()],
        },
        CodecCap {
            kind: Kind::Video,
            name: "H264".into(),
            rtcp_fb: vec!["nack".into(), "nack pli".into(), "ccm fir".into(), "transport-cc".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 자격은_길이가_계약이다() {
        let c = IceCreds::fresh();
        assert_eq!(c.publish_ufrag.chars().count(), UFRAG_LEN);
        assert_eq!(c.subscribe_pwd.chars().count(), PWD_LEN);
        // ★보내기와 받기는 다른 값이다 — 같으면 두 연결을 가를 수 없다.
        assert_ne!(c.publish_ufrag, c.subscribe_ufrag);
    }

    #[test]
    fn 지문은_프로세스_것이라_두_번_불러도_같다() {
        let d = Dtls::bake().expect("굽는다");
        assert!(d.fingerprint.starts_with("sha-256 "), "{}", d.fingerprint);
        // "sha-256 " + 32바이트 × "AB:" − 콜론 하나
        assert_eq!(d.fingerprint.len(), 8 + 32 * 3 - 1, "{}", d.fingerprint);
        assert_eq!(d.config().fingerprint, d.fingerprint);
        assert_eq!(d.config().setup, "passive");
    }

    #[test]
    fn 확장_번호는_여섯_고정이다() {
        assert_eq!(extmap().iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 4, 5, 6, 10, 11]);
    }
}
