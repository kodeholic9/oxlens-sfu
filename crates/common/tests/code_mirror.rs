// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-DOC-CODE-MIRROR · 정:CAND-DOC-GUARD-PLACE · 정:CAND-DOC-ORCHESTRATION-ASPECT · model: claude-opus-5-5
//! 구조 가드 — 프로세스 크레이트의 최상위 모듈 ↔ 규격 영역 · 디렉터리 없는 개념 영역 · 개념 영역의 `may_use` 밖 호출을 센다. 규격을 못 읽으면 빨강.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const PROCESS_CRATES: &[&str] = &["oxhubd", "oxsfud", "oxlogd"];
const ROOT_FILES: &[&str] = &["main", "lib"];

struct Deps {
    concepts: BTreeSet<String>,
    known: BTreeSet<String>,
    may: BTreeMap<String, BTreeSet<String>>,
    raw: serde_json::Value,
}

fn deps() -> Deps {
    let p = common::spec_root().join("server/out/deps.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("규격을 못 읽었다 {}: {e}", p.display()));
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("deps.json: {e}"));
    let list = |k: &str| -> BTreeSet<String> {
        raw[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_lowercase).collect()).unwrap_or_default()
    };
    let concepts = list("concepts");
    let known: BTreeSet<String> = concepts.union(&list("aspects")).cloned().collect();
    let mut may = BTreeMap::new();
    if let Some(areas) = raw["areas"].as_object() {
        for (a, v) in areas {
            let set = v["may_use"].as_array().map(|x| x.iter().filter_map(|y| y.as_str()).map(str::to_lowercase).collect()).unwrap_or_default();
            may.insert(a.to_lowercase(), set);
        }
    }
    assert!(!concepts.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 deps.json 에 개념 영역이 없다");
    Deps { concepts, known, may, raw }
}

