// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 기동 배너 — 그 프로세스가 읽는 세 자리의 키 전량을 출처 이름 그대로. 여러 줄(`part` · `parts`) · 칸은 줄 안에서만 끊지 않는다 · 홀로 줄 상한을 넘는 칸은 기동 실패. 비밀은 `***` · 없는 선택 키는 `-`.

#[derive(Debug, Clone, Default)]
pub struct Banner {
    fields: Vec<(String, String)>,
}

pub const SECRET: &str = "***";
pub const ABSENT: &str = "-";

impl Banner {
    pub fn new() -> Banner {
        Banner::default()
    }

    pub fn put(&mut self, key: impl Into<String>, value: impl std::fmt::Display) {
        self.fields.push((key.into(), value.to_string()));
    }

    pub fn put_opt<T: std::fmt::Display>(&mut self, key: impl Into<String>, value: Option<T>) {
        match value {
            Some(v) => self.put(key, v),
            None => self.put(key, ABSENT),
        }
    }

    pub fn put_secret(&mut self, key: impl Into<String>, present: bool) {
        self.put(key, if present { SECRET } else { ABSENT });
    }

    pub fn put_list<T: std::fmt::Display>(&mut self, key: impl Into<String>, values: &[T]) {
        let joined = values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
        self.put(key, format!("[{joined}]"));
    }

    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    pub fn render(&self) -> String {
        self.fields.iter().map(|(k, v)| format!("{k}={}", crate::log::line::quote(v))).collect::<Vec<_>>().join(" ")
    }

    pub fn split(&self, line_max: usize, line_of: &dyn Fn(usize, usize, &str) -> String) -> Result<Vec<String>, String> {
        split(&self.fields, line_max, line_of)
    }
}

pub const PARTS_WIDTH: usize = 999;

pub fn split(fields: &[(String, String)], line_max: usize, line_of: &dyn Fn(usize, usize, &str) -> String) -> Result<Vec<String>, String> {
    let cells: Vec<(String, String)> = fields.iter().map(|(k, v)| (k.clone(), format!("{k}={}", crate::log::line::quote(v)))).collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (key, cell) in &cells {
        let part = chunks.len() + 1;
        let candidate = if cur.is_empty() { cell.clone() } else { format!("{cur} {cell}") };
        if line_of(part, PARTS_WIDTH, &candidate).len() <= line_max {
            cur = candidate;
            continue;
        }
        if cur.is_empty() || line_of(part + 1, PARTS_WIDTH, cell).len() > line_max {
            return Err(key.clone());
        }
        chunks.push(std::mem::take(&mut cur));
        cur = cell.clone();
    }
    if !cur.is_empty() || chunks.is_empty() {
        chunks.push(cur);
    }
    let parts = chunks.len();
    Ok(chunks.iter().enumerate().map(|(i, c)| line_of(i + 1, parts, c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_absent_are_marked() {
        let mut b = Banner::new();
        b.put("hub.listen", "0.0.0.0:1974");
        b.put_secret("hub.auth.jwt_secret", true);
        b.put_opt::<String>("hub.log_dir", None);
        b.put_list("supervisor.backoff_ms", &[200, 500]);
        assert_eq!(b.render(), "hub.listen=0.0.0.0:1974 hub.auth.jwt_secret=*** hub.log_dir=- supervisor.backoff_ms=[200,500]");
    }

    fn line(part: usize, parts: usize, msg: &str) -> String {
        format!("head part={part} parts={parts} msg={}", crate::log::line::quote(msg))
    }

    #[test]
    fn many_cells_split_into_parts_without_cutting_a_cell() {
        let mut b = Banner::new();
        for i in 0..40 {
            b.put(format!("unit[{i}].cmd"), format!("./target/debug/oxsfud --grpc-listen 127.0.0.1:{}", 50000 + i));
        }
        let lines = b.split(300, &line).expect("fits");
        assert!(lines.len() > 1, "O-sym-RULE-BOOT-ARGS-4 여러 줄");
        assert!(lines.iter().all(|l| l.len() <= 300), "O-sym-RULE-BOOT-ARGS-4 줄마다 상한 안");
        assert!(lines.iter().all(|l| !l.contains("truncated")), "O-sym-RULE-BOOT-ARGS-4 배너 줄은 자르지 않는다");
        let n = lines.len();
        for (i, l) in lines.iter().enumerate() {
            assert!(l.starts_with(&format!("head part={} parts={n} ", i + 1)), "O-sym-RULE-BOOT-ARGS-4 part · parts — {l}");
        }
        let unquote = |q: &str| q.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(q).replace("\\\"", "\"").replace("\\\\", "\\");
        let joined: Vec<String> = lines.iter().map(|l| l.split_once("msg=").map(|x| unquote(x.1)).unwrap_or_default()).collect();
        assert_eq!(joined.join(" "), b.render(), "O-sym-RULE-BOOT-ARGS-3 칸 차례는 한 줄일 때와 같다 · 칸 전량");
    }

    #[test]
    fn one_line_when_it_fits() {
        let mut b = Banner::new();
        b.put("a", 1);
        b.put("b", 2);
        assert_eq!(b.split(1024, &line).expect("fits"), vec!["head part=1 parts=1 msg=\"a=1 b=2\"".to_string()], "O-sym-RULE-BOOT-ARGS-4 들어가면 한 줄 · part=1 parts=1");
    }

    #[test]
    fn a_cell_alone_over_the_cap_names_itself() {
        let mut b = Banner::new();
        b.put("ok", 1);
        b.put("unit[0].cmd", "x".repeat(400));
        b.put("after", 2);
        assert_eq!(b.split(300, &line), Err("unit[0].cmd".to_string()), "O-sym-RULE-BOOT-ARGS-4 홀로 넘치는 칸은 기동 실패 · 그 칸을 짚는다");
    }
}
