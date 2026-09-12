// author: kodeholic (powered by Claude)
// spec: v1.1 · 연§10-1 · 연§10-2 · model: claude-opus-5

//! 실패 코드 — ★**번호 공간은 하나다.** A 평면 응답·`LEAVE` 사유·C 평면(운영)·대외(`ctl`)가 같은 표를 쓴다.
//!
//! ★**모르는 코드는 앞자리로 판단한다** — 그래야 서버가 코드를 늘려도 클라가 안 깨진다.
//! `permanent` 가 오면 그것을 우선한다.

/// 앞자리 — 모르는 번호를 만났을 때의 처방.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// 1xxx 요청이 틀렸다 — 재시도하지 않는다(앱 코드를 고친다).
    Request,
    /// 2xxx 신원·권한.
    Identity,
    /// 3xxx 지금 상태에서 안 된다.
    State,
    /// 4xxx 한계에 걸렸다 — 줄이거나 기다린다.
    Limit,
    /// 5xxx 서버 사정 — 다시 시도한다.
    Server,
}

macro_rules! codes {
    ($( $name:ident = $num:literal, $wire:literal, $perm:expr ; )*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Code { $( $name ),* }

        impl Code {
            pub const ALL: &'static [Code] = &[ $( Code::$name ),* ];

            pub fn from_u16(v: u16) -> Option<Code> {
                match v { $( $num => Some(Code::$name), )* _ => None }
            }

            pub fn as_u16(self) -> u16 {
                match self { $( Code::$name => $num, )* }
            }

            /// wire 이름 — `Failure.name` 에 그대로 싣는다.
            pub fn name(self) -> &'static str {
                match self { $( Code::$name => $wire, )* }
            }

            /// ★`None` = 이 코드는 `LEAVE` 사유로만 쓰여 응답의 `permanent` 축이 없다.
            pub fn permanent(self) -> Option<bool> {
                match self { $( Code::$name => $perm, )* }
            }
        }
    };
}

codes! {
    // 1xxx — 요청이 틀렸다(전부 permanent)
    UnknownOp          = 1001, "UNKNOWN_OP",          Some(true);
    InvalidPayload     = 1002, "INVALID_PAYLOAD",     Some(true);
    MissingField       = 1003, "MISSING_FIELD",       Some(true);
    VersionMismatch    = 1004, "VERSION_MISMATCH",    Some(true);
    CodecRequired      = 1005, "CODEC_REQUIRED",      Some(true);
    CodecMismatch      = 1006, "CODEC_MISMATCH",      Some(true);
    FieldConflict      = 1007, "FIELD_CONFLICT",      Some(true);
    ProtocolError      = 1008, "PROTOCOL_ERROR",      None;
    // 2xxx — 신원·권한
    NotBound           = 2001, "NOT_BOUND",           Some(false);
    TokenInvalid       = 2002, "TOKEN_INVALID",       Some(true);
    TokenExpired       = 2003, "TOKEN_EXPIRED",       Some(false);
    InvalidApiKey      = 2004, "INVALID_API_KEY",     Some(true);
    ClaimNotAllowed    = 2005, "CLAIM_NOT_ALLOWED",   Some(true);
    NotAuthorized      = 2006, "NOT_AUTHORIZED",      Some(true);
    SessionNotFound    = 2008, "SESSION_NOT_FOUND",   Some(false);
    SessionRevoked     = 2009, "SESSION_REVOKED",     None;
    DuplicateSession   = 2010, "DUPLICATE_SESSION",   None;
    // 3xxx — 지금 상태에서 안 된다
    RoomNotFound       = 3001, "ROOM_NOT_FOUND",      Some(false);
    NotInRoom          = 3002, "NOT_IN_ROOM",         Some(false);
    RoomNotEmpty       = 3004, "ROOM_NOT_EMPTY",      Some(false);
    TrackNotFound      = 3005, "TRACK_NOT_FOUND",     Some(false);
    TrackOpUnsupported = 3006, "TRACK_OP_UNSUPPORTED",Some(true);
    TrackBoundToRoom   = 3007, "TRACK_BOUND_TO_ROOM", Some(false);
    SsrcCollision      = 3008, "SSRC_COLLISION",      Some(true);
    PreconditionFailed = 3009, "PRECONDITION_FAILED", Some(false);
    // 4xxx — 한계(전부 permanent 아님)
    RoomFull           = 4001, "ROOM_FULL",           Some(false);
    TrackLimit         = 4002, "TRACK_LIMIT",         Some(false);
    QuotaExceeded      = 4003, "QUOTA_EXCEEDED",      Some(false);
    ListenLimit        = 4004, "LISTEN_LIMIT",        Some(false);
    MidLimit           = 4005, "MID_LIMIT",           Some(false);
    FlowTimeout        = 4006, "FLOW_TIMEOUT",        None;
    FlowOverflow       = 4007, "FLOW_OVERFLOW",       None;
    HeartbeatTimeout   = 4008, "HEARTBEAT_TIMEOUT",   None;
    // 5xxx — 서버 사정(전부 permanent 아님)
    SfuUnavailable     = 5001, "SFU_UNAVAILABLE",     Some(false);
    SfuError           = 5002, "SFU_ERROR",           Some(false);
    InternalError      = 5003, "INTERNAL_ERROR",      Some(false);
    ServerShutdown     = 5004, "SERVER_SHUTDOWN",     None;
}

