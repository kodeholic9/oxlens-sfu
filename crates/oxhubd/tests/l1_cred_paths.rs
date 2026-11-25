// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:COND-TRN-CORS-HEADER-AUTH · 연:RULE-TRN-HTTP-AUTH-TWO · model: claude-opus-5-5
//! 1층 소스 가드 — 게이트웨이 HTTP 의 자격 전달 경로는 `Authorization` · `x-oxlens-session` 둘뿐이다: 읽는 요청 헤더 · 질의 키 집합에 쿠키 · 토큰 질의가 없다 · CORS 허용 헤더가 그 둘과 `content-type` 이다(쿠키 인증이 들어오면 이 시험이 먼저 깨진다 — `*` 를 못 쓴다)(context 202611/20261124b 서버 규칙 갈래).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

fn args_of(line: &str, call: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find(call) {
        let after = &rest[i + call.len()..];
        if let Some(j) = after.find(')') {
            out.push(after[..j].trim().to_string());
        }
        rest = &rest[i + call.len()..];
    }
    out
}

#[test]
fn credentials_travel_only_in_authorization_and_the_session_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut heads = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut allow = Vec::new();
    let mut lines = 0;
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        for l in text.split("#[cfg(test)]").next().unwrap_or("").lines() {
            lines += 1;
            heads.extend(args_of(l, "headers.get("));
            keys.extend(args_of(l, "q.get(").into_iter().filter(|k| k != "k"));
            for c in ["text(\"", "num(\"", "flag(\""] {
                keys.extend(args_of(l, c).into_iter().map(|k| format!("\"{k}")));
            }
            if l.contains("ACCESS_CONTROL_ALLOW_HEADERS") {
                allow.push(l.trim().to_string());
            }
        }
    }
    assert!(lines > 3_000, "stand check — the hub sources were read ({lines})");
    let want_h: BTreeSet<String> = ["header::ORIGIN", "header::HOST", "header::AUTHORIZATION", "\"x-oxlens-session\"", "header::RANGE"].into_iter().map(String::from).collect();
    assert_eq!(heads, want_h, "C-cond-COND-TRN-CORS-HEADER-AUTH-1① the hub reads credentials only from Authorization and x-oxlens-session — no cookie");
    let want_q: BTreeSet<String> = ["\"tracks\"", "\"user\"", "\"all\"", "\"if_version\"", "\"if_epoch\"", "\"confirm\"", "\"room\"", "\"session\"", "\"since\"", "\"until\"", "\"diag\"", "\"stat\""].into_iter().map(String::from).collect();
    assert_eq!(keys, want_q, "C-cond-COND-TRN-CORS-HEADER-AUTH-1① no credential travels in a query key");
    assert_eq!(allow.len(), 1, "stand check — one CORS allow-headers line: {allow:?}");
    assert!(allow[0].contains("\"authorization, content-type, x-oxlens-session\""), "C-cond-COND-TRN-CORS-HEADER-AUTH-1① CORS allows exactly the two credential headers and content-type: {}", allow[0]);
}
