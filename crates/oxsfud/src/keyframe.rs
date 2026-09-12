// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§10-2 · §6-2 · model: claude-opus-5

//! 키프레임 판정 — ★★**단 전환이 서는 자리가 여기다**(정§10-2).
//!
//! 단을 갈아탈 때 아무 패킷에서나 갈아타면 ★**새 단의 첫 프레임이 앞 프레임을 참조**해
//! 디코더가 깨진다. 그래서 전환은 ★**target 단의 키프레임이 도착한 순간**에만 선다.
//!
//! ★★**코덱을 모른 채 판정기 둘을 다 돌리지 않는다** — 먼저 매칭되는 쪽을 취하면
//! 선언과 실 payload 가 어긋났을 때 ★**엉뚱한 판정기가 붙는다**(VP8 판정기가 H264 를
//! 키프레임이라 답하는 식). 코덱은 등록이 이미 알고 있으니 그것으로 고른다.
//!
//! ★**키프레임 판정기가 없는 코덱은 지원 코덱이 아니다**(정§6-2) — 전환도 게이트도
//! 설 자리가 없기 때문이다.

/// 등록에서 고르는 판정기.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Vp8,
    H264,
}

impl Codec {
    /// 선언 문자열 → 코덱. ★**표에 없으면 `None`** 이다(입구에서 거절할 근거다).
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "VP8" => Some(Codec::Vp8),
            "H264" => Some(Codec::H264),
            _ => None,
        }
    }

    pub fn is_keyframe(&self, rtp: &[u8]) -> bool {
        match self {
            Codec::Vp8 => is_vp8_keyframe(rtp),
            Codec::H264 => is_h264_keyframe(rtp),
        }
    }
}

/// 머리를 건너뛴 본문의 시작.
fn payload_at(rtp: &[u8]) -> Option<usize> {
    if rtp.len() < 12 || rtp[0] >> 6 != 2 {
        return None;
    }
    let csrc = (rtp[0] & 0x0F) as usize * 4;
    let mut at = 12 + csrc;
    if rtp[0] & 0x10 != 0 {
        if rtp.len() < at + 4 {
            return None;
        }
        let words = u16::from_be_bytes([rtp[at + 2], rtp[at + 3]]) as usize;
        at += 4 + words * 4;
    }
    (at < rtp.len()).then_some(at)
}

/// VP8 — ★**파티션 시작 조각만** 답할 수 있다(RFC 7741).
///
/// 이어지는 조각에는 프레임 머리가 없으므로 ★**모른다고 답한다**(지어내지 않는다).
pub fn is_vp8_keyframe(rtp: &[u8]) -> bool {
    let Some(at) = payload_at(rtp) else { return false };
    let desc = rtp[at];
    // S 비트 — 파티션의 첫 조각인가.
    if desc & 0x10 == 0 {
        return false;
    }
    let mut p = at + 1;
    // X 비트 — 확장 서술자가 붙어 있으면 그만큼 건너뛴다.
    if desc & 0x80 != 0 {
        let Some(&x) = rtp.get(p) else { return false };
        p += 1;
        if x & 0x80 != 0 {
            // I — PictureID. 첫 바이트의 최상위가 서면 두 바이트다.
            let Some(&pid) = rtp.get(p) else { return false };
            p += if pid & 0x80 != 0 { 2 } else { 1 };
        }
        if x & 0x40 != 0 {
            p += 1; // L — TL0PICIDX
        }
        if x & 0x20 != 0 || x & 0x10 != 0 {
            p += 1; // T·K — TID/KEYIDX
        }
    }
    // ★프레임 머리의 최하위 비트가 0 이면 키프레임이다.
    matches!(rtp.get(p), Some(&b) if b & 0x01 == 0)
}