fn strip(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if b[i] == '/' && i + 1 < b.len() && b[i + 1] == '*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if b[i] == '"' {
            out.push('"');
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                if i < b.len() && b[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            out.push('"');
            i += 1;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn use_tree(t: &str, pre: &[String], out: &mut Vec<Vec<String>>) {
    let s = t.trim();
    let Some(open) = s.find('{') else {
        let s = s.split(" as ").next().unwrap_or(s);
        let mut path = pre.to_vec();
        path.extend(s.split("::").map(str::trim).filter(|p| !p.is_empty()).map(str::to_string));
        if !path.is_empty() {
            out.push(path);
        }
        return;
    };
    let mut head = pre.to_vec();
    head.extend(s[..open].split("::").map(str::trim).filter(|p| !p.is_empty()).map(str::to_string));
    let close = s.rfind('}').unwrap_or(s.len());
    let body = &s[open + 1..close];
    let (mut depth, mut cur) = (0i32, String::new());
    for ch in body.chars() {
        match ch {
            '{' => {
                depth += 1;
                cur.push(ch);
            }
            '}' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                use_tree(&cur, &head, out);
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        use_tree(&cur, &head, out);
    }
}

fn resolve(module: &[String], path: &[String]) -> Option<Vec<String>> {
    let first = path.first()?;
    if first == "crate" {
        return Some(path[1..].to_vec());
    }
    if first == "super" || first == "self" {
        let mut cur = module.to_vec();
        let mut rest = path;
        while let Some(p) = rest.first() {
            if p == "super" {
                cur.pop()?;
            } else if p != "self" {
                break;
            }
            rest = &rest[1..];
        }
        cur.extend(rest.iter().cloned());
        return Some(cur);
    }
    None
}

fn module_of(src: &Path, f: &Path) -> Vec<String> {
    let rel = f.strip_prefix(src).unwrap_or(f).with_extension("");
    let mut parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().to_string()).collect();
    if parts.last().map(|l| l == "mod").unwrap_or(false) || (parts.len() == 1 && ROOT_FILES.contains(&parts[0].as_str())) {
        parts.pop();
    }
    parts
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn paths_in(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut found = Vec::new();
    let mut spans = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find("use ") {
        let at = i + off;
        let before_ok = at == 0 || !text.as_bytes()[at - 1].is_ascii_alphanumeric() && text.as_bytes()[at - 1] != b'_';
        let rest = &text[at + 4..];
        let starts = ["crate", "super", "self"].iter().any(|k| rest.trim_start().starts_with(k));
        if before_ok && starts && let Some(end) = rest.find(';') {
            let line = text[..at].matches('\n').count() + 1;
            let mut v = Vec::new();
            use_tree(&rest[..end], &[], &mut v);
            found.extend(v.into_iter().map(|p| (line, p)));
            spans.push((at, at + 4 + end));
            i = at + 4 + end;
            continue;
        }
        i = at + 4;
    }
    let bytes = text.as_bytes();
    for kw in ["crate::", "super::", "self::"] {
        let mut j = 0;
        while let Some(off) = text[j..].find(kw) {
            let at = j + off;
            j = at + kw.len();
            let prev = if at == 0 { b' ' } else { bytes[at - 1] };
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b':' {
                continue;
            }
            if spans.iter().any(|(a, b)| *a <= at && at < *b) {
                continue;
            }
            let mut k = at;
            while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_' || bytes[k] == b':') {
                k += 1;
            }
            let path: Vec<String> = text[at..k].split("::").filter(|p| !p.is_empty()).map(str::to_string).collect();
            if path.len() >= 2 {
                found.push((text[..at].matches('\n').count() + 1, path));
            }
        }
    }
    found
}

#[test]
fn mirror_has_no_breaks() {
    let d = deps();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (mut stray, mut outside, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
    let mut any = false;
    for c in PROCESS_CRATES {
        let src = root.join("crates").join(c).join("src");
        if !src.join("main.rs").exists() {
            continue;
        }
        any = true;
        for e in std::fs::read_dir(&src).into_iter().flatten().flatten() {
            let p = e.path();
            let name = if p.is_dir() { p.file_name().map(|n| n.to_string_lossy().to_string()) } else { p.file_stem().map(|n| n.to_string_lossy().to_string()) };
            let Some(name) = name else { continue };
            if p.is_file() && (p.extension().map(|x| x != "rs").unwrap_or(true) || ROOT_FILES.contains(&name.as_str())) {
                continue;
            }
            seen.insert(name.clone());
            if !d.known.contains(&name) {
                stray.push(format!("{c}/src/{name}"));
            }
        }
        let mut files = Vec::new();
        rs_files(&src, &mut files);
        for f in files {
            let module = module_of(&src, &f);
            let Some(me) = module.first().cloned() else { continue };
            if !d.concepts.contains(&me) {
                continue;
            }
            let text = strip(&std::fs::read_to_string(&f).unwrap_or_default());
            for (line, p) in paths_in(&text) {
                let Some(r) = resolve(&module, &p) else { continue };
                let Some(target) = r.first() else { continue };
                if target == &me || !d.concepts.contains(target) {
                    continue;
                }
                if !d.may.get(&me).map(|s| s.contains(target)).unwrap_or(false) {
                    outside.push(format!("{}:{line} {} → {}", f.display(), me.to_uppercase(), target.to_uppercase()));
                }
            }
        }
    }
    assert!(any, "S-sym-RULE-DOC-CODE-MIRROR-1 프로세스 크레이트가 하나도 없다");
    let missing: Vec<String> = if PROCESS_CRATES.iter().all(|c| root.join("crates").join(c).join("src/main.rs").exists()) {
        d.concepts.iter().filter(|c| !seen.contains(*c)).map(|c| c.to_uppercase()).collect()
    } else {
        Vec::new()
    };
    assert!(stray.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 영역 없는 최상위 모듈 {stray:?}");
    assert!(missing.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 디렉터리 없는 개념 영역 {missing:?}");
    assert!(outside.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 may_use 밖 호출 {outside:?}");
}

#[test]
fn may_use_is_direct_relation_and_seam_only() {
    let d = deps();
    let areas = d.raw["areas"].as_object().unwrap_or_else(|| panic!("deps.json areas 없음"));
    for (a, v) in areas {
        let set = |k: &str| -> BTreeSet<String> { v[k].as_array().map(|x| x.iter().filter_map(|y| y.as_str()).map(str::to_string).collect()).unwrap_or_default() };
        let direct: BTreeSet<String> = set("relation").union(&set("seam")).cloned().collect();
        assert_eq!(set("may_use"), direct, "S-sym-RULE-DOC-CODE-MIRROR-2 {a} 의 may_use 는 관계 직접 ∪ 이음매 직접");
    }
}

#[test]
fn use_tree_flattens_nested_braces() {
    let mut v = Vec::new();
    use_tree("crate::a::{b, c::{d, e as f}}", &[], &mut v);
    assert_eq!(v, vec![vec!["crate", "a", "b"], vec!["crate", "a", "c", "d"], vec!["crate", "a", "c", "e"]].into_iter().map(|x| x.into_iter().map(String::from).collect::<Vec<_>>()).collect::<Vec<_>>());
    assert_eq!(resolve(&["sub".into(), "x".into()], &["super".into(), "super".into(), "flr".into()]), Some(vec!["flr".to_string()]));
}
