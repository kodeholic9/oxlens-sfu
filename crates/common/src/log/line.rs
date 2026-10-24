// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-OBSERVE · 정:RULE-OPS-LOGGER · 운영 규격서 §7-3 · 운영 규격서 §7-5 · model: claude-opus-5-5
//! 줄 형식 — logfmt 한 줄 = 레코드 하나. 머리(ts mono node proc kind spec) → 상관 키(표 차례) → 그 밖 → msg 끝.

pub const CORRELATION: &[&str] = &["session", "room", "user", "epoch", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq", "arev"];
pub const KINDS: &[&str] = &["ev", "stat", "diag"];

pub fn quote(v: &str) -> String {
    if !v.is_empty() && !v.contains([' ', '"', '=', '\n', '\t']) {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
}

#[derive(Debug, Clone, Default)]
pub struct Record {
    pub kind: Option<String>,
    pub spec: Option<String>,
    pub msg: Option<String>,
    pub fields: Vec<(String, String)>,
}

impl Record {
    pub fn set(&mut self, k: &str, v: String) {
        match k {
            "kind" => self.kind = Some(v),
            "spec" => self.spec = Some(v),
            "message" | "msg" => self.msg = Some(v),
            _ => {
                if let Some(slot) = self.fields.iter_mut().find(|(n, _)| n == k) {
                    slot.1 = v;
                } else {
                    self.fields.push((k.to_string(), v));
                }
            }
        }
    }
}

pub fn render(ts: &str, mono_us: u64, node: &str, proc_: &str, r: &Record, max: usize) -> String {
    let kind = r.kind.as_deref().filter(|k| KINDS.contains(k)).unwrap_or("diag");
    let mut out = format!("ts={ts} mono={mono_us} node={} proc={proc_} kind={kind}", quote(node));
    if let (true, Some(s)) = (kind == "ev", &r.spec) {
        out.push_str(" spec=");
        out.push_str(&quote(s));
    }
    for key in CORRELATION {
        if let Some((_, v)) = r.fields.iter().find(|(k, _)| k == key) {
            out.push(' ');
            out.push_str(key);
            out.push('=');
            out.push_str(&quote(v));
        }
    }
    for (k, v) in &r.fields {
        if CORRELATION.contains(&k.as_str()) || v.is_empty() {
            continue;
        }
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(&quote(v));
    }
    let Some(msg) = r.msg.as_deref().filter(|m| !m.is_empty()) else {
        return clip(out, max, false);
    };
    let tail = format!(" msg={}", quote(msg));
    if out.len() + tail.len() <= max {
        out.push_str(&tail);
        return out;
    }
    let room = max.saturating_sub(out.len() + " truncated=1 msg=\"\"".len());
    let mut cut: String = String::new();
    for c in msg.chars() {
        if cut.len() + c.len_utf8() > room {
            break;
        }
        cut.push(c);
    }
    out.push_str(" truncated=1 msg=");
    out.push_str(&quote(&cut));
    clip(out, max, true)
}

fn clip(mut s: String, max: usize, already: bool) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max.saturating_sub(if already { 0 } else { " truncated=1".len() });
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    if !already {
        s.push_str(" truncated=1");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_quoting() {
        let mut r = Record::default();
        r.set("kind", "ev".into());
        r.set("spec", "CP-OP-ROOM-JOIN".into());
        r.set("ssrc", "3735928559".into());
        r.set("user", "u7".into());
        r.set("x", "a b".into());
        r.set("message", "hi there".into());
        let l = render("T", 5, "n1", "sfu", &r, 1024);
        assert_eq!(l, "ts=T mono=5 node=n1 proc=sfu kind=ev spec=CP-OP-ROOM-JOIN user=u7 ssrc=3735928559 x=\"a b\" msg=\"hi there\"");
    }

    #[test]
    fn diag_has_no_spec_and_long_msg_is_cut() {
        let mut r = Record::default();
        r.set("spec", "X".into());
        r.set("message", "y".repeat(5000));
        let l = render("T", 1, "n", "hub", &r, 300);
        assert!(l.len() <= 300);
        assert!(l.contains("truncated=1"));
        assert!(!l.contains("spec="));
    }
}
