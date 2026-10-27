// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-CFG-THREE-PLACES · 운영:RULE-BOOT-ARGS · model: claude-opus-5-5
//! 기동 배너 — 그 프로세스가 읽는 세 자리의 키 전량을 출처 이름 그대로 한 줄에. 비밀은 `***` · 없는 선택 키는 `-`.

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
}
