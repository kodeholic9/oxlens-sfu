// author: kodeholic (powered by Claude)
//! WS Close 사유 — 연§10-3. Close 는 프레임이 아니라 연결 전체의 종료 사유다.
//! 클라는 `code` 로 판단한다. `reason` 은 이름 문자열 그대로(ASCII, 123B 이하).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CloseCode {
    ProtocolError = 4000,
    FlowTimeout = 4001,
    FlowOverflow = 4002,
    HeartbeatTimeout = 4003,
    TokenExpired = 4004,
    DuplicateSession = 4005,
    ServerShutdown = 4006,
}

impl CloseCode {
    pub const ALL: [CloseCode; 7] = [
        CloseCode::ProtocolError, CloseCode::FlowTimeout, CloseCode::FlowOverflow, CloseCode::HeartbeatTimeout,
        CloseCode::TokenExpired, CloseCode::DuplicateSession, CloseCode::ServerShutdown,
    ];

    pub fn code(self) -> u16 {
        self as u16
    }

    pub fn from_code(code: u16) -> Option<CloseCode> {
        CloseCode::ALL.iter().copied().find(|c| c.code() == code)
    }

    pub fn reason(self) -> &'static str {
        match self {
            CloseCode::ProtocolError => "PROTOCOL_ERROR",
            CloseCode::FlowTimeout => "FLOW_TIMEOUT",
            CloseCode::FlowOverflow => "FLOW_OVERFLOW",
            CloseCode::HeartbeatTimeout => "HEARTBEAT_TIMEOUT",
            CloseCode::TokenExpired => "TOKEN_EXPIRED",
            CloseCode::DuplicateSession => "DUPLICATE_SESSION",
            CloseCode::ServerShutdown => "SERVER_SHUTDOWN",
        }
    }
}

/// 연§10-3 — `4000`·`4001`·`4002`·`4005` 만 재접속을 막는다. 모르는 코드는 다시 붙는다.
pub fn reconnectable(code: u16) -> bool {
    !matches!(code, 4000 | 4001 | 4002 | 4005)
}

/// 연§10-3 — `4004` 만 새 토큰이 필요하다.
pub fn needs_new_token(code: u16) -> bool {
    code == 4004
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_names_fit_ws_limit_and_policy() {
        for c in CloseCode::ALL {
            assert!(c.reason().len() <= 123);
            assert!(c.reason().is_ascii());
            assert_eq!(CloseCode::from_code(c.code()), Some(c));
        }
        assert!(!reconnectable(4005));
        assert!(reconnectable(4003));
        assert!(reconnectable(4006));
        assert!(reconnectable(4999));
        assert!(needs_new_token(4004));
    }
}