/// H264 — ★**SPS(7) 도 키프레임으로 본다**(RFC 6184).
///
/// ★IDR(5)만 보면 ★**해상도 정보 없이 IDR 만 받은 디코더가 깨진다** — SPS 가 전환의
/// 경계라는 것이 선례(mediasoup #283 · Janus)의 결론이다.
pub fn is_h264_keyframe(rtp: &[u8]) -> bool {
    let Some(at) = payload_at(rtp) else { return false };
    let nal = rtp[at] & 0x1F;
    match nal {
        5 | 7 => true,
        // STAP-A — 안에 든 것 중 하나라도 SPS·IDR 이면 그렇다.
        24 => {
            let mut p = at + 1;
            while p + 2 < rtp.len() {
                let size = u16::from_be_bytes([rtp[p], rtp[p + 1]]) as usize;
                p += 2;
                if size == 0 || p >= rtp.len() {
                    break;
                }
                let inner = rtp[p] & 0x1F;
                if inner == 5 || inner == 7 {
                    return true;
                }
                p += size;
            }
            false
        }
        // FU-A — ★**시작 조각만** 답할 수 있다.
        28 => {
            let Some(&hdr) = rtp.get(at + 1) else { return false };
            hdr & 0x80 != 0 && matches!(hdr & 0x1F, 5 | 7)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtp(payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn vp8_는_프레임_머리로_가른다() {
        // 서술자 0x10(S=1, X=0) + 프레임 머리.
        assert!(is_vp8_keyframe(&rtp(&[0x10, 0x10, 0, 0])));
        assert!(!is_vp8_keyframe(&rtp(&[0x10, 0x11, 0, 0])));
    }

    #[test]
    fn vp8_이어지는_조각은_모른다고_답한다() {
        // ★S=0 — 프레임 머리가 없는 자리다. 지어내면 엉뚱한 곳에서 전환이 선다.
        assert!(!is_vp8_keyframe(&rtp(&[0x00, 0x10, 0, 0])));
    }

    #[test]
    fn vp8_확장_서술자를_건너뛴다() {
        // X=1(0x90), 확장 바이트 I|L|T = 0xE0, PictureID 2바이트(0x80,0x01), TL0(0x00), TID(0x00).
        let p = [0x90, 0xE0, 0x80, 0x01, 0x00, 0x00, 0x10, 0, 0];
        assert!(is_vp8_keyframe(&rtp(&p)), "★건너뛰기가 어긋나면 남의 바이트를 프레임 머리로 읽는다");
        let mut q = p;
        q[6] = 0x11;
        assert!(!is_vp8_keyframe(&rtp(&q)));
    }

    #[test]
    fn vp8_확장이_잘려_있으면_안_답한다() {
        assert!(!is_vp8_keyframe(&rtp(&[0x90])));
        assert!(!is_vp8_keyframe(&rtp(&[0x90, 0xE0, 0x80])));
    }

    #[test]
    fn h264_는_sps_도_키프레임이다() {
        assert!(is_h264_keyframe(&rtp(&[0x65, 0, 0])), "IDR");
        assert!(is_h264_keyframe(&rtp(&[0x67, 0, 0])), "★SPS 도 경계다");
        assert!(!is_h264_keyframe(&rtp(&[0x41, 0, 0])));
    }

    #[test]
    fn h264_묶음과_조각도_읽는다() {
        // STAP-A: [24][len=2][0x67 ..]
        assert!(is_h264_keyframe(&rtp(&[24, 0, 2, 0x67, 0x00])));
        assert!(!is_h264_keyframe(&rtp(&[24, 0, 2, 0x41, 0x00])));
        // FU-A 시작 조각 — S=1, type=5.
        assert!(is_h264_keyframe(&rtp(&[28, 0x85, 0, 0])));
        // ★이어지는 조각은 경계가 아니다.
        assert!(!is_h264_keyframe(&rtp(&[28, 0x05, 0, 0])));
    }

    #[test]
    fn 코덱은_등록이_고른다() {
        // ★둘 다 돌려 먼저 맞는 쪽을 취하면 여기서 갈린다 — H264 non-IDR(0x41)은
        //   VP8 눈으로 보면 S=0 이라 거짓, 반대로 VP8 키프레임(0x10,0x10)은 H264 눈으로
        //   nal=16 이라 거짓이다. 섞어 쓰면 어느 쪽이 답했는지 모른다.
        assert_eq!(Codec::from_name("vp8"), Some(Codec::Vp8));
        assert_eq!(Codec::from_name("H264"), Some(Codec::H264));
        assert_eq!(Codec::from_name("AV1"), None, "★판정기가 없으면 지원이 아니다");
        assert!(Codec::Vp8.is_keyframe(&rtp(&[0x10, 0x10, 0, 0])));
        assert!(!Codec::H264.is_keyframe(&rtp(&[0x10, 0x10, 0, 0])));
    }

    #[test]
    fn 본문_앞의_확장을_건너뛴다() {
        let mut p = vec![0x90, 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7];
        p.extend_from_slice(&0xBEDEu16.to_be_bytes());
        p.extend_from_slice(&1u16.to_be_bytes());
        p.extend_from_slice(&[(10 << 4), b'h', 0, 0]);
        p.extend_from_slice(&[0x10, 0x10, 0, 0]);
        assert!(is_vp8_keyframe(&p));
    }
}
