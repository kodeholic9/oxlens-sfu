// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-DOC-CODE-MIRROR · model: claude-opus-5-5
//! 구조 가드 — 코드 배치가 규격 영역의 거울인가: 영역 없는 최상위 모듈 · 디렉터리 없는 영역 · 개념 영역의 `may_use` 밖 `crate::` 호출을 센다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

struct Deps {
    concepts: BTreeSet<String>,
    aspects: BTreeSet<String>,
    may_use: BTreeMap<String, BTreeSet<String>>,
}

fn load() -> Deps {
    let p = spec_dir().join("server/out/deps.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("deps.json 없음 {}: {e}", p.display()));
    let v: serde_json::Value = serde_json::from_str(&text).expect("deps.json");
    let set = |k: &str| -> BTreeSet<String> {
        v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.to_lowercase()).collect()).unwrap_or_default()
    };
    let mut may_use = BTreeMap::new();
    if let Some(areas) = v["areas"].as_object() {
        for (name, a) in areas {
            let m: BTreeSet<String> = a["may_use"]
                .as_array()
                .map(|x| x.iter().filter_map(|y| y.as_str()).map(|s| s.to_lowercase()).collect())
                .unwrap_or_default();
            may_use.insert(name.to_lowercase(), m);
        }
    }
    Deps { concepts: set("concepts"), aspects: set("aspects"), may_use }
}

fn process_crates() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    ["oxhubd", "oxsfud"].iter().map(|c| root.join(c).join("src")).filter(|p| p.is_dir()).collect()
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

fn crate_refs(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find("crate::") {
        rest = &rest[i + 7..];
        if let Some(inner) = rest.strip_prefix('{') {
            let mut depth = 1usize;
            let mut end = 0;
            for (j, ch) in inner.char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let body = &inner[..end];
            let mut d = 0usize;
            let mut start = 0usize;
            let mut items = Vec::new();
            for (j, ch) in body.char_indices() {
                match ch {
                    '{' => d += 1,
                    '}' => d = d.saturating_sub(1),
                    ',' if d == 0 => {
                        items.push(&body[start..j]);
                        start = j + 1;
                    }
                    _ => {}
                }
            }
            items.push(&body[start..]);
            for it in items {
                if let Some(n) = ident(it.trim()) {
                    out.insert(n);
                }
            }
        } else if let Some(n) = ident(rest) {
            out.insert(n);
        }
    }
    out
}

fn ident(s: &str) -> Option<String> {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let n: String = s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if n.is_empty() { None } else { Some(n) }
}

#[test]
fn code_mirror() {
    let deps = load();
    let crates = process_crates();
    let areas: BTreeSet<String> = deps.concepts.union(&deps.aspects).cloned().collect();
    let mut breaks: Vec<String> = Vec::new();
    let mut have_dir: BTreeSet<String> = BTreeSet::new();
    for src in &crates {
        let Ok(rd) = std::fs::read_dir(src) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if matches!(name.as_str(), "lib" | "main") {
                continue;
            }
            if p.is_dir() {
                have_dir.insert(name.clone());
            }
            if !areas.contains(&name) {
                breaks.push(format!("영역 없는 최상위 모듈 {}", p.display()));
            }
        }
        for c in &deps.concepts {
            let dir = src.join(c);
            if !dir.is_dir() {
                continue;
            }
            let allowed = deps.may_use.get(c).cloned().unwrap_or_default();
            let mut files = Vec::new();
            rs_files(&dir, &mut files);
            for f in files {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                for r in crate_refs(&text) {
                    if deps.concepts.contains(&r) && &r != c && !allowed.contains(&r) {
                        breaks.push(format!("{} → crate::{r} (may_use 밖 · {c})", f.display()));
                    }
                }
            }
        }
    }
    if crates.len() == 2 {
        for c in &deps.concepts {
            if !have_dir.contains(c) {
                breaks.push(format!("디렉터리 없는 영역 {c}"));
            }
        }
    }
    assert!(breaks.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR — 거울 깨짐 {}:\n{}", breaks.len(), breaks.join("\n"));
}

#[test]
fn crate_refs_reads_groups_and_raw() {
    let r = crate_refs("use crate::{room::X, r#pub::{A, B}};\nlet x = crate::flr::y();");
    assert_eq!(r, ["flr", "pub", "room"].iter().map(|s| s.to_string()).collect());
}
