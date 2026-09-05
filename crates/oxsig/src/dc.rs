// author: kodeholic (powered by Claude)
//! DC 프레임 — 연§3-3. `ver(1)=0x01 · svc(1) · len(2 BE) · payload`. 잘린 프레임은 조용히 버린다.

pub const VER: u8 = 0x01;
pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD_LEN: usize = 0xFFFF;
/// 발언권(MBCP) — 이 문서가 정하는 유일한 svc.
pub const SVC_MBCP: u8 = 0x01;
/// 발성 감지 — 확장(연§11-6). 무전 방은 발신 금지·수신 무시.
pub const SVC_VOICE_ACTIVITY: u8 = 0x02;
/// `0x80~` 앱 예약.
pub const SVC_APP_MIN: u8 = 0x80;
/// 연§9-7 — 채널 이름 하나. 다른 이름은 서버가 거부한다.
pub const CHANNEL_LABEL: &str = "unreliable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DcError {
    /// 연§3-3 — `len` 초과는 보내는 쪽이 거부한다.
    TooLong(usize),
}

pub fn encode(svc: u8, payload: &[u8]) -> Result<Vec<u8>, DcError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(DcError::TooLong(payload.len()));
    }
    let len = payload.len() as u16;
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(VER);
    out.push(svc);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// `None` = 버린다(짧다 · ver 다르다 · 잘렸다). 예외를 던지지 않는다.
pub fn decode(frame: &[u8]) -> Option<(u8, &[u8])> {
    if frame.len() < HEADER_LEN || frame[0] != VER {
        return None;
    }
    let len = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    let payload = frame.get(HEADER_LEN..HEADER_LEN + len)?;
    Some((frame[1], payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_drop_rules() {
        let f = encode(SVC_MBCP, &[9, 8, 7]).unwrap();
        assert_eq!(f, vec![1, 1, 0, 3, 9, 8, 7]);
        assert_eq!(decode(&f), Some((SVC_MBCP, &[9u8, 8, 7][..])));
        assert_eq!(decode(&f[..6]), None);
        assert_eq!(decode(&[2, 1, 0, 0]), None);
        assert_eq!(decode(&[1, 1, 0]), None);
        assert_eq!(decode(&[1, 0x80, 0, 0]), Some((0x80, &[][..])));
        assert_eq!(encode(1, &vec![0; MAX_PAYLOAD_LEN + 1]), Err(DcError::TooLong(MAX_PAYLOAD_LEN + 1)));
    }
}
