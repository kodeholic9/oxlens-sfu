// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 기동 인자 — 프로세스마다 받는 이름을 선언하고, 모르는 인자는 기동 실패다(인자 > 코드 기본값).

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub name: &'static str,
    pub value: bool,
}

pub const fn opt(name: &'static str) -> Spec {
    Spec { name, value: true }
}

pub const fn flag(name: &'static str) -> Spec {
    Spec { name, value: false }
}

#[derive(Debug, Clone, Default)]
pub struct Args {
    values: BTreeMap<String, String>,
    order: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String], known: &[Spec]) -> Result<Args, String> {
        let mut out = Args::default();
        let mut i = 0;
        while i < argv.len() {
            let a = &argv[i];
            let Some(raw) = a.strip_prefix("--") else {
                return Err(format!("인자 꼴이 아니다: {a}"));
            };
            let (name, inline) = match raw.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (raw, None),
            };
            let Some(spec) = known.iter().find(|s| s.name == name) else {
                return Err(format!("모르는 인자: --{name}"));
            };
            let value = if spec.value {
                match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        argv.get(i).cloned().ok_or_else(|| format!("--{name} 에 값이 없다"))?
                    }
                }
            } else {
                "true".to_string()
            };
            if !out.values.contains_key(name) {
                out.order.push(name.to_string());
            }
            out.values.insert(name.to_string(), value);
            i += 1;
        }
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn has(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn require(&self, name: &str) -> Result<&str, String> {
        self.get(name).ok_or_else(|| format!("--{name} 가 없다(기본값 없음)"))
    }

    pub fn parse_num<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v.parse::<T>().map(Some).map_err(|_| format!("--{name} 값이 틀렸다: {v}")),
        }
    }

    pub fn names(&self) -> &[String] {
        &self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: &[Spec] = &[opt("id"), opt("policy"), flag("version")];

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_known_and_refuses_unknown() {
        let a = Args::parse(&v(&["--id", "n1", "--policy=p.toml", "--version"]), K).expect("ok");
        assert_eq!(a.get("id"), Some("n1"));
        assert_eq!(a.get("policy"), Some("p.toml"));
        assert!(a.has("version"));
        assert!(Args::parse(&v(&["--nope", "x"]), K).is_err());
        assert!(Args::parse(&v(&["--id"]), K).is_err());
        assert!(a.require("missing").is_err());
    }
}
