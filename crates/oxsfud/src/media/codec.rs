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

/// `1005` 의 `details.supported` — 지원 전량을 그대로 알린다.
pub fn supported_list() -> Vec<String> {
    SUPPORTED_VIDEO.iter().map(|c| (*c).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
