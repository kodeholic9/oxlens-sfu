// author: kodeholic (powered by Claude)
//! mid 풀 — 정§7-2. 구독 연결마다 하나. 신규는 지금까지 준 최대값보다 크고(다음 값 카운터),
//! 해제분은 ★같은 kind 풀로만 재활용한다 — 브라우저(libwebrtc)가 재활용 아닌 m-section 의 media type 변경을 거부한다.

use std::collections::BTreeSet;

use oxsig::schema::MediaKind;

/// 형은 십진 정수 문자열, 상한은 u8 이다.
pub const MID_MAX: u16 = 255;
/// 정§7-2 — 받기 mid 의 시작. ★`pc_mode` 와 무관하다(발급기를 한 갈래로 둔다).
/// 1pc 은 클라 발행 mid 0..31 과 BUNDLE 을 공유하므로 이 오프셋이 있어야 겹치지 않고,
/// 2pc 도 같은 규칙을 쓴다.
pub const SUB_BASE: u16 = 32;

#[derive(Debug)]
pub struct MidPool {
    next: u16,
    free_audio: BTreeSet<u16>,
    free_video: BTreeSet<u16>,
}

impl Default for MidPool {
    fn default() -> Self {
        Self::new()
    }
}

impl MidPool {
    pub fn new() -> Self {
        Self { next: SUB_BASE, free_audio: BTreeSet::new(), free_video: BTreeSet::new() }
    }


    fn free_of(&mut self, kind: MediaKind) -> &mut BTreeSet<u16> {
        match kind {
            MediaKind::Audio => &mut self.free_audio,
            MediaKind::Video => &mut self.free_video,
        }
    }

    /// `None` = 고갈. 조용히 등록만 안 하는 것은 §16-2 위반이라 부르는 쪽이 반드시 표면화한다.
    pub fn alloc(&mut self, kind: MediaKind) -> Option<u16> {
        if let Some(m) = self.free_of(kind).pop_first() {
            return Some(m);
        }
        if self.next > MID_MAX {
            return None;
        }
        let m = self.next;
        self.next += 1;
        Some(m)
    }

    pub fn release(&mut self, kind: MediaKind, mid: u16) {
        self.free_of(kind).insert(mid);
    }

    pub fn exhausted(&self) -> bool {
        self.next > MID_MAX && self.free_audio.is_empty() && self.free_video.is_empty()
    }
}

/// wire 는 십진 정수 문자열이다(연§4-1).
pub fn to_wire(mid: u16) -> String {
    mid.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_mode_independent_and_reuse_is_kind_bound() {
        let mut p = MidPool::new();
        assert_eq!(
            (p.alloc(MediaKind::Audio), p.alloc(MediaKind::Video)),
            (Some(SUB_BASE), Some(SUB_BASE + 1)),
            "받기 mid 는 모드와 무관하게 32 부터다"
        );

        p.release(MediaKind::Audio, SUB_BASE);
        assert_eq!(p.alloc(MediaKind::Video), Some(SUB_BASE + 2), "audio 해제분은 video 가 못 쓴다");
        assert_eq!(p.alloc(MediaKind::Audio), Some(SUB_BASE), "같은 kind 면 재활용");
        assert_eq!(p.alloc(MediaKind::Audio), Some(SUB_BASE + 3), "신규는 준 최대값보다 크다");
        assert_eq!(to_wire(SUB_BASE + 3), "35");
    }

    #[test]
    fn exhaustion_is_a_reported_state() {
        let mut p = MidPool::new();
        for want in SUB_BASE..=MID_MAX {
            assert_eq!(p.alloc(MediaKind::Video), Some(want));
        }
        assert!(p.exhausted(), "32~255 를 다 준 뒤엔 카운터도 해제분도 없다");
        assert_eq!(p.alloc(MediaKind::Video), None);
        p.release(MediaKind::Video, SUB_BASE + 7);
        assert!(!p.exhausted(), "고갈은 영구가 아니다");
        assert_eq!(p.alloc(MediaKind::Video), Some(SUB_BASE + 7));
    }
}
