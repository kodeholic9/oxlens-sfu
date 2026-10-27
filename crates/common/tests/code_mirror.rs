// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-DOC-CODE-MIRROR · 정:CAND-DOC-GUARD-PLACE · 정:CAND-DOC-ORCHESTRATION-ASPECT · model: claude-opus-5-5
//! 구조 가드 — 코드 배치는 규격 영역의 거울이다. `server/out/deps.json` 을 읽어 셋을 센다(영역 없는 최상위 모듈 · 디렉터리 없는 개념 영역 · 개념 영역의 `may_use` 밖 `crate::` 호출).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const LIB_CRATES: &[&str] = &["common", "oxsig", "oxadmin"];
const ROOT_FILES: &[&str] = &["main", "lib"];

fn spec_root() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

struct Deps {
    concepts: BTreeSet<String>,
    known: BTreeSet<String>,
    may: BTreeMap<String, BTreeSet<String>>,
    raw: serde_json::Value,
}

fn lower_set(v: &serde_json::Value) -> BTreeSet<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.to_lowercase()).collect()).unwrap_or_default()
}

fn deps() -> Deps {
    let path = spec_root().join("server/out/deps.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("규격 위치를 못 읽었다 {}: {e} — OXLENS_SPEC 를 주거나 저장소 기준 ../../context/spec", path.display()));
    let raw: serde_json::Value = serde_json::from_str(&text).expect("deps.json");
    let concepts = lower_set(&raw["concepts"]);
    let mut known = concepts.clone();
    known.extend(lower_set(&raw["aspects"]));
    let mut may = BTreeMap::new();
    if let Some(areas) = raw["areas"].as_object() {
        for (a, v) in areas {
            may.insert(a.to_lowercase(), lower_set(&v["may_use"]));
        }
    }
    Deps { concepts, known, may, raw }
}

fn strip(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                if i < b.len() && b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 1;
            out.push_str("\"\"");
        } else {
            let c = src[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn seg(s: &str) -> String {
    s.trim().trim_start_matches("r#").to_string()
}

fn use_tree(t: &str) -> Vec<Vec<String>> {
    fn walk(s: &str, pre: &[String], out: &mut Vec<Vec<String>>) {
        let s = s.trim();
        match s.find('{') {
            None => {
                let s = s.split(" as ").next().unwrap_or(s);
                let mut p = pre.to_vec();
                p.extend(s.split("::").map(str::trim).filter(|x| !x.is_empty()).map(seg));
                if !p.is_empty() {
                    out.push(p);
                }
            }
            Some(i) => {
                let mut p = pre.to_vec();
                p.extend(s[..i].split("::").map(str::trim).filter(|x| !x.is_empty()).map(seg));
                let end = s.rfind('}').unwrap_or(s.len());
                let body = &s[i + 1..end];
                let mut depth = 0;
                let mut cur = String::new();
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
                            walk(&cur, &p, out);
                            cur.clear();
                        }
                        _ => cur.push(ch),
                    }
                }
                if !cur.trim().is_empty() {
                    walk(&cur, &p, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(t, &[], &mut out);
    out
}

fn module_of(src_root: &Path, f: &Path) -> Vec<String> {
    let rel = f.strip_prefix(src_root).unwrap_or(f);
    let mut parts: Vec<String> = rel.iter().map(|x| x.to_string_lossy().trim_end_matches(".rs").to_string()).collect();
    if parts.last().is_some_and(|l| l == "mod") {
        parts.pop();
    }
    if parts.len() == 1 && ROOT_FILES.contains(&parts[0].as_str()) {
        parts.clear();
    }
    parts
}

fn resolve(module: &[String], path: &[String]) -> Option<Vec<String>> {
    let first = path.first()?;
    match first.as_str() {
        "crate" => Some(path[1..].to_vec()),
        "self" => {
            let mut m = module.to_vec();
            m.extend_from_slice(&path[1..]);
            Some(m)
        }
        "super" => {
            let mut m = module.to_vec();
            let mut rest = path;
            while rest.first().is_some_and(|s| s == "super") {
                m.pop()?;
                rest = &rest[1..];
            }
            m.extend_from_slice(rest);
            Some(m)
        }
        _ => None,
    }
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

struct Count {
    stray: Vec<String>,
    missing: Vec<String>,
    outside: Vec<String>,
}

fn paths_in(text: &str) -> Vec<Vec<String>> {
    let bytes = text.as_bytes();
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find("use ") {
        let at = from + i;
        from = at + 4;
        let ok_before = at == 0 || (!bytes[at - 1].is_ascii_alphanumeric() && bytes[at - 1] != b'_');
        let tail = &text[at + 4..];
        if ok_before
            && (tail.starts_with("crate") || tail.starts_with("super") || tail.starts_with("self"))
            && let Some(end) = tail.find(';')
        {
            paths.extend(use_tree(&tail[..end]));
            spans.push((at, at + 4 + end));
        }
    }
    for token in ["crate::", "super::", "self::"] {
        let mut from = 0;
        while let Some(i) = text[from..].find(token) {
            let at = from + i;
            from = at + token.len();
            if spans.iter().any(|(a, b)| *a <= at && at < *b) {
                continue;
            }
            let before = if at == 0 { b' ' } else { bytes[at - 1] };
            if before.is_ascii_alphanumeric() || before == b'_' || before == b':' {
                continue;
            }
            let s: String = text[at..].chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':' || *c == '#').collect();
            paths.push(s.split("::").filter(|x| !x.is_empty()).map(seg).collect());
        }
    }
    paths
}

fn count(d: &Deps) -> Count {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut stray = Vec::new();
    let mut outside = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates).expect("crates").flatten().map(|e| e.path()).collect();
    dirs.sort();
    for c in dirs {
        let name = c.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let src = c.join("src");
        if !src.join("main.rs").exists() || LIB_CRATES.contains(&name.as_str()) {
            continue;
        }
        for e in std::fs::read_dir(&src).expect("src").flatten() {
            let p = e.path();
            let stem = if p.is_dir() { p.file_name() } else { p.file_stem() }.map(|x| x.to_string_lossy().to_string());
            let Some(stem) = stem else { continue };
            if p.is_file() && (p.extension().is_none_or(|x| x != "rs") || ROOT_FILES.contains(&stem.as_str())) {
                continue;
            }
            seen.insert(stem.clone());
            if !d.known.contains(&stem) {
                stray.push(format!("{name}/src/{stem}"));
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
            let empty = BTreeSet::new();
            let may = d.may.get(&me).unwrap_or(&empty);
            for p in paths_in(&text) {
                let Some(r) = resolve(&module, &p) else { continue };
                let Some(target) = r.first() else { continue };
                if target == &me || !d.concepts.contains(target) {
                    continue;
                }
                if !may.contains(target) {
                    outside.push(format!("{} {} → {} ({})", f.display(), me.to_uppercase(), target.to_uppercase(), p.join("::")));
                }
            }
        }
    }
    let missing = d.concepts.iter().filter(|c| !seen.contains(*c)).map(|c| c.to_uppercase()).collect();
    outside.sort();
    outside.dedup();
    Count { stray, missing, outside }
}

#[test]
fn code_mirrors_spec_areas() {
    let d = deps();
    let c = count(&d);
    assert!(c.stray.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 영역 없는 최상위 모듈 {:?}", c.stray);
    assert!(c.missing.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 디렉터리 없는 개념 영역 {:?}", c.missing);
    assert!(c.outside.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 may_use 밖 호출 {:#?}", c.outside);
}

#[test]
fn may_use_is_relation_union_seam() {
    let d = deps();
    let areas = d.raw["areas"].as_object().expect("areas");
    for (a, v) in areas {
        let mut want = lower_set(&v["relation"]);
        want.extend(lower_set(&v["seam"]));
        assert_eq!(lower_set(&v["may_use"]), want, "S-sym-RULE-DOC-CODE-MIRROR-2 {a} may_use = 관계 직접 ∪ 이음매 직접");
    }
}

#[test]
fn guard_sees_nested_use_trees_super_and_raw_idents() {
    let t = use_tree("crate::a::{b, c::{d, e as f}}");
    assert_eq!(t, vec![vec!["crate", "a", "b"], vec!["crate", "a", "c", "d"], vec!["crate", "a", "c", "e"]]);
    let m: Vec<String> = vec!["flr".into(), "core".into()];
    assert_eq!(resolve(&m, &["super".to_string(), "super".to_string(), "room".to_string()]), Some(vec!["room".to_string()]));
    assert!(!strip("let a = \"crate::x\"; // crate::y").contains("crate"));
    let p = paths_in("use crate::room::{core::Room, shell};\nfn f() { crate::flr::x(); crate::r#pub::Stream::new(); }");
    assert!(p.contains(&vec!["crate".to_string(), "flr".to_string(), "x".to_string()]));
    assert!(p.contains(&vec!["crate".to_string(), "room".to_string(), "core".to_string(), "Room".to_string()]));
    assert!(p.contains(&vec!["crate".to_string(), "pub".to_string(), "Stream".to_string(), "new".to_string()]));
    assert_eq!(use_tree("crate::r#pub::{Stream}"), vec![vec!["crate", "pub", "Stream"]]);
}
