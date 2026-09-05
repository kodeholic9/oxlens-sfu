// author: kodeholic (powered by Claude)
//! 첫 바이트 분류 — 정§12 demux 행(RFC 5764 §5.1.2). 포트 하나로 STUN·DTLS·SRTP 가 섞여 들어온다.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packet {
    Stun,
    Dtls,
    Srtp,
    Unknown,
}

pub fn classify(buf: &[u8]) -> Packet {
    match buf.first() {
        Some(0x00..=0x03) => Packet::Stun,
        Some(0x14..=0x3F) => Packet::Dtls,
        Some(0x80..=0xBF) => Packet::Srtp,
        _ => Packet::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_byte_ranges() {
        assert_eq!(classify(&[0x00, 0x01]), Packet::Stun);
        assert_eq!(classify(&[0x16, 0xFE]), Packet::Dtls);
        assert_eq!(classify(&[0x17]), Packet::Dtls);
        assert_eq!(classify(&[0x80, 0x60]), Packet::Srtp);
        assert_eq!(classify(&[0xC8]), Packet::Unknown);
        assert_eq!(classify(&[]), Packet::Unknown);
    }
}
