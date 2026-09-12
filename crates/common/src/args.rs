// author: kodeholic (powered by Claude)
// spec: v1.1 · 정§18-1 · 운영 §8-2 · model: claude-opus-5

//! 기동 인자 — ★**프로세스가 2개 떴을 때 서로 달라져야 하는 값**이 여기다(같아야 하면 파일).
//!
//! ★**인자 > 파일 > 코드 기본값.** 인자가 권위라 파일에 안 남으므로
//! ★**기동 배너 한 줄이 해석된 최종값 전량**을 남기는 것이 유일한 사후 재구성 경로다.

/// 값을 받지 않는 인자 — ★**먼저 걸러야 한다.** 아래 짝짓기가 무조건 하나를 삼킨다.
const FLAGS: &[&str] = &["--help", "--version"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub system: Option<String>,
    pub policy: Option<String>,
    /// ★`node_id` — 안정 배치 키. `sfu_id`(기동마다 새 값)와 다른 값이다.
    pub id: Option<String>,
    pub build: Option<String>,
    /// ★**파일값을 덮는다.**
    pub listen: Option<String>,
    /// ★**파일값을 덮는다**(정본은 시스템 파일 — 두 프로세스가 같아야 하는 값이다).
    pub log_dir: Option<String>,
    pub help: bool,
    pub version: bool,
    /// ★**그 바이너리만 아는 인자** — 부르는 쪽이 미리 선언한 것만 여기 담긴다.
    ///
    /// ★**선언하지 않은 것은 여전히 기동 실패다** — 엄격함을 잃지 않으면서
    /// hub 가 sfud 의 인자를 알 필요도, 그 반대도 없게 한다.
    pub extra: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgError(pub String);

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "args: {}", self.0)
    }
}

impl std::error::Error for ArgError {}

impl Args {
    /// ★**모르는 인자는 기동 실패다** — strict 파싱(정§18-1).
    pub fn parse<I, S>(argv: I) -> Result<Args, ArgError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Args::parse_allowing(argv, &[])
    }

    /// 그 바이너리만 아는 인자를 함께 받는다 — ★**선언한 것만**.
    pub fn parse_allowing<I, S>(argv: I, extra_keys: &[&str]) -> Result<Args, ArgError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let items: Vec<String> = argv.into_iter().map(|s| s.as_ref().to_string()).collect();
        let mut out = Args::default();
        let mut i = 0;
        while i < items.len() {
            let key = items[i].as_str();
            if FLAGS.contains(&key) {
                match key {
                    "--help" => out.help = true,
                    "--version" => out.version = true,
                    _ => unreachable!(),
                }
                i += 1;
                continue;
            }
            let val = items.get(i + 1).cloned().ok_or_else(|| {
                ArgError(format!("{key} 는 값을 받는다 — 값이 없다"))
            })?;
            match key {
                "--system" => out.system = Some(val),
                "--policy" => out.policy = Some(val),
                "--id" => out.id = Some(val),
                "--build" => out.build = Some(val),
                "--listen" => out.listen = Some(val),
                "--log-dir" => out.log_dir = Some(val),
                k if extra_keys.contains(&k) => {
                    out.extra.insert(k.to_string(), val);
                }
                _ => return Err(ArgError(format!("모르는 인자: {key}"))),
            }
            i += 2;
        }
        Ok(out)
    }

    /// 배너 한 줄 — ★**해석된 최종값 전량.** 인자가 없으면 무엇이 파일에서 왔는지도 보여야 한다.
    pub fn banner(&self, resolved_listen: &str, resolved_log_dir: Option<&str>) -> String {
        format!(
            "id={} build={} system={} policy={} listen={}{} log_dir={}{}",
            self.id.as_deref().unwrap_or("-"),
            self.build.as_deref().unwrap_or(crate::build::BuildId::UNKNOWN),
            self.system.as_deref().unwrap_or("-"),
            self.policy.as_deref().unwrap_or("-"),
            resolved_listen,
            if self.listen.is_some() { "(인자)" } else { "(파일)" },
            resolved_log_dir.unwrap_or("-"),
            if self.log_dir.is_some() { "(인자)" } else { "(파일)" },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 모르는_인자는_기동_실패다() {
        let e = Args::parse(["--nope", "x"]).unwrap_err();
        assert!(e.0.contains("--nope"), "{}", e.0);
    }

    #[test]
    fn 값을_안_받는_인자를_먼저_거른다() {
        // ★안 거르면 아래 짝짓기가 뒤엣것을 삼켜 `--id` 가 사라진다.
        let a = Args::parse(["--help", "--id", "hub-1"]).expect("parse");
        assert!(a.help);
        assert_eq!(a.id.as_deref(), Some("hub-1"));
    }

    #[test]
    fn 값이_빠지면_선다() {
        let e = Args::parse(["--id"]).unwrap_err();
        assert!(e.0.contains("값이 없다"), "{}", e.0);
    }

    #[test]
    fn 선언한_인자만_받는다() {
        // ★hub 가 sfud 의 인자를 알 필요가 없고, 그 반대도 없다.
        let a = Args::parse_allowing(["--id", "s1", "--udp-port", "20000"], &["--udp-port"])
            .expect("parse");
        assert_eq!(a.extra.get("--udp-port").map(String::as_str), Some("20000"));
        // ★선언하지 않으면 여전히 기동 실패다.
        assert!(Args::parse(["--id", "s1", "--udp-port", "20000"]).is_err());
    }

    #[test]
    fn 배너가_어디서_온_값인지_말한다() {
        let a = Args::parse(["--id", "hub-1", "--listen", "0.0.0.0:1974"]).expect("parse");
        let b = a.banner("0.0.0.0:1974", None);
        assert!(b.contains("listen=0.0.0.0:1974(인자)"), "{b}");
        assert!(b.contains("log_dir=-(파일)"), "{b}");
        assert!(b.contains("build=unknown"), "★안 준 값을 지어내지 않는다: {b}");
    }
}
