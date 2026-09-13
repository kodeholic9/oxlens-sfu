// author: kodeholic (powered by Claude)
// spec: v1.1 · 운영 §6 · model: claude-opus-5

//! 사람이 읽는 표 — ★**기본 출력**이다(`--json` 은 자동화용 raw).
//!
//! ★★**모르는 것은 `-` 로 둔다**(운영 §6 금지 칸). 서버가 안 낸 값을 도구가 계산해 채우면
//! ★**서버가 모른다는 사실이 화면에서 사라진다** — 그것이 제일 비싼 거짓이다.

/// 빈 자리의 표기. ★**한 글자로 고정한다** — `0`·`false`·빈칸과 섞이면 뜻이 사라진다.
pub const UNKNOWN: &str = "-";

/// JSON 한 값을 칸 하나로. ★**없으면 `-`** 이고, 그것이 「서버가 안 냈다」의 표기다.
pub fn cell(v: Option<&serde_json::Value>) -> String {
    match v {
        None | Some(serde_json::Value::Null) => UNKNOWN.to_string(),
        Some(serde_json::Value::String(s)) if s.is_empty() => UNKNOWN.to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Bool(b)) => (if *b { "yes" } else { "no" }).to_string(),
        Some(serde_json::Value::Array(a)) if a.is_empty() => UNKNOWN.to_string(),
        Some(serde_json::Value::Array(a)) => {
            a.iter().map(|x| cell(Some(x))).collect::<Vec<_>>().join(",")
        }
        Some(x) => x.to_string(),
    }
}

/// 그 줄에서 열들을 뽑는다 — ★**키가 없으면 `-`**(빈 문자열이 아니다).
pub fn row(v: &serde_json::Value, keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| cell(v.get(*k))).collect()
}

/// 칸 너비를 맞춰 찍는다. ★**한글이 섞이면 폭이 안 맞는다** — 표시 폭으로 센다.
pub fn render(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut w: Vec<usize> = head.iter().map(|h| width(h)).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(width(c));
            }
        }
    }
    let mut out = String::new();
    push_line(&mut out, &w, &head.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    out.push_str(&w.iter().map(|n| "─".repeat(*n)).collect::<Vec<_>>().join("  "));
    out.push('\n');
    for r in rows {
        push_line(&mut out, &w, r);
    }
    if rows.is_empty() {
        out.push_str("(없다)\n");
    }
    out
}

fn push_line(out: &mut String, w: &[usize], cells: &[String]) {
    let last = cells.len().saturating_sub(1);
    for (i, c) in cells.iter().enumerate() {
        out.push_str(c);
        if i != last {
            out.push_str(&" ".repeat(w[i].saturating_sub(width(c)) + 2));
        }
    }
    out.push('\n');
}

/// ★**표시 폭** — 한글·기호는 두 칸이다. 바이트로 세면 표가 어긋난다.
fn width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let c = c as u32;
            let wide = (0x1100..=0x115F).contains(&c)
                || (0x2E80..=0xA4CF).contains(&c)
                || (0xAC00..=0xD7A3).contains(&c)
                || (0xF900..=0xFAFF).contains(&c)
                || (0xFF00..=0xFF60).contains(&c)
                || (0xFFE0..=0xFFE6).contains(&c);
            if wide { 2 } else { 1 }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 모르는_것은_대시다() {
        // ★★`0`·`false` 와 「서버가 안 냈다」가 같은 글자면 운영자가 못 가른다.
        let v = serde_json::json!({ "a": 0, "b": false, "c": null, "d": "" });
        assert_eq!(row(&v, &["a", "b", "c", "d", "e"]), vec!["0", "no", "-", "-", "-"]);
    }

    #[test]
    fn 배열은_이어_붙이되_빈_것은_대시다() {
        let v = serde_json::json!({ "x": ["tcp/a", "tcp/b"], "y": [] });
        assert_eq!(row(&v, &["x", "y"]), vec!["tcp/a,tcp/b", "-"]);
    }

    #[test]
    fn 한글은_두_칸으로_센다() {
        // ★바이트로 세면 한글 머리글이 든 표가 통째로 어긋난다.
        assert_eq!(width("방"), 2);
        assert_eq!(width("ab"), 2);
        let t = render(&["방", "수"], &[vec!["r1".into(), "3".into()]]);
        assert!(t.contains("r1"), "{t}");
    }

    #[test]
    fn 빈_표는_비었다고_말한다() {
        // ★머리글만 찍으면 「못 물어봤다」처럼 보인다.
        assert!(render(&["a"], &[]).contains("(없다)"));
    }
}
