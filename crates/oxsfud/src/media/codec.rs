// author: kodeholic (powered by Claude)
//! 코덱 등록부 — 정§6-3 ★단일 출처. 지원 = 키프레임 판정기를 가진 것뿐이고,
//! 미지정과 미지원은 같은 취급이다(조용한 기본값 금지 — 그것이 검은 화면을 만든다).

use oxsig::schema::MediaKind;

/// 정§6-3 수치 — 1차 판 지원 video 전량. VP9·AV1 은 판정기가 없어 `1005` 로 거절하고 `details.supported` 로 알린다.
pub const SUPPORTED_VIDEO: [&str; 2] = ["VP8", "H264"];
/// 무전 audio 는 이것 하나라 맞출 것이 없다(연§6-3).
pub const AUDIO_CODEC: &str = "opus";

/// 이름은 대소문자를 가리지 않고 받되, 저장·전달은 등록부의 정규 표기로 한다.
pub fn canonical_video(name: &str) -> Option<&'static str> {
    SUPPORTED_VIDEO.iter().copied().find(|c| c.eq_ignore_ascii_case(name))
}

pub fn canonical(kind: MediaKind, name: Option<&str>) -> Option<&'static str> {
    match kind {
        MediaKind::Audio => Some(AUDIO_CODEC),
        MediaKind::Video => canonical_video(name?),
    }
}

/// 정§6-3 — ★**지원의 근거가 이것이다.** 판정기가 없는 코덱은 목록에 없다(VP9·AV1).
/// 전환은 키프레임에서만 확정되므로(정§10-2) 이 함수가 틀리면 레이어가 영영 안 바뀐다.
pub fn is_keyframe(codec: &str, payload: &[u8]) -> bool {
    match codec {
        "VP8" => vp8_keyframe(payload),
        "H264" => h264_keyframe(payload),
        _ => false,
    }
}

/// RFC 7741 §4.2 — payload descriptor 를 건너뛴 뒤 payload header 의 `P` 비트가 0 이면 키프레임.
/// ★첫 조각(`S=1`·`PID=0`)만 판정 대상이다 — 뒤 조각엔 header 가 없다.
fn vp8_keyframe(payload: &[u8]) -> bool {
    let Some(&first) = payload.first() else { return false };
    let (x, s, pid) = (first & 0x80 != 0, first & 0x10 != 0, first & 0x07);
    if !s || pid != 0 {
        return false;
    }
    let mut at = 1;
    if x {
        let Some(&ext) = payload.get(1) else { return false };
        at = 2;
        if ext & 0x80 != 0 {
            // PictureID — 1옥텟이거나(M=0) 2옥텟(M=1).
            let Some(&pic) = payload.get(at) else { return false };
            at += if pic & 0x80 != 0 { 2 } else { 1 };
        }
        if ext & 0x40 != 0 {
            at += 1;
        }
        if ext & 0x30 != 0 {
            at += 1;
        }
    }
    payload.get(at).is_some_and(|h| h & 0x01 == 0)
}

/// RFC 6184 §5.3 — NAL 종류가 IDR(5)·SPS(7)면 키프레임. STAP-A(24)는 안쪽을 본다.
fn h264_keyframe(payload: &[u8]) -> bool {
    let Some(&first) = payload.first() else { return false };
    match first & 0x1F {
        5 | 7 => true,
        24 => {
            let mut at = 1;
            while at + 2 <= payload.len() {
                let len = usize::from(u16::from_be_bytes([payload[at], payload[at + 1]]));
                let Some(&nal) = payload.get(at + 2) else { return false };
                if matches!(nal & 0x1F, 5 | 7) {
                    return true;
                }
                at += 2 + len;
            }
            false
        }
        // FU-A(28)의 시작 조각은 안쪽 NAL 종류를 둘째 바이트가 든다.
        28 => payload.get(1).is_some_and(|b| b & 0x80 != 0 && matches!(b & 0x1F, 5 | 7)),
        _ => false,
    }
}

/// `1005` 의 `details.supported` — 지원 전량을 그대로 알린다.
pub fn supported_list() -> Vec<String> {
    SUPPORTED_VIDEO.iter().map(|c| (*c).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframe_predicate_is_why_a_codec_is_supported() {
        // VP8: descriptor(S=1·PID=0) + payload header 의 P 비트.
        assert!(is_keyframe("VP8", &[0x10, 0x00, 0, 0]));
        assert!(!is_keyframe("VP8", &[0x10, 0x01, 0, 0]), "P=1 은 델타");
        assert!(!is_keyframe("VP8", &[0x11, 0x00]), "PID≠0 은 뒤 조각이라 판정 대상이 아니다");
        // X=1 + PictureID 2옥텟 + TL0PICIDX + TID → header 는 다섯째 바이트.
        assert!(is_keyframe("VP8", &[0x90, 0xC0, 0x80, 0x01, 0x0A, 0x00]));
        assert!(!is_keyframe("VP8", &[0x90, 0xC0, 0x80, 0x01, 0x0A, 0x01]));

        // H264: IDR·SPS · STAP-A 안쪽 · FU-A 시작 조각.
        assert!(is_keyframe("H264", &[0x65]) && is_keyframe("H264", &[0x67]));
        assert!(!is_keyframe("H264", &[0x41]), "non-IDR 은 델타");
        assert!(is_keyframe("H264", &[0x78, 0x00, 0x01, 0x67, 0x00, 0x01, 0x41]), "STAP-A 안에 SPS");
        assert!(is_keyframe("H264", &[0x7C, 0x85]) && !is_keyframe("H264", &[0x7C, 0x05]), "FU-A 는 시작 조각만");

        assert!(!is_keyframe("VP9", &[0x10, 0x00]), "판정기가 없으면 지원이 아니다");
        assert!(!is_keyframe("VP8", &[]));
    }

    #[test]
    fn registry_is_the_only_gate() {
        assert_eq!(canonical_video("vp8"), Some("VP8"));
        assert_eq!(canonical_video("h264"), Some("H264"));
        assert_eq!(canonical_video("VP9"), None, "판정기가 없으면 지원이 아니다");
        assert_eq!(canonical_video("AV1"), None);
        assert_eq!(canonical(MediaKind::Audio, None), Some("opus"), "audio 는 이름을 묻지 않는다");
        assert_eq!(canonical(MediaKind::Video, None), None, "미지정도 미지원과 같은 취급");
        assert_eq!(supported_list(), vec!["VP8".to_owned(), "H264".to_owned()]);
    }
}
