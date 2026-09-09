// author: kodeholic (powered by Claude)
//! mid 풀 — 정§7-2. 구독 연결마다 하나. 신규는 지금까지 준 최대값보다 크고(다음 값 카운터),
//! 해제분은 ★같은 kind 풀로만 재활용한다 — 브라우저(libwebrtc)가 재활용 아닌 m-section 의 media type 변경을 거부한다.

use std::collections::BTreeSet;

use oxsig::schema::{MediaKind, PcMode};

/// 형은 십진 정수 문자열, 상한은 u8 이다.
pub const MID_MAX: u16 = 255;
/// 정§7-2 — `1pc` 받기 mid 의 시작. 클라 발행 mid 와 ★한 BUNDLE 을 쓰므로 이 오프셋이 있어야
/// 겹치지 않는다. 한도는 동시 발행 상한 + 데이터 채널 위의 여유다(연§4-1).
pub const SUB_BASE_1PC: u16 = 32;
/// 정§7-2 — `2pc` 받기 mid 의 시작. 받기 연결에는 ★클라 m-line 이 없어(연§9-6 — DC 도 없다)
/// 비켜 줄 상대가 없다. 오프셋을 두면 `mid` 공간만 32개 잃는다.
pub const SUB_BASE_2PC: u16 = 0;

/// 그 모드의 시작값.
pub const fn sub_base(mode: PcMode) -> u16 {
    match mode {
        PcMode::OnePc => SUB_BASE_1PC,
        PcMode::TwoPc => SUB_BASE_2PC,
    }
}

#[derive(Debug)]
pub struct MidPool {
    next: u16,
    free_audio: BTreeSet<u16>,
    free_video: BTreeSet<u16>,
}

impl MidPool {
    pub fn new(mode: PcMode) -> Self {
        Self { next: sub_base(mode), free_audio: BTreeSet::new(), free_video: BTreeSet::new() }
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
    fn base_follows_the_mode_and_reuse_is_kind_bound() {
        let mut p = MidPool::new(PcMode::OnePc);
        assert_eq!(
            (p.alloc(MediaKind::Audio), p.alloc(MediaKind::Video)),
            (Some(SUB_BASE_1PC), Some(SUB_BASE_1PC + 1)),
            "1pc 는 클라 발행 mid 와 한 BUNDLE 이라 32 부터다"
        );

        p.release(MediaKind::Audio, SUB_BASE_1PC);
        assert_eq!(p.alloc(MediaKind::Video), Some(SUB_BASE_1PC + 2), "audio 해제분은 video 가 못 쓴다");
        assert_eq!(p.alloc(MediaKind::Audio), Some(SUB_BASE_1PC), "같은 kind 면 재활용");
        assert_eq!(p.alloc(MediaKind::Audio), Some(SUB_BASE_1PC + 3), "신규는 준 최대값보다 크다");
        assert_eq!(to_wire(SUB_BASE_1PC + 3), "35");
    }

    /// 정§7-2 — 받기 연결에는 클라 m-line 이 없다(연§9-6). 비켜 줄 상대가 없으므로 0 부터다.
    #[test]
    fn two_pc_starts_at_zero_because_nothing_shares_that_space() {
        let mut p = MidPool::new(PcMode::TwoPc);
        assert_eq!(
            (p.alloc(MediaKind::Audio), p.alloc(MediaKind::Video)),
            (Some(0), Some(1)),
            "2pc 받기 mid 는 0 부터다"
        );
        assert_eq!(to_wire(0), "0");
    }

    #[test]
    fn exhaustion_is_a_reported_state() {
        let mut p = MidPool::new(PcMode::OnePc);
        for want in SUB_BASE_1PC..=MID_MAX {
            assert_eq!(p.alloc(MediaKind::Video), Some(want));
        }
        assert!(p.exhausted(), "32~255 를 다 준 뒤엔 카운터도 해제분도 없다");
        assert_eq!(p.alloc(MediaKind::Video), None);
        p.release(MediaKind::Video, SUB_BASE_1PC + 7);
        assert!(!p.exhausted(), "고갈은 영구가 아니다");
        assert_eq!(p.alloc(MediaKind::Video), Some(SUB_BASE_1PC + 7));
    }
}
