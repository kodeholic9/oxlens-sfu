// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:RULE-LOG-LINE 운영:RULE-LOG-KEYS 운영:RULE-LOG-KIND 운영:RULE-LOG-TRANSITION 운영:RULE-LOG-SENDER 운영:LOG.log.LINE 운영:LOG.log.TRANSITION · model: claude-opus-5-5
//! 줄 하나 조립 — 칸 차례 고정(머리 → 상관 키 → 레코드 값 → `msg`) · 없는 키 생략 · 줄 상한 넘으면 `msg` 자르고 `truncated=1`.

use std::fmt::Write as _;

pub const HEAD: &[&str] = &["ts", "mono", "node", "proc", "kind", "spec"];
pub const CORR: &[&str] = &["session", "room", "user", "epoch", "track", "ssrc", "seq", "floor_sequence", "mbcp_seq", "arev"];
pub const REST_LINE: &[&str] = &[
    "build",
    "truncated",
    "spooled",
    "dropped",
    "packets_received",
    "packets_lost",
    "jitter",
    "bytes_received",
    "packets_sent",
    "bytes_sent",
    "retransmitted_packets_sent",
    "nack_count",
    "pli_count",
    "rid",
];
pub const REST_TRANSITION: &[&str] = &["area", "key", "from", "to", "cause", "impossible"];

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

#[derive(Debug, Default, Clone)]
pub struct Record {
    pub fields: Vec<(String, String)>,
    pub ins: Vec<(String, String)>,
    pub msg: String,
}

impl Record {
    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        if let Some(slot) = self.fields.iter_mut().find(|(n, _)| n == k) {
            slot.1 = v;
        } else {
            self.fields.push((k.to_string(), v));
        }
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    fn take(&self, k: &str) -> Option<&str> {
        self.get(k).filter(|v| !v.is_empty())
    }
}

pub fn quote(v: &str) -> String {
    let needs = v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '"' || c == '=' || c.is_control());
    if !needs {
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
            c if c.is_control() => {
                let _ = write!(s, "\\u{{{:x}}}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn push(out: &mut String, k: &str, v: &str) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(k);
    out.push('=');
    out.push_str(&quote(v));
}

pub fn render(r: &Record, max: usize) -> String {
    let transition = r.take("area").is_some();
    let rest: &[&str] = if transition { REST_TRANSITION } else { REST_LINE };
    let known: Vec<&str> = HEAD.iter().chain(CORR).chain(REST_LINE).chain(REST_TRANSITION).copied().collect();
    let mut extra = String::new();
    for (k, v) in &r.fields {
        if !known.contains(&k.as_str()) && !v.is_empty() {
            if !extra.is_empty() {
                extra.push(' ');
            }
            extra.push_str(k);
            extra.push('=');
            extra.push_str(v);
        }
    }
    let msg = match (r.msg.is_empty(), extra.is_empty()) {
        (true, true) => String::new(),
        (false, true) => r.msg.clone(),
        (true, false) => extra,
        (false, false) => format!("{} {}", r.msg, extra),
    };
    let mut head = String::with_capacity(256);
    for k in HEAD.iter().chain(CORR).chain(rest) {
        if *k == "truncated" {
            continue;
        }
        if let Some(v) = r.take(k) {
            push(&mut head, k, v);
        }
    }
    if transition {
        for (k, v) in &r.ins {
            push(&mut head, &format!("in.{k}"), v);
        }
    }
    let full = if msg.is_empty() { head.clone() } else { format!("{head} msg={}", quote(&msg)) };
    if full.len() <= max {
        return full;
    }
    let budget = max.saturating_sub(head.len() + " truncated=1 msg=\"\"".len() + 8);
    let mut cut = msg.len().min(budget);
    while cut > 0 && !msg.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = head;
    push(&mut out, "truncated", "1");
    let q = quote(&msg[..cut]);
    out.push_str(" msg=");
    out.push_str(&q);
    if out.len() > max {
        let mut c = max;
        while c > 0 && !out.is_char_boundary(c) {
            c -= 1;
        }
        out.truncate(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pairs: &[(&str, &str)], msg: &str) -> Record {
        let mut r = Record { msg: msg.into(), ..Default::default() };
        for (k, v) in pairs {
            r.set(k, *v);
        }
        r
    }

    #[test]
    fn order_is_fixed_and_absent_keys_skipped() {
        let r = rec(
            &[("user", "u1"), ("kind", "ev"), ("node", "n"), ("proc", "hub"), ("ts", "T"), ("mono", "5"), ("spec", "CP-OP-BIND"), ("room", "")],
            "bound ok",
        );
        assert_eq!(render(&r, 1024), "ts=T mono=5 node=n proc=hub kind=ev spec=CP-OP-BIND user=u1 msg=\"bound ok\"");
    }

    #[test]
    fn unknown_keys_fold_into_msg() {
        let r = rec(&[("ts", "T"), ("mono", "1"), ("node", "n"), ("proc", "sfu"), ("kind", "diag"), ("why", "x")], "m");
        assert_eq!(render(&r, 1024), "ts=T mono=1 node=n proc=sfu kind=diag msg=\"m why=x\"");
    }

    #[test]
    fn transition_line_order() {
        let mut r = rec(
            &[
                ("ts", "T"),
                ("mono", "1"),
                ("node", "n"),
                ("proc", "sfu"),
                ("kind", "ev"),
                ("spec", "FLR.S.IDLE.req"),
                ("room", "r1"),
                ("area", "FLR"),
                ("key", "r1"),
                ("from", "IDLE"),
                ("to", "TAKEN"),
                ("cause", "REQUEST"),
            ],
            "",
        );
        r.ins.push(("priority".into(), "0".into()));
        assert_eq!(
            render(&r, 1024),
            "ts=T mono=1 node=n proc=sfu kind=ev spec=FLR.S.IDLE.req room=r1 area=FLR key=r1 from=IDLE to=TAKEN cause=REQUEST in.priority=0"
        );
    }

    #[test]
    fn long_msg_truncated_with_mark() {
        let r = rec(&[("ts", "T"), ("mono", "1"), ("node", "n"), ("proc", "hub"), ("kind", "diag")], &"가".repeat(800));
        let s = render(&r, 300);
        assert!(s.len() <= 300);
        assert!(s.contains("truncated=1 msg="));
    }
}
