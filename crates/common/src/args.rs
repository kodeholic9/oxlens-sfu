// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 기동 인자 — `--이름 값` · `--이름`(깃발). 모르는 인자는 기동 실패(파일과 같은 strict).

use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Args {
    values: BTreeMap<String, String>,
    flags: Vec<String>,
}

impl Args {
    pub fn parse(argv: impl IntoIterator<Item = String>, valued: &[&str], flags: &[&str]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut it = argv.into_iter();
        while let Some(x) = it.next() {
            let Some(name) = x.strip_prefix("--") else {
                return Err(format!("인자 {x} 를 모른다"));
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            if flags.contains(&name.as_str()) {
                a.flags.push(name);
                continue;
            }
            if !valued.contains(&name.as_str()) {
                return Err(format!("인자 --{name} 를 모른다"));
            }
            let v = match inline {
                Some(v) => v,
                None => it.next().ok_or_else(|| format!("--{name} 에 값이 없다"))?,
            };
            a.values.insert(name, v);
        }
        Ok(a)
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.values.get(k).map(String::as_str)
    }

    pub fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }

    pub fn all(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_and_both_forms() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let a = Args::parse(v(&["--id", "n1", "--policy=p.toml", "--version"]), &["id", "policy"], &["version"]).unwrap_or_default();
        assert_eq!(a.get("id"), Some("n1"));
        assert_eq!(a.get("policy"), Some("p.toml"));
        assert!(a.flag("version"));
        assert!(Args::parse(v(&["--bogus", "1"]), &["id"], &[]).is_err());
    }
}
