// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§6 · 연§3-2 · model: claude-opus-5

//! op 카탈로그 — ★**전량 18**(연§6). 이 목록 밖의 번호는 `1001` 이다.
//!
//! 접미사가 규율이다 — `_EVENT` 는 **존재·관계**가 바뀌었다(구조를 고친다) ·
//! `_STATE` 는 **속성**이 바뀌었다(표시만 고친다). `LEAVE` 는 세션 축이라 그 규율 밖의 이름이다.

/// 흐름 제어 우선순위 단(연§3-2). ★**방 상태를 나르는 것은 전부 1단**이다 — 순서가 곧 상태다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// 0 복구 — ★**비어 있다(예약).** `RESUME` 은 `BIND` 응답 뒤 직렬이라 앞지를 것이 없다.
    Recovery,
    /// 1 세션·방·미디어.
    Session,
    /// 2 데이터.
    Data,
    /// 3 진단.
    Diagnostic,
}

macro_rules! ops {
    ($( $name:ident = $num:literal, $lane:ident ; )*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Op { $( $name ),* }

        impl Op {
            /// 카탈로그 전량.
            pub const ALL: &'static [Op] = &[ $( Op::$name ),* ];

            pub fn from_u16(v: u16) -> Option<Op> {
                match v { $( $num => Some(Op::$name), )* _ => None }
            }

            pub fn as_u16(self) -> u16 {
                match self { $( Op::$name => $num, )* }
            }

            /// 이 op 이 타는 단. 늘리거나 바꾸면 연§3-2 의 성립 조건이 먼저 깨진다.
            pub fn lane(self) -> Lane {
                match self { $( Op::$name => Lane::$lane, )* }
            }
        }
    };
}

ops! {
    Bind             = 0x0101, Session;
    Resume           = 0x0102, Session;
    Heartbeat        = 0x0103, Session;
    Leave            = 0x0104, Session;
    RoomJoin         = 0x0201, Session;
    RoomLeave        = 0x0202, Session;
    PublishTracks    = 0x0301, Session;
    Ready            = 0x0302, Session;
    SubscribeLayer   = 0x0303, Session;
    TrackSet         = 0x0304, Session;
    Affiliation      = 0x0401, Session;
    Message          = 0x0501, Data;
    Task             = 0x0601, Diagnostic;
    ParticipantEvent = 0x0701, Session;
    TrackEvent       = 0x0702, Session;
    TrackState       = 0x0703, Session;
    RoomEvent        = 0x0704, Session;
    ParticipantState = 0x0705, Session;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 전량_18() {
        assert_eq!(Op::ALL.len(), 18, "★연§6 은 전량 18 이다");
    }

    #[test]
    fn 번호_왕복() {
        for &op in Op::ALL {
            assert_eq!(Op::from_u16(op.as_u16()), Some(op));
        }
    }

    #[test]
    fn 번호가_겹치지_않는다() {
        let mut seen = std::collections::BTreeSet::new();
        for &op in Op::ALL {
            assert!(seen.insert(op.as_u16()), "★번호 중복: {:#06x}", op.as_u16());
        }
    }

    #[test]
    fn 방_상태를_나르는_것은_전부_한_단이다() {
        // ★하나만 위로 올리면 뒤에 온 것이 먼저 도착해 상태가 되감긴다(연§3-2).
        for op in [
            Op::RoomEvent,
            Op::ParticipantEvent,
            Op::TrackEvent,
            Op::TrackState,
            Op::ParticipantState,
            Op::Affiliation,
        ] {
            assert_eq!(op.lane(), Lane::Session, "{op:?}");
        }
    }

    #[test]
    fn 복구_단은_비어_있다() {
        assert!(
            Op::ALL.iter().all(|o| o.lane() != Lane::Recovery),
            "★0단은 예약이다 — 채우려면 연§3-2 를 먼저 고친다"
        );
    }
}