/// ★**폐기 번호 — 비워 둔다.** *"내지 않는다"* 도 계약이라 목록으로 남긴다.
///
/// `2007` 세션 중복은 실패가 아니라 축출이고(연§6-1), `3003` 은 발행처가 없어졌다.
pub const RETIRED: &[u16] = &[2007, 3003];

impl Code {
    /// 앞자리. ★**모르는 번호도 이것만으로 처방이 선다.**
    pub fn family_of(v: u16) -> Option<Family> {
        match v / 1000 {
            1 => Some(Family::Request),
            2 => Some(Family::Identity),
            3 => Some(Family::State),
            4 => Some(Family::Limit),
            5 => Some(Family::Server),
            _ => None,
        }
    }

    pub fn family(self) -> Family {
        Code::family_of(self.as_u16()).expect("코드는 1xxx~5xxx 다")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 번호_이름_왕복() {
        for &c in Code::ALL {
            assert_eq!(Code::from_u16(c.as_u16()), Some(c));
            assert!(!c.name().is_empty());
        }
    }

    #[test]
    fn 폐기_번호는_내지_않는다() {
        for &v in RETIRED {
            assert_eq!(Code::from_u16(v), None, "★{v} 는 비워 둔 번호다");
        }
    }

    #[test]
    fn 앞자리가_처방을_준다() {
        // ★모르는 번호 — 표에 없어도 앞자리로 답이 나와야 한다.
        assert_eq!(Code::family_of(1999), Some(Family::Request));
        assert_eq!(Code::family_of(5999), Some(Family::Server));
        assert_eq!(Code::family_of(999), None);
    }

    #[test]
    fn 일천번대는_전부_영구다() {
        for &c in Code::ALL {
            if c.family() == Family::Request && c.permanent().is_some() {
                assert_eq!(c.permanent(), Some(true), "{:?}", c);
            }
        }
    }

    #[test]
    fn 사천오천번대는_영구가_아니다() {
        for &c in Code::ALL {
            if matches!(c.family(), Family::Limit | Family::Server) {
                assert_ne!(c.permanent(), Some(true), "{:?}", c);
            }
        }
    }

    #[test]
    fn leave_사유는_permanent_축이_없다() {
        // ★`LEAVE` 는 응답이 아니라 종료 통지다 — 재시도 축을 씌우면 뜻이 갈린다.
        for c in [
            Code::ProtocolError,
            Code::SessionRevoked,
            Code::DuplicateSession,
            Code::FlowTimeout,
            Code::FlowOverflow,
            Code::HeartbeatTimeout,
            Code::ServerShutdown,
        ] {
            assert_eq!(c.permanent(), None, "{c:?}");
        }
    }
}
