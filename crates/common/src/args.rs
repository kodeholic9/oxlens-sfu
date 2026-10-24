// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-CFG-THREE-PLACES · 정:IMPL-CFG-SYSTEM-FILE · 운영 규격서 §8-2 · model: claude-opus-5-5
//! 기동 인자 — 아는 이름만 받는다(모르는 인자 · 값 없는 인자는 기동 실패). 인자 > 파일 > 코드 기본값.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Args {
    values: BTreeMap<String, String>,
    flags: Vec<String>,
}

impl Args {
    pub fn parse(argv: &[String], valued: &[&str], switches: &[&str]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut i = 0;
        while i < argv.len() {
            let raw = &argv[i];
            let Some(name) = raw.strip_prefix("--") else {
                return Err(format!("모르는 인자 {raw}"));
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            if switches.contains(&name.as_str()) {
                a.flags.push(name);
            } else if valued.contains(&name.as_str()) {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        argv.get(i).cloned().ok_or_else(|| format!("--{name} 에 값이 없다"))?
                    }
                };
                a.values.insert(name, v);
            } else {
                return Err(format!("모르는 인자 --{name}"));
            }
            i += 1;
        }
        Ok(a)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    pub fn require(&self, name: &str) -> Result<&str, String> {
        self.get(name).ok_or_else(|| format!("--{name} 가 없다"))
    }

    pub fn banner(&self) -> String {
        let mut parts: Vec<String> = self.values.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.extend(self.flags.iter().map(|f| format!("{f}=1")));
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn strict_parse() {
        let a = Args::parse(&v(&["--id", "n1", "--policy=p.toml", "--version"]), &["id", "policy"], &["version"]).expect("ok");
        assert_eq!(a.get("id"), Some("n1"));
        assert_eq!(a.get("policy"), Some("p.toml"));
        assert!(a.has("version"));
        assert!(Args::parse(&v(&["--nope"]), &["id"], &[]).is_err());
        assert!(Args::parse(&v(&["--id"]), &["id"], &[]).is_err());
        assert!(Args::parse(&v(&["bare"]), &["id"], &[]).is_err());
    }
}
