// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-BUS-SHAPE · 정:RULE-MESH-SHAPE · model: claude-opus-5-5
//! 버스 종속 가드 — 운영 소스(`crates/*/src`)에서 oxmesh 를 쓰는 곳은 `crates/oxmesh/` · client 공통 `common` 의 `bus` · router 쪽 `crates/oxhubd/src/bus/` 셋뿐 · 그 둘도 `pub use oxmesh` 로 형을 내지 않는다 · `[dependencies]` 에 oxmesh 를 두는 크레이트는 common · oxhubd 뿐(시험의 router 는 `[dev-dependencies]`).

use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
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

fn crate_names() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(crates_dir())
        .map(|rd| rd.flatten().filter(|e| e.path().join("Cargo.toml").is_file()).filter_map(|e| e.file_name().to_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn owner(krate: &str, rel: &Path) -> bool {
    let r = rel.to_string_lossy().replace('\\', "/");
    match krate {
        "common" => r == "src/bus.rs" || r.starts_with("src/bus/"),
        "oxhubd" => r.starts_with("src/bus/"),
        _ => false,
    }
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn mesh_uses(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        let code = code.split(" //").next().unwrap_or(code);
        let mut from = 0;
        while let Some(i) = code[from..].find("oxmesh") {
            let at = from + i;
            from = at + "oxmesh".len();
            let before = code[..at].chars().next_back();
            let after = &code[from..];
            if before.map(|c| ident_char(c) || c == '"').unwrap_or(false) || after.chars().next().map(ident_char).unwrap_or(false) {
                continue;
            }
            let head = code[..at].trim_end();
            if after.trim_start().starts_with("::") || head.ends_with("use") || head.ends_with("crate") {
                out.push((n + 1, line.trim().to_string()));
            }
        }
    }
    out
}

fn dep_section_has_mesh(toml: &str) -> bool {
    let mut in_deps = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_deps = t == "[dependencies]";
            continue;
        }
        if in_deps && t.split(['=', '.', ' ']).next() == Some("oxmesh") {
            return true;
        }
    }
    false
}

#[test]
fn mesh_reads_split() {
    let found = mesh_uses("use oxmesh::{Mesh, Token};\nlet m = oxmesh::Mesh::open(c);\n// oxmesh::Query\nlet f = target.starts_with(\"oxmesh\");\nuse oxmesh as m;\nlet my_oxmesh::x = 1;\n");
    assert_eq!(found.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![1, 2, 5], "판독기 — 경로 · use 는 잡고 주석 · 문자열 · 다른 이름은 안 잡는다");
    assert!(dep_section_has_mesh("[dependencies]\noxmesh = { workspace = true }\n"));
    assert!(!dep_section_has_mesh("[dependencies]\ncommon = 1\n\n[dev-dependencies]\noxmesh = { workspace = true }\n"));
}

#[test]
fn mesh_only_in_bus_owners() {
    let root = crates_dir();
    let crates = crate_names();
    assert!(crates.iter().any(|c| c == "oxtapd") && crates.iter().any(|c| c == "oxsfud"), "크레이트 목록을 못 읽었다(안 쟀다) {crates:?}");
    let mut leaks = Vec::new();
    let mut scanned = 0usize;
    let mut owner_uses = 0usize;
    for c in crates.iter().filter(|c| c.as_str() != "oxmesh") {
        let base = root.join(c);
        let mut files = Vec::new();
        rs_files(&base.join("src"), &mut files);
        for f in files {
            scanned += 1;
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let rel = f.strip_prefix(&base).unwrap_or(&f).to_path_buf();
            let uses = mesh_uses(&text);
            if owner(c, &rel) {
                owner_uses += uses.len();
                for (n, l) in text.lines().enumerate() {
                    if l.trim_start().starts_with("pub use oxmesh") {
                        leaks.push(format!("{}:{} oxmesh 형을 밖에 낸다: {}", f.display(), n + 1, l.trim()));
                    }
                }
                continue;
            }
            for (n, l) in uses {
                leaks.push(format!("{}:{} {l}", f.display(), n));
            }
        }
        let toml = std::fs::read_to_string(base.join("Cargo.toml")).unwrap_or_default();
        if dep_section_has_mesh(&toml) && !matches!(c.as_str(), "common" | "oxhubd") {
            leaks.push(format!("{}/Cargo.toml [dependencies] 에 oxmesh", base.display()));
        }
    }
    assert!(scanned > 0, "소스 파일을 못 찾았다(안 쟀다)");
    assert!(owner_uses > 0, "bus 모듈 둘에서 oxmesh 사용을 하나도 못 봤다 — 주인 경로가 낡았다");
    assert!(leaks.is_empty(), "정:RULE-BUS-SHAPE — oxmesh 는 common 의 bus(client) · oxhubd bus/(router) 로만 쓴다 · 샌 곳 {}:\n{}", leaks.len(), leaks.join("\n"));
}
