// author: kodeholic (powered by Claude)
// spec: v1.6 · 운영:RULE-LOG-LINE · 운영:LOG.log.LINE · 운영:LOG.log.TRANSITION · 운영:RULE-LOG-KEYS · model: claude-opus-5-5
//! 줄 한 개 — logfmt · 칸 차례 고정(머리 → 상관 키 → 레코드별 → `msg`) · 없는 키는 생략 · 줄 상한을 넘으면 `msg` 를 잘라 `truncated=1`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ev,
    Stat,
    Diag,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Ev => "ev",
            Kind::Stat => "stat",
            Kind::Diag => "diag",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "ev" => Some(Kind::Ev),
            "stat" => Some(Kind::Stat),
            "diag" => Some(Kind::Diag),
            _ => None,
        }
    }
}

pub const KEYS: [&str; 11] =
    ["session", "room", "user", "epoch", "unit", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq", "arev"];

pub fn key_index(name: &str) -> Option<usize> {
    KEYS.iter().position(|k| *k == name)
}

#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub spec: Option<String>,
    pub keys: [Option<String>; 11],
    pub recs: Vec<(String, String)>,
    pub raw: Option<String>,
    pub msg: Option<String>,
}

impl Line {
    pub fn new(kind: Kind) -> Line {
        Line { kind, spec: None, keys: Default::default(), recs: Vec::new(), raw: None, msg: None }
    }

    pub fn ev(spec: &str) -> Line {
        let mut l = Line::new(Kind::Ev);
        l.spec = Some(spec.to_string());
        l
    }

    pub fn diag(spec: &str) -> Line {
        let mut l = Line::new(Kind::Diag);
        l.spec = Some(spec.to_string());
        l
    }

    pub fn key(mut self, name: &str, v: impl ToString) -> Line {
        let v = v.to_string();
        if let Some(i) = key_index(name)
            && !v.is_empty()
        {
            self.keys[i] = Some(v);
        }
        self
    }

    pub fn rec(mut self, name: &str, v: impl ToString) -> Line {
        self.recs.push((name.to_string(), v.to_string()));
        self
    }

    pub fn msg(mut self, m: impl Into<String>) -> Line {
        let m = m.into();
        self.msg = if m.is_empty() { None } else { Some(m) };
        self
    }

    pub fn set_key_if_absent(&mut self, name: &str, v: &str) {
        if let Some(i) = key_index(name)
            && self.keys[i].is_none()
            && !v.is_empty()
        {
            self.keys[i] = Some(v.to_string());
        }
    }

    pub fn render(&self, head: &Head, ts: &str, mono_us: u64, max: usize) -> String {
        let mut out = String::with_capacity(256);
        push_kv(&mut out, "ts", ts);
        push_kv(&mut out, "mono", &mono_us.to_string());
        push_kv(&mut out, "node", &head.node);
        push_kv(&mut out, "proc", head.proc_name);
        push_kv(&mut out, "kind", self.kind.as_str());
        if self.kind != Kind::Stat
            && let Some(s) = &self.spec
        {
            push_kv(&mut out, "spec", s);
        }
        for (i, k) in KEYS.iter().enumerate() {
            if let Some(v) = &self.keys[i] {
                push_kv(&mut out, k, v);
            }
        }
        for (k, v) in &self.recs {
            push_kv(&mut out, k, v);
        }
        if let Some(r) = &self.raw
            && !r.is_empty()
        {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(r);
        }
        let Some(m) = &self.msg else {
            return out;
        };
        let mut full = out.clone();
        push_kv(&mut full, "msg", m);
        if full.len() <= max {
            return full;
        }
        push_kv(&mut out, "truncated", "1");
        let room = max.saturating_sub(out.len() + " msg=\"\"".len());
        let mut cut = String::new();
        for ch in m.chars() {
            let enc = quote(&ch.to_string());
            let piece = enc.trim_matches('"');
            if cut.len() + piece.len() > room {
                break;
            }
            cut.push(ch);
        }
        push_kv(&mut out, "msg", &cut);
        out
    }
}

#[derive(Debug, Clone)]
pub struct Head {
    pub node: String,
    pub proc_name: &'static str,
}

