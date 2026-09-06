// author: kodeholic (powered by Claude)
//! 빌드 신원 — ★돌고 있는 것이 **어느 소스의 산물인지** 서버가 스스로 말한다.
//!
//! 없으면 옛 바이너리를 상대로 회귀를 돌고도 초록으로 읽는다. 20260906 에 두 번 그랬다:
//! 2층 하네스가 서버를 안 띄우고 이미 떠 있는 것에 붙는데, 그것이 7시간 전 빌드였다.
//! ★사람이 기억으로 막을 일이 아니다 — 물어보면 답하게 만든다.

/// `<git short rev>[-dirty]`. git 이 없으면 `unknown`.
pub const REV: &str = env!("OXLENS_BUILD_REV");

/// 빌드 시각(unix 초).
pub const BUILT_AT: &str = env!("OXLENS_BUILD_AT");

/// 한 줄로 — `abc1234-dirty@1757140000`. 견주기는 문자열 같음 하나면 된다.
pub fn stamp() -> String {
    format!("{REV}@{BUILT_AT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_always_says_something() {
        assert!(!REV.is_empty(), "빈 신원은 신원이 아니다");
        assert!(BUILT_AT.parse::<u64>().is_ok_and(|v| v > 0), "시각이 0 이면 견줄 수 없다");
        assert!(stamp().contains('@'));
    }
}
