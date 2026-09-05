// author: kodeholic (powered by Claude)
//! mid 풀 — 정§7-2. 구독 연결마다 하나. 신규는 지금까지 준 최대값보다 크고(다음 값 카운터),
//! 해제분은 ★같은 kind 풀로만 재활용한다 — 브라우저(libwebrtc)가 재활용 아닌 m-section 의 media type 변경을 거부한다.

use std::collections::BTreeSet;

use oxsig::schema::{MediaKind, PcMode};

/// 형은 십진 정수 문자열, 상한은 u8 이다.
pub const MID_MAX: u16 = 255;
/// 정§7-2 — 1pc 는 클라 발행 mid 0..31 과 BUNDLE 을 공유하므로 오프셋을 나눠 갖는다.
pub const ONE_PC_BASE: u16 = 32;

#[derive(Debug)]
pub struct MidPool {
    next: u16,
    free_audio: BTreeSet<u16>,
    free_video: BTreeSet<u16>,
}

impl MidPool {
    pub fn new(pc_mode: PcMode) -> Self {
        let next = match pc_mode {
            PcMode::OnePc => ONE_PC_BASE,
            PcMode::TwoPc => 0,
        };
        Self { next, free_audio: BTreeSet::new(), free_video: BTreeSet::new() }
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
    fn base_differs_by_mode_and_reuse_is_kind_bound() {
        let mut two = MidPool::new(PcMode::TwoPc);
        assert_eq!((two.alloc(MediaKind::Audio), two.alloc(MediaKind::Video)), (Some(0), Some(1)));
        let mut one = MidPool::new(PcMode::OnePc);
        assert_eq!(one.alloc(MediaKind::Audio), Some(ONE_PC_BASE));

        two.release(MediaKind::Audio, 0);
        assert_eq!(two.alloc(MediaKind::Video), Some(2), "audio 해제분은 video 가 못 쓴다");
        assert_eq!(two.alloc(MediaKind::Audio), Some(0), "같은 kind 면 재활용");
        assert_eq!(two.alloc(MediaKind::Audio), Some(3), "신규는 준 최대값보다 크다");
        assert_eq!(to_wire(3), "3");
    }

    #[test]
    fn exhaustion_is_a_reported_state() {
        let mut p = MidPool::new(PcMode::TwoPc);
        for want in 0..=MID_MAX {
            assert_eq!(p.alloc(MediaKind::Video), Some(want));
        }
        assert!(p.exhausted(), "0~255 를 다 준 뒤엔 카운터도 해제분도 없다");
        assert_eq!(p.alloc(MediaKind::Video), None);
        p.release(MediaKind::Video, 7);
        assert!(!p.exhausted(), "고갈은 영구가 아니다");
        assert_eq!(p.alloc(MediaKind::Video), Some(7));
    }
}
