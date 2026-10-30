// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-BOOT-ARGS · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 기동 인자 — `--이름 값` · 아는 이름만 받는다(모르는 인자는 기동 실패) · 배너는 앞 `--` 를 뗀 이름으로 싣는다.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Args {
    values: BTreeMap<String, String>,
    flags: Vec<String>,
    known: Vec<&'static str>,
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
    pub fn parse<I: IntoIterator<Item = String>>(
        it: I,
        known: &[&'static str],
        flags: &[&'static str],
    ) -> Result<Args, ArgError> {
        let mut a = Args { known: known.to_vec(), ..Default::default() };
        let mut it = it.into_iter();
        while let Some(x) = it.next() {
            let Some(name) = x.strip_prefix("--") else {
                return Err(ArgError(format!("unexpected argument {x}")));
            };
            let (name, inline) = match name.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (name.to_string(), None),
            };
            if flags.contains(&name.as_str()) {
                a.flags.push(name);
                continue;
            }
            if !known.contains(&name.as_str()) {
                return Err(ArgError(format!("unknown argument --{name}")));
            }
            let v = match inline {
                Some(v) => v,
                None => it.next().ok_or_else(|| ArgError(format!("--{name} needs a value")))?,
            };
            a.values.insert(name, v);
        }
        Ok(a)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(|s| s.as_str())
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    pub fn require(&self, name: &str) -> Result<&str, ArgError> {
        self.get(name).ok_or_else(|| ArgError(format!("--{name} is required")))
    }

    pub fn parse_num<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, ArgError> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v.parse::<T>().map(Some).map_err(|_| ArgError(format!("--{name} {v} is not a number"))),
        }
    }

    pub fn banner_fields(&self) -> Vec<(String, String)> {
        self.known
            .iter()
            .map(|k| (k.to_string(), self.get(k).map(|v| v.to_string()).unwrap_or_else(|| "-".to_string())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_and_banner() {
        let a = Args::parse(
            ["--id", "n1", "--udp-port=20000"].iter().map(|s| s.to_string()),
            &["id", "udp-port", "build"],
            &["version"],
        )
        .unwrap();
        assert_eq!(a.get("id"), Some("n1"));
        assert_eq!(a.parse_num::<u16>("udp-port").unwrap(), Some(20000));
        assert_eq!(a.banner_fields()[2], ("build".to_string(), "-".to_string()));
        assert!(Args::parse(["--nope", "1"].iter().map(|s| s.to_string()), &["id"], &[]).is_err());
    }
}
