// author: kodeholic (powered by Claude)
// spec: v1.12 · 운영:RULE-LOG-SPEC · 운영:RULE-LOG-KIND · model: claude-opus-5-5
//! 진단 줄 가드 — 우리 코드가 내는 `kind=diag` 줄은 그 판단을 정한 진술을 `spec` 으로 단다(운영:RULE-LOG-SPEC 「진단 줄의 근거」) · `SV-` 라벨은 정의서 라벨 레지스트리(`server/out/labels.json`)에 있다 · `Line::new(Kind::Diag)` 대신 `Line::diag(spec)` (남의 크레이트 줄을 받는 `common/src/log` 는 밖).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn sv_labels() -> BTreeSet<String> {
    let p = spec_dir().join("server/out/labels.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("labels.json 없음 {}: {e}", p.display()));
    let v: serde_json::Value = serde_json::from_str(&text).expect("labels.json");
    v.as_array().map(|a| a.iter().filter_map(|x| x["label"].as_str()).map(|s| format!("SV-{s}")).collect()).unwrap_or_default()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
}

fn sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    for c in ["oxhubd", "oxsfud", "oxmesh", "oxsig", "common", "oxadmin"] {
        rs_files(&root.join(c).join("src"), &mut out);
    }
    out.retain(|p| !p.to_string_lossy().contains("common/src/log/"));
    out
}

fn macro_body(text: &str, open: usize) -> &str {
    let b = text.as_bytes();
    let (mut depth, mut i, mut in_str) = (0i32, open, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == b'\\' {
                i += 1;
            } else if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return &text[open + 1..i];
            }
        }
        i += 1;
    }
    &text[open..]
}

fn spec_value(body: &str) -> Option<String> {
    let at = body.find("spec = \"")? + "spec = \"".len();
    let end = body[at..].find('"')?;
    Some(body[at..at + end].to_string())
}

fn line_no(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

fn test_region(text: &str) -> usize {
    text.find("#[cfg(test)]").unwrap_or(text.len())
}

#[test]
fn diag_lines_carry_spec_from_registry() {
    let labels = sv_labels();
    assert!(labels.len() > 100, "정의서 라벨 레지스트리가 비었다({})", labels.len());
    let mut bad = Vec::new();
    let mut seen = 0usize;
    for p in sources() {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let stop = test_region(&text);
        let name = p.display().to_string();
        for mac in ["tracing::info!(", "tracing::warn!(", "tracing::error!(", "tracing::debug!(", "tracing::trace!("] {
            let mut from = 0;
            while let Some(k) = text[from..].find(mac) {
                let at = from + k;
                from = at + mac.len();
                if at >= stop {
                    break;
                }
                let body = macro_body(&text, at + mac.len() - 1);
                if !body.contains("kind = \"diag\"") {
                    continue;
                }
                seen += 1;
                if !body.contains("spec = ") {
                    bad.push(format!("{name}:{} 진단 줄에 spec 이 없다", line_no(&text, at)));
                    continue;
                }
                match spec_value(body) {
                    None => {}
                    Some(s) if s.starts_with("SV-") && !labels.contains(&s) => bad.push(format!("{name}:{} spec {s} 가 정의서 라벨 레지스트리에 없다", line_no(&text, at))),
                    Some(_) => {}
                }
            }
        }
        let mut from = 0;
        while let Some(k) = text[from..].find("Kind::Diag)") {
            let at = from + k;
            from = at + 1;
            if at >= stop {
                break;
            }
            if text[..at].ends_with("Line::new(") || text[..at].ends_with("Line::new(common::log::") {
                bad.push(format!("{name}:{} Line::new(Kind::Diag) — spec 없는 진단 줄 · Line::diag(spec) 를 쓴다", line_no(&text, at)));
            }
        }
    }
    assert!(seen > 20, "진단 줄을 거의 못 찾았다({seen}) — 가드가 공허하다");
    assert!(bad.is_empty(), "진단 줄 근거 {}건:\n{}", bad.len(), bad.join("\n"));
}
