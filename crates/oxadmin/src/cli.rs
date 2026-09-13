// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §6 · §1 · model: claude-opus-5

//! 인자와 해석 사다리 — ★**어디 붙었나를 매번 보여준다.**
//!
//! ★★**안 보이면 옆 hub 를 보고도 모른다**(운영 §6 증상). 개발기에 hub 가 둘 이상 뜨는
//! 것이 정상 형상이라(운영 §8) 이 한 줄이 없으면 *"값이 이상하다"* 의 절반이 ★**엉뚱한
//! 곳을 본 것**이다 — 실측 20260913(node 셋 형상).

/// 붙을 곳이 어디서 왔나. ★**값과 출처를 같이 든다** — 출처 없이 주소만 찍으면
/// *"왜 거기냐"* 를 다시 따져야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub url: String,
    pub from: &'static str,
}

/// 내장 기본 — 사다리의 맨 아래.
pub const DEFAULT_URL: &str = "http://127.0.0.1:19745";
pub const ENV_URL: &str = "OXADMIN_URL";

/// 해석 사다리 ★`--hub` > env `OXADMIN_URL` > 내장 기본(운영 §6).
pub fn resolve(hub: Option<&str>, env: Option<String>) -> Endpoint {
    match (hub, env) {
        (Some(v), _) => Endpoint { url: normalize(v), from: "--hub" },
        (None, Some(v)) if !v.is_empty() => Endpoint { url: normalize(&v), from: ENV_URL },
        _ => Endpoint { url: DEFAULT_URL.to_string(), from: "기본값" },
    }
}

/// `host:port` 만 줘도 받는다 — ★**사람이 치는 값**이다.
fn normalize(v: &str) -> String {
    let v = v.trim_end_matches('/');
    if v.starts_with("http://") || v.starts_with("https://") {
        v.to_string()
    } else {
        format!("http://{v}")
    }
}

/// 파싱된 명령줄. ★**표가 기본이고 `--json` 이 자동화용**이다(운영 §6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cli {
    pub args: Vec<String>,
    pub hub: Option<String>,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub json: bool,
    pub all: bool,
    pub yes: bool,
    pub help: bool,
}

/// 값을 받지 않는 것 — ★**먼저 걸러야** 짝짓기가 뒤엣것을 안 삼킨다.
const FLAGS: &[&str] = &["--json", "--all", "--yes", "--help", "-h"];

pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Cli, String> {
    let items: Vec<String> = argv.into_iter().collect();
    let mut out = Cli::default();
    let mut i = 0;
    while i < items.len() {
        let k = items[i].as_str();
        if FLAGS.contains(&k) {
            match k {
                "--json" => out.json = true,
                "--all" => out.all = true,
                "--yes" => out.yes = true,
                _ => out.help = true,
            }
            i += 1;
            continue;
        }
        if !k.starts_with("--") {
            out.args.push(k.to_string());
            i += 1;
            continue;
        }
        // ★**모르는 인자는 실패다** — 조용히 무시하면 오타가 「기본 동작」이 된다.
        let v = items.get(i + 1).cloned().ok_or_else(|| format!("{k} 는 값을 받는다"))?;
        match k {
            "--hub" => out.hub = Some(v),
            "--api-key" => out.api_key = Some(v),
            "--api-secret" => out.api_secret = Some(v),
            _ => return Err(format!("모르는 인자: {k}")),
        }
        i += 2;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 사다리는_인자가_환경을_이긴다() {
        // ★순서가 뒤집히면 env 를 켜 둔 터미널에서 `--hub` 가 조용히 무시된다.
        let e = resolve(Some("127.0.0.1:19746"), Some("http://127.0.0.1:19745".into()));
        assert_eq!(e, Endpoint { url: "http://127.0.0.1:19746".into(), from: "--hub" });
        let e = resolve(None, Some("127.0.0.1:19747".into()));
        assert_eq!(e.from, ENV_URL);
        assert_eq!(e.url, "http://127.0.0.1:19747");
        assert_eq!(resolve(None, None).url, DEFAULT_URL);
        // ★빈 문자열은 「안 준 것」이다 — 안 그러면 `OXADMIN_URL=` 하나로 도구가 죽는다.
        assert_eq!(resolve(None, Some(String::new())).from, "기본값");
    }

    #[test]
    fn 사람이_치는_값을_받는다() {
        assert_eq!(normalize("127.0.0.1:19745/"), "http://127.0.0.1:19745");
        assert_eq!(normalize("https://ops.example:443"), "https://ops.example:443");
    }

    #[test]
    fn 모르는_인자는_실패다() {
        // ★조용히 무시하면 `--jsno` 가 「표로 낸다」로 읽히고 스크립트가 깨진 채 돈다.
        assert!(parse(["rooms".to_string(), "--jsno".to_string(), "x".to_string()]).is_err());
        let c = parse(["room".into(), "r1".into(), "--json".into()]).expect("parse");
        assert_eq!(c.args, vec!["room", "r1"]);
        assert!(c.json && !c.all);
    }

    #[test]
    fn 깃발은_뒤엣것을_안_삼킨다() {
        // ★`--json` 이 값을 받는 것으로 읽히면 `rooms` 가 인자로 먹힌다.
        let c = parse(["--json".into(), "rooms".into()]).expect("parse");
        assert_eq!(c.args, vec!["rooms"]);
    }
}
