// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-DOC-CODE-MIRROR · 정:RULE-DOC-OWNERSHIP · model: claude-opus-5-5
//! 구조 가드 — 코드 배치는 규격 영역의 거울이다: 영역 없는 최상위 모듈 · 디렉터리 없는 영역 · `may_use` 밖 호출(`server/out/deps.json`)을 센다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn spec_dir() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn crates() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

struct Deps {
    concepts: BTreeSet<String>,
    known: BTreeSet<String>,
    may_use: BTreeMap<String, BTreeSet<String>>,
}

fn deps() -> Deps {
    let p = spec_dir().join("server/out/deps.json");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("deps.json {}: {e} — OXLENS_SPEC 로 규격 위치를 준다", p.display()));
    let v: Value = serde_json::from_str(&t).unwrap_or_else(|e| panic!("deps.json: {e}"));
    let list = |k: &str| -> BTreeSet<String> { v[k].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str).map(str::to_lowercase).collect() };
    let concepts = list("concepts");
    let mut known = concepts.clone();
    known.extend(list("aspects"));
    let mut may_use = BTreeMap::new();
    if let Some(a) = v["areas"].as_object() {
        for (k, x) in a {
            let set: BTreeSet<String> = x["may_use"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str).map(str::to_lowercase).collect();
            may_use.insert(k.to_lowercase(), set);
        }
    }
    Deps { concepts, known, may_use }
}

fn modules(src: &Path) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(src).unwrap_or_else(|e| panic!("{}: {e}", src.display())).flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let dir = e.path().is_dir();
        let m = if dir { name.clone() } else { name.trim_end_matches(".rs").to_string() };
        if !dir && (m == "main" || m == "lib" || !name.ends_with(".rs")) {
            continue;
        }
        out.push((m, dir));
    }
    out.sort();
    out
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn area_of(m: &str) -> String {
    m.trim_start_matches("r#").to_string()
}

fn calls(text: &str, known: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find("crate::") {
        let s = i + off + "crate::".len();
        let rest = &text[s..];
        if let Some(inner) = rest.strip_prefix('{') {
            let end = inner.find('}').unwrap_or(inner.len());
            for part in inner[..end].split(',') {
                let head = part.trim().split("::").next().unwrap_or("").trim();
                let a = area_of(head);
                if known.contains(&a) {
                    out.insert(a);
                }
            }
        } else {
            let head: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '#').collect();
            let a = area_of(&head);
            if known.contains(&a) {
                out.insert(a);
            }
        }
        i = s;
        if i >= bytes.len() {
            break;
        }
    }
    out
}

#[test]
fn code_mirrors_spec_areas() {
    let d = deps();
    let owned: [(&str, &[&str]); 4] = [
        ("oxsfud", &["link", "room", "aff", "pub", "trk", "sub", "flr", "bus", "fwd", "sim", "rtcp", "aux", "ops"]),
        ("oxhubd", &["ses", "bus", "ops"]),
        ("oxlogd", &["ops"]),
        ("oxadmin", &["ops"]),
    ];
    let mut stray = Vec::new();
    let mut missing = Vec::new();
    let mut outside = Vec::new();
    for (krate, must) in owned {
        let src = crates().join(krate).join("src");
        let mods = modules(&src);
        for (m, _) in &mods {
            if !d.known.contains(&area_of(m)) {
                stray.push(format!("{krate}/src/{m}"));
            }
        }
        for a in must {
            if !mods.iter().any(|(m, dir)| *dir && area_of(m) == *a) {
                missing.push(format!("{krate}/src/{a}/"));
            }
        }
        for (m, _) in &mods {
            let a = area_of(m);
            if !d.concepts.contains(&a) {
                continue;
            }
            let allowed = d.may_use.get(&a).cloned().unwrap_or_default();
            let mut fs = Vec::new();
            let p = src.join(m);
            if p.is_dir() {
                files(&p, &mut fs);
            } else {
                fs.push(src.join(format!("{m}.rs")));
            }
            for f in fs {
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                for c in calls(&text, &d.known) {
                    if c != a && d.concepts.contains(&c) && !allowed.contains(&c) {
                        outside.push(format!("{} → {c}", f.strip_prefix(crates()).unwrap_or(&f).display()));
                    }
                }
            }
        }
    }
    assert!(stray.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 영역 없는 최상위 모듈 {stray:?}");
    assert!(missing.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 디렉터리 없는 영역 {missing:?}");
    assert!(outside.is_empty(), "S-sym-RULE-DOC-CODE-MIRROR-1 may_use 밖 호출 {outside:?}");
}

#[test]
fn guard_counts_a_planted_violation() {
    let d = deps();
    let c = calls("use crate::{room::Room, link::lock};\nfn f() { crate::r#pub::x(); }", &d.known);
    assert_eq!(c, ["link", "pub", "room"].iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(), "S-sym-RULE-DOC-CODE-MIRROR-1 가드가 호출을 센다");
    let flr_may = d.may_use.get("flr").cloned().unwrap_or_default();
    assert!(!flr_may.contains("link"), "S-sym-RULE-DOC-CODE-MIRROR-2 may_use 는 전이 닫힘이 아니다 — FLR → LINK 는 밖");
}