pub fn needs_quote(v: &str) -> bool {
    v.is_empty() || v.chars().any(|c| c == ' ' || c == '"' || c == '=' || c == '\\' || c.is_control())
}

pub fn quote(v: &str) -> String {
    if !needs_quote(v) {
        return v.to_string();
    }
    let mut s = String::with_capacity(v.len() + 2);
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

pub fn fields(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut it = line.chars().peekable();
    loop {
        while it.peek() == Some(&' ') {
            it.next();
        }
        let mut k = String::new();
        while let Some(&c) = it.peek() {
            if c == '=' || c == ' ' {
                break;
            }
            k.push(c);
            it.next();
        }
        if k.is_empty() && it.peek().is_none() {
            return out;
        }
        let mut v = String::new();
        if it.peek() == Some(&'=') {
            it.next();
            if it.peek() == Some(&'"') {
                it.next();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => v.push('\n'),
                            Some('r') => v.push('\r'),
                            Some('t') => v.push('\t'),
                            Some(x) => v.push(x),
                            None => break,
                        },
                        c => v.push(c),
                    }
                }
            } else {
                while let Some(&c) = it.peek() {
                    if c == ' ' {
                        break;
                    }
                    v.push(c);
                    it.next();
                }
            }
        }
        if !k.is_empty() {
            out.push((k, v));
        }
    }
}

pub fn push_kv(out: &mut String, k: &str, v: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(k);
    out.push('=');
    out.push_str(&quote(v));
}

pub fn kvs(fields: &[(String, String)]) -> String {
    let mut s = String::new();
    for (k, v) in fields {
        push_kv(&mut s, k, v);
    }
    s
}

pub fn ts_now() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head() -> Head {
        Head { node: "n1".into(), proc_name: "sfu" }
    }

    #[test]
    fn order_and_quote() {
        let l = Line::ev("CP-OP-BIND").key("user", "u1").key("session", "abcd1234").rec("from", "-").msg("bound ok");
        let s = l.render(&head(), "T", 5, 1024);
        assert_eq!(s, "ts=T mono=5 node=n1 proc=sfu kind=ev spec=CP-OP-BIND session=abcd1234 user=u1 from=- msg=\"bound ok\"");
    }

    #[test]
    fn empty_key_value_is_absent() {
        let mut l = Line::ev("CP-OP-BIND").key("session", "").key("user", "u1");
        l.set_key_if_absent("room", "");
        let s = l.render(&head(), "T", 5, 1024);
        assert_eq!(s, "ts=T mono=5 node=n1 proc=sfu kind=ev spec=CP-OP-BIND user=u1");
    }

    #[test]
    fn fields_reads_back_what_render_wrote() {
        let l = Line::ev("CP-OP-BIND").key("user", "u1").rec("from", "-").msg("say \"user=u2\" a\\b\nx");
        let s = l.render(&head(), "T", 5, 1024);
        let f = fields(&s);
        let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        assert_eq!(get("user"), Some("u1"));
        assert_eq!(get("msg"), Some("say \"user=u2\" a\\b\nx"));
        assert_eq!(f.iter().filter(|(k, _)| k == "user").count(), 1);
        assert_eq!(fields("a=1  b=\"\" c"), vec![("a".into(), "1".into()), ("b".into(), String::new()), ("c".into(), String::new())]);
    }

    #[test]
    fn diag_keeps_spec_stat_drops_it_and_msg_truncates() {
        let mut l = Line::new(Kind::Diag).msg("x".repeat(500));
        l.spec = Some("SV-RULE-SIM-TICK".into());
        let s = l.render(&head(), "T", 1, 256);
        assert!(s.len() <= 256);
        assert!(s.contains("truncated=1"));
        assert!(s.starts_with("ts=T mono=1 node=n1 proc=sfu kind=diag spec=SV-RULE-SIM-TICK "), "O-sym-RULE-LOG-SPEC-2① 진단 줄의 근거");
        let mut st = Line::new(Kind::Stat).rec("peers", 1);
        st.spec = Some("X".into());
        assert!(!st.render(&head(), "T", 1, 256).contains("spec="));
    }
}
