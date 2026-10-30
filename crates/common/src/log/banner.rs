// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-BOOT-ARGS · 정:RULE-CFG-THREE-PLACES · 운영:LOG.log.LINE · model: claude-opus-5-5
//! 기동 배너 — 해석된 최종값 전량을 여러 줄로 · 칸은 한 줄 안에서 끊지 않는다 · `part` · `parts` · `build` 는 `part=1` 에만 · 홀로 넘치는 칸은 기동 실패.

use super::line::{Head, Line, push_kv};

pub const SPEC: &str = "SV-STATE-UNIT-STATE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerError(pub String);

impl std::fmt::Display for BannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "banner: {}", self.0)
    }
}

impl std::error::Error for BannerError {}

pub struct Banner<'a> {
    pub head: &'a Head,
    pub epoch: Option<&'a str>,
    pub build: &'a str,
    pub unit: Option<&'a str>,
    pub msg: &'a str,
    pub fields: &'a [(String, String)],
    pub line_max: usize,
}

const TS_WIDTH: usize = 32;
const MONO_WIDTH: usize = 20;
const PART_WIDTH: usize = 24;

impl Banner<'_> {
    fn base(&self, part: usize) -> Line {
        let mut l = Line::ev(SPEC);
        if let Some(e) = self.epoch {
            l = l.key("epoch", e);
        }
        if let Some(u) = self.unit {
            l = l.key("unit", u);
        }
        if part == 1 {
            l = l.rec("build", self.build);
        }
        l
    }

    fn overhead(&self, part: usize) -> usize {
        let mut l = self.base(part);
        l.msg = Some(self.msg.to_string());
        l.render(self.head, &"0".repeat(TS_WIDTH), 0, usize::MAX).len() + MONO_WIDTH + PART_WIDTH
    }

    pub fn split(&self) -> Result<Vec<Vec<(String, String)>>, BannerError> {
        let mut groups: Vec<Vec<(String, String)>> = vec![Vec::new()];
        let mut used = 0usize;
        for (k, v) in self.fields {
            let mut one = String::new();
            push_kv(&mut one, k, v);
            let need = one.len() + one.matches(['"', '\\']).count() + 1;
            let part = groups.len();
            let room = self.line_max.saturating_sub(self.overhead(part));
            if need > self.line_max.saturating_sub(self.overhead(2).max(self.overhead(1))) && need > room {
                return Err(BannerError(format!("field {k} alone exceeds line_max {}", self.line_max)));
            }
            if used + need > room && !groups.last().map(|g| g.is_empty()).unwrap_or(true) {
                groups.push(Vec::new());
                used = 0;
            }
            used += need;
            if let Some(g) = groups.last_mut() {
                g.push((k.clone(), v.clone()));
            }
        }
        Ok(groups)
    }

    pub fn lines(&self) -> Result<Vec<Line>, BannerError> {
        let groups = self.split()?;
        let parts = groups.len();
        Ok(groups
            .into_iter()
            .enumerate()
            .map(|(i, g)| {
                let mut l = self.base(i + 1).rec("part", i + 1).rec("parts", parts);
                l.msg = Some(format!("{} {}", self.msg, super::line::kvs(&g)));
                l
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_fields_split_and_fit() {
        let head = Head { node: "node-a".into(), proc_name: "hub" };
        let fields: Vec<(String, String)> =
            (0..80).map(|i| (format!("unit[{i}].cmd"), format!("./target/debug/oxsfud --grpc-listen 127.0.0.1:{i}"))).collect();
        let b = Banner { head: &head, epoch: None, build: "abc.1", unit: None, msg: "boot", fields: &fields, line_max: 1024 };
        let lines = b.lines().unwrap();
        assert!(lines.len() > 1);
        let n: usize = lines.iter().map(|l| l.msg.as_ref().map(|r| r.matches("].cmd=").count()).unwrap_or(0)).sum();
        assert_eq!(n, 80);
        for (i, l) in lines.iter().enumerate() {
            let s = l.render(&head, &super::super::line::ts_now(), 123_456_789, 1024);
            assert!(s.len() <= 1024, "{}", s.len());
            assert!(!s.contains("truncated"));
            assert_eq!(s.contains("build="), i == 0);
        }
    }

    #[test]
    fn lone_field_over_limit_fails() {
        let head = Head { node: "n".into(), proc_name: "sfu" };
        let fields = vec![("x".to_string(), "y".repeat(2000))];
        let b = Banner { head: &head, epoch: Some("e"), build: "b", unit: None, msg: "boot", fields: &fields, line_max: 1024 };
        assert!(b.lines().is_err());
    }
}
