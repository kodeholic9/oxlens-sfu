// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §3-4 · §8-2 · 정§16-1 · model: claude-opus-5

//! 빌드 신원 — ★**돌고 있는 것이 어느 소스의 산물인지 서버가 스스로 말한다.**
//!
//! ★**사람이 기억으로 막을 일이 아니다** — 하네스가 옛 바이너리를 상대로 초록을 내는 것을
//! 구조로 봉쇄하는 자리다(`--build` 로 받은 값을 `/admin/snapshot` 이 그대로 낸다).

/// ★**컴파일 때 찍힌 신원** — 이 바이너리가 어느 소스의 산물인가.
pub const STAMP: &str = env!("OX_BUILD_STAMP");

/// 기동 인자가 준 **배포 라벨**. 안 주면 `"unknown"` — ★**지어내지 않는다.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildId(String);

impl BuildId {
    pub const UNKNOWN: &'static str = "unknown";

    pub fn new(v: Option<String>) -> Self {
        match v {
            Some(s) if !s.trim().is_empty() => BuildId(s),
            // ★빈 문자열을 받아 주면 "값이 있다"와 "없다"가 화면에서 같아진다.
            _ => BuildId(Self::UNKNOWN.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_known(&self) -> bool {
        self.0 != Self::UNKNOWN
    }

    /// 밖에 내는 한 줄 — ★**소스 신원이 앞이고 라벨이 뒤다.**
    pub fn line(&self) -> String {
        if self.is_known() {
            format!("{STAMP} ({})", self.0)
        } else {
            STAMP.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 바이너리가_스스로_말한다() {
        // ★컴파일 때 찍힌다 — 사람이 안 줘도 소스 신원이 나온다.
        assert!(!STAMP.is_empty());
        assert!(STAMP.contains('+'), "SHA+시각 꼴이어야 한다: {STAMP}");
        assert_eq!(BuildId::new(None).line(), STAMP, "★라벨이 없어도 신원은 있다");
        assert!(BuildId::new(Some("배포7".into())).line().contains("배포7"));
    }

    #[test]
    fn 없으면_모른다고_말한다() {
        assert_eq!(BuildId::new(None).as_str(), "unknown");
        assert!(!BuildId::new(Some("  ".into())).is_known(), "★빈 값은 값이 아니다");
        assert!(BuildId::new(Some("a1b2c3".into())).is_known());
    }
}
