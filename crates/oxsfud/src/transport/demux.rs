// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§12 · model: claude-opus-5

//! 한 포트에 세 가지가 온다 — ★**첫 바이트가 가른다**(RFC 5764 §5.1.2).
//!
//! ★**포트 하나가 배치 전제다**(정§12) — 방화벽 예외가 UDP 목적지 한 개라 포트 레인지
//! SFU 대비 우위이고, 그 대가가 이 분류기 하나다.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packet {
    Stun,
    Dtls,
    /// SRTP·SRTCP — 가르는 것은 다음 층이다(PT 로 갈린다).
    Srtp,
    /// ★**조용히 버리지 않는다** — 부르는 쪽이 세고 로그를 남긴다.
    Unknown,
}

pub fn classify(buf: &[u8]) -> Packet {
    match buf.first() {
        None => Packet::Unknown,
        Some(0x00..=0x03) => Packet::Stun,
        Some(0x14..=0x3F) => Packet::Dtls,
        Some(0x80..=0xBF) => Packet::Srtp,
        Some(_) => Packet::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 세_대역이_안_겹친다() {
        assert_eq!(classify(&[0x00, 0x01]), Packet::Stun);
        // DTLS ContentType — handshake 0x16 · application_data 0x17.
        assert_eq!(classify(&[0x16, 0xFE]), Packet::Dtls);
        assert_eq!(classify(&[0x17, 0xFE]), Packet::Dtls);
        // RTP 판 2 — 첫 바이트 `10xxxxxx`.
        assert_eq!(classify(&[0x80, 0x60]), Packet::Srtp);
        assert_eq!(classify(&[0xBF]), Packet::Srtp);
    }

    #[test]
    fn 빈_것도_모르는_것도_한_갈래다() {
        assert_eq!(classify(&[]), Packet::Unknown);
        assert_eq!(classify(&[0x50]), Packet::Unknown);
        // ★대역 사이의 구멍 — 0x40~0x7F 는 아무것도 아니다.
        assert_eq!(classify(&[0x7F]), Packet::Unknown);
    }
}
