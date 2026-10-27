// author: kodeholic (powered by Claude)
// spec: v1.4 · 운영:RULE-LOG-LINE · 운영:RULE-LOG-KEYS · 운영:RULE-LOG-KIND · 운영:RULE-LOG-TRANSITION · 운영:LOG.log.LINE · 운영:LOG.log.TRANSITION · 연:DEV-TRN-SESSION-SECRET · model: claude-opus-5-5
//! 줄 하나 = 레코드 하나 = datagram 하나. 칸 차례 고정 — 머리(ts · mono · node · proc · kind · spec) → 상관 키 → 레코드별 값 → `msg`.

use sha2::{Digest, Sha256};

pub const CORRELATION: &[&str] = &["session", "room", "user", "epoch", "unit", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq", "arev"];

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
}

#[derive(Debug, Clone, Default)]
pub struct Head {
    pub node: String,
    pub proc_: String,
    pub epoch: Option<String>,
}

pub fn quote(v: &str) -> String {
    if !v.is_empty() && !v.chars().any(|c| c.is_whitespace() || c == '"' || c == '=') {
        return v.to_string();
    }
    let mut s = String::with_capacity(v.len() + 2);
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

pub fn fingerprint(session_id: &str) -> String {
    let d = Sha256::digest(session_id.as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

pub struct Record<'a> {
    pub ts: &'a str,
    pub mono: u64,
    pub kind: Kind,
    pub fields: &'a [(String, String)],
    pub msg: Option<&'a str>,
}

pub fn render(head: &Head, r: &Record<'_>, line_max: usize) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("ts=");
    out.push_str(r.ts);
    out.push_str(" mono=");
    out.push_str(&r.mono.to_string());
    out.push_str(" node=");
    out.push_str(&quote(&head.node));
    out.push_str(" proc=");
    out.push_str(&head.proc_);
    out.push_str(" kind=");
    out.push_str(r.kind.as_str());
    if r.kind == Kind::Ev
        && let Some((_, v)) = r.fields.iter().find(|(k, _)| k == "spec")
    {
        out.push_str(" spec=");
        out.push_str(&quote(v));
    }
    for key in CORRELATION {
        let v = r.fields.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let v = match (v, *key) {
            (Some(v), _) => Some(v),
            (None, "epoch") => head.epoch.as_deref(),
            _ => None,
        };
        if let Some(v) = v
            && !v.is_empty()
        {
            out.push(' ');
            out.push_str(key);
            out.push('=');
            out.push_str(&quote(v));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for (k, v) in r.fields {
        if k == "spec" || k == "message" || CORRELATION.contains(&k.as_str()) || seen.contains(&k.as_str()) || v.is_empty() {
            continue;
        }
        seen.push(k);
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(&quote(v));
    }
    let Some(msg) = r.msg.filter(|m| !m.is_empty()) else {
        return out;
    };
    let tail = format!(" msg={}", quote(msg));
    if out.len() + tail.len() <= line_max {
        out.push_str(&tail);
        return out;
    }
    let mark = " truncated=1";
    let room = line_max.saturating_sub(out.len() + mark.len() + " msg=\"\"".len() + 8);
    let mut cut = String::new();
    for c in msg.chars() {
        if quote(&cut).len() + c.len_utf8() + 2 > room {
            break;
        }
        cut.push(c);
    }
    out.push_str(mark);
    out.push_str(" msg=");
    out.push_str(&quote(&cut));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn order_is_head_then_correlation_then_record_then_msg() {
        let h = Head { node: "node-a".into(), proc_: "sfu".into(), epoch: Some("e1".into()) };
        let fields = f(&[("from", "Idle"), ("room", "r1"), ("spec", "FLR.S.x"), ("session", "abcd1234"), ("area", "FLR")]);
        let r = Record { ts: "T", mono: 5, kind: Kind::Ev, fields: &fields, msg: Some("a b") };
        assert_eq!(render(&h, &r, 1024), "ts=T mono=5 node=node-a proc=sfu kind=ev spec=FLR.S.x session=abcd1234 room=r1 epoch=e1 from=Idle area=FLR msg=\"a b\"");
    }

    #[test]
    fn diag_has_no_spec_and_absent_keys_are_omitted() {
        let h = Head { node: "n".into(), proc_: "hub".into(), epoch: None };
        let fields = f(&[("spec", "X"), ("track", "")]);
        let r = Record { ts: "T", mono: 0, kind: Kind::Diag, fields: &fields, msg: None };
        assert_eq!(render(&h, &r, 1024), "ts=T mono=0 node=n proc=hub kind=diag");
    }

    #[test]
    fn long_msg_is_cut_and_marked() {
        let h = Head { node: "n".into(), proc_: "hub".into(), epoch: None };
        let long = "x".repeat(5000);
        let r = Record { ts: "T", mono: 0, kind: Kind::Diag, fields: &[], msg: Some(&long) };
        let s = render(&h, &r, 300);
        assert!(s.len() <= 300, "{}", s.len());
        assert!(s.contains(" truncated=1 msg="));
    }

    #[test]
    fn fingerprint_is_eight_hex() {
        let fp = fingerprint("secret-session");
        assert_eq!(fp.len(), 8);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(!fp.contains("secret"));
    }
}
