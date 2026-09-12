// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§18-1 · §15-6 · §16-1 · 운영 §7 · §8-2 · model: claude-opus-5

//! 기동 — ★**인자 > 파일 > 코드 기본값**을 한 자리에서 푼다.
//!
//! ★★**기동 배너 한 줄이 해석된 최종값 전량**이다. 인자가 권위라 파일에 안 남으므로
//! ★**로그가 유일한 사후 재구성 경로**다.

use common::{Args, BuildId, Policy, System};

/// 풀린 값. ★**여기서 나온 뒤로는 아무도 다시 안 푼다** — 두 곳에서 풀면 갈린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub node_id: String,
    pub build: BuildId,
    pub listen: String,
    pub log_dir: Option<String>,
    pub system: System,
    pub policy: Policy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootError(pub String);

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "boot: {}", self.0)
    }
}

impl std::error::Error for BootError {}

impl Resolved {
    /// ★**한 번만 푼다.** 파일을 읽는 것은 부르는 쪽이고 여기는 값만 받는다 —
    /// 그래야 1층이 파일 없이 해석 규칙을 시험한다.
    pub fn new(args: &Args, system: System, policy: Policy) -> Result<Self, BootError> {
        let node_id = match &args.id {
            Some(v) if !v.trim().is_empty() => v.clone(),
            // ★`node_id` 는 배치 키다 — 없으면 방이 어디로 갈지가 기동마다 달라진다.
            _ => return Err(BootError("--id 가 없다 — node_id 는 안정 배치 키라 필수다".into())),
        };
        // ★인자가 파일을 덮는다.
        let listen = args.listen.clone().unwrap_or_else(|| system.hub.listen.clone());
        let log_dir = match (&args.log_dir, system.dirs.log.as_str()) {
            (Some(v), _) if !v.trim().is_empty() => Some(v.clone()),
            (_, f) if !f.trim().is_empty() => Some(f.to_string()),
            // ★없으면 stdout 이다 — 지어내지 않는다.
            _ => None,
        };
        Ok(Self {
            node_id,
            build: BuildId::new(args.build.clone()),
            listen,
            log_dir,
            system,
            policy,
        })
    }

    /// ★**해석된 최종값 전량** — 무엇이 인자에서 왔는지까지.
    pub fn banner(&self, args: &Args) -> String {
        format!(
            "[boot] {} level={:?} rotate={} retention={}d units={} zenoh={}",
            args.banner(&self.listen, self.log_dir.as_deref()),
            self.policy.logging.level,
            self.policy.logging.rotate_max_bytes,
            self.policy.logging.retention_days,
            self.system.units.len(),
            self.system.zenoh.mode,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(listen: &str, log: &str) -> System {
        System::parse(&format!(
            "[hub]\nlisten = \"{listen}\"\n[hub.auth]\njwt_secret = \"s\"\n[dirs]\nlog = \"{log}\"\n"
        ))
        .expect("system")
    }

    #[test]
    fn id_가_없으면_안_뜬다() {
        let a = Args::parse(["--listen", "x"]).expect("args");
        assert!(Resolved::new(&a, sys("0.0.0.0:1974", ""), Policy::default()).is_err());
    }

    #[test]
    fn 인자가_파일을_덮는다() {
        let a = Args::parse(["--id", "hub-1", "--listen", "127.0.0.1:1975", "--log-dir", "/tmp/a"])
            .expect("args");
        let r = Resolved::new(&a, sys("0.0.0.0:1974", "/var/log/ox"), Policy::default()).expect("r");
        assert_eq!(r.listen, "127.0.0.1:1975");
        assert_eq!(r.log_dir.as_deref(), Some("/tmp/a"));
    }

    #[test]
    fn 인자가_없으면_파일이_선다() {
        let a = Args::parse(["--id", "hub-1"]).expect("args");
        let r = Resolved::new(&a, sys("0.0.0.0:1974", "/var/log/ox"), Policy::default()).expect("r");
        assert_eq!(r.listen, "0.0.0.0:1974");
        assert_eq!(r.log_dir.as_deref(), Some("/var/log/ox"));
    }

    #[test]
    fn 둘_다_없으면_stdout_이다() {
        let a = Args::parse(["--id", "hub-1"]).expect("args");
        let r = Resolved::new(&a, sys("0.0.0.0:1974", ""), Policy::default()).expect("r");
        assert!(r.log_dir.is_none(), "★없는 것을 지어내지 않는다");
    }

    #[test]
    fn 배너가_해석_결과_전량을_낸다() {
        let a = Args::parse(["--id", "hub-1", "--build", "abc123"]).expect("args");
        let r = Resolved::new(&a, sys("0.0.0.0:1974", ""), Policy::default()).expect("r");
        let b = r.banner(&a);
        assert!(b.contains("id=hub-1"), "{b}");
        assert!(b.contains("build=abc123"), "{b}");
        assert!(b.contains("listen=0.0.0.0:1974(파일)"), "{b}");
        assert!(b.contains("units=0"), "{b}");
        assert!(b.contains("zenoh=router"), "{b}");
    }
}
