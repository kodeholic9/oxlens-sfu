// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-DOC-CODE-MIRROR 정:CAND-DOC-GUARD-PLACE 정:CAND-DOC-ORCHESTRATION-ASPECT 정:RULE-DOC-OWNERSHIP · model: claude-opus-5-5
//! 구조 가드 — `server/out/deps.json` 을 읽어 셋을 센다: ①영역 없는 최상위 모듈 ②디렉터리 없는 개념 영역 ③개념 영역의 `may_use` 밖 `crate::` 호출. 규격 위치는 `OXLENS_SPEC` 이 이긴다 · 없으면 실패.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

const PROCESS_CRATES: &[&str] = &["oxhubd", "oxsfud", "oxlogd"];

fn spec_dir() -> PathBuf {
    match std::env::var("OXLENS_SPEC") {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec"),
    }
}

fn crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn deps() -> Value {
    let p = spec_dir().join("server/out/deps.json");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("S-sym-RULE-DOC-CODE-MIRROR-1 deps.json 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("deps.json 형: {e}"))
}

fn names(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_lowercase)).collect()).unwrap_or_default()
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

fn crate_targets(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = src;
    while let Some(i) = rest.find("crate::") {
        let after = &rest[i + "crate::".len()..];
        let after = after.strip_prefix("r#").unwrap_or(after);
        let id: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !id.is_empty() {
            out.insert(id.to_lowercase());
        }
        rest = &rest[i + "crate::".len()..];
    }
    out
}

#[test]
fn code_mirrors_spec_areas() {
    let d = deps();
    let concepts = names(&d, "concepts");
    let aspects = names(&d, "aspects");
    let known: BTreeSet<String> = concepts.iter().chain(aspects.iter()).cloned().collect();
    let may_use: BTreeMap<String, BTreeSet<String>> = d
        .get("areas")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.to_lowercase(), names(v, "may_use").into_iter().collect())).collect())
        .unwrap_or_default();
    let mut stray = Vec::new();
    let mut found_dirs = BTreeSet::new();
    let mut crossings = Vec::new();
    for c in PROCESS_CRATES {
        let src = crates_dir().join(c).join("src");
        let Ok(rd) = std::fs::read_dir(&src) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if p.is_file() && (name == "main" || name == "lib") {
                continue;
            }
            if !known.contains(&name) {
                stray.push(format!("{c}/src/{name}"));
                continue;
            }
            if p.is_dir() {
                found_dirs.insert(name.clone());
            }
            if !concepts.contains(&name) {
                continue;
            }
            let allowed = may_use.get(&name).cloned().unwrap_or_default();
            let mut files = Vec::new();
            if p.is_dir() {
                rs_files(&p, &mut files);
            } else {
                files.push(p.clone());
            }
            for f in files {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                for t in crate_targets(&text) {
                    if t != name && known.contains(&t) && !allowed.contains(&t) {
                        crossings.push(format!("{c}/{name} → {t} ({})", f.display()));
                    }
                }
            }
        }
    }
    let missing: Vec<&String> = concepts.iter().filter(|a| !found_dirs.contains(*a)).collect();
    assert!(stray.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 영역 없는 최상위 모듈: {stray:?}");
    assert!(missing.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 디렉터리 없는 개념 영역: {missing:?}");
    assert!(crossings.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-2 may_use 밖 crate:: 호출: {crossings:#?}");
}

#[test]
fn may_use_is_not_closure() {
    let d = deps();
    let sub = d.pointer("/areas/SUB/may_use").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
    let concepts = names(&d, "concepts").len();
    assert!(sub < concepts - 1, "S-sym-RULE-DOC-CODE-MIRROR-2 may_use 가 전이 닫힘이면 SUB 가 개념 전부를 부른다");
}
