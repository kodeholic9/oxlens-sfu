// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:RULE-HOOK-DIRECT · model: claude-opus-5-5
//! zenoh 종속 가드 — 운영 소스(`crates/*/src`)에서 zenoh 크레이트(`zenoh` · `zenoh_*`)를 쓰는 곳은 대외 연동의 zenoh 입구 `crates/oxhubd/src/hook/zgate/` 하나뿐 · `[dependencies]` 에 zenoh 크레이트를 두는 크레이트는 oxhubd 뿐이고 셋(`zenoh-protocol` · `zenoh-codec` · `zenoh-buffers`)만 `=1.10.1` 로 고정(상호운용 시험의 실제 클라이언트 `zenoh` 는 `[dev-dependencies]`).

use std::path::{Path, PathBuf};

const ALLOWED: [&str; 3] = ["zenoh-protocol", "zenoh-codec", "zenoh-buffers"];
const PIN: &str = "\"=1.10.1\"";

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
    krate == "oxhubd" && rel.to_string_lossy().replace('\\', "/").starts_with("src/hook/zgate/")
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn zenoh_uses(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        let code = code.split(" //").next().unwrap_or(code);
        let mut from = 0;
        while let Some(i) = code[from..].find("zenoh") {
            let at = from + i;
            let end = at + code[at..].chars().take_while(|c| ident_char(*c)).map(char::len_utf8).sum::<usize>();
            from = end.max(at + 5);
            let before = code[..at].chars().next_back();
            if before.map(|c| ident_char(c) || c == '"' || c == '-' || c == '/').unwrap_or(false) {
                continue;
            }
            let name = &code[at..end];
            if !(name == "zenoh" || name.starts_with("zenoh_")) {
                continue;
            }
            let head = code[..at].trim_end();
            if code[end..].trim_start().starts_with("::") || head.ends_with("use") || head.ends_with("crate") {
                out.push((n + 1, line.trim().to_string()));
            }
        }
    }
    out
}

fn deps(toml: &str, section: &str) -> Vec<(String, String)> {
    let mut on = false;
    let mut out = Vec::new();
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            on = t == section;
            continue;
        }
        if !on {
            continue;
        }
        let name = t.split(['=', '.', ' ']).next().unwrap_or("");
        if name == "zenoh" || name.starts_with("zenoh-") {
            out.push((name.to_string(), t.to_string()));
        }
    }
    out
}

#[test]
fn zenoh_reads_split() {
    let found = zenoh_uses("use zenoh_protocol::core;\nlet x = zenoh_codec::Zenoh080::new();\n// zenoh_buffers::ZBuf\nlet s = \"zenoh_protocol\";\nlet my_zenoh_x = 1;\nzenoh::open(c);\nuse oxhubd::hook::zgate;\n");
    assert_eq!(found.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![1, 2, 6], "판독기 — 경로 · use 는 잡고 주석 · 문자열 · 다른 이름은 안 잡는다");
    assert_eq!(deps("[dependencies]\nzenoh-codec = \"=1.10.1\"\n[dev-dependencies]\nzenoh = { version = \"=1.10.1\" }\n", "[dependencies]").len(), 1);
}

#[test]
fn zenoh_only_in_zgate() {
    let root = crates_dir();
    let crates = crate_names();
    assert!(crates.iter().any(|c| c == "oxhubd") && crates.iter().any(|c| c == "oxsfud"), "크레이트 목록을 못 읽었다(안 쟀다) {crates:?}");
    let mut leaks = Vec::new();
    let mut scanned = 0usize;
    let mut owner_uses = 0usize;
    for c in &crates {
        let base = root.join(c);
        let mut files = Vec::new();
        rs_files(&base.join("src"), &mut files);
        for f in files {
            scanned += 1;
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let rel = f.strip_prefix(&base).unwrap_or(&f).to_path_buf();
            let uses = zenoh_uses(&text);
            if owner(c, &rel) {
                owner_uses += uses.len();
                for (n, l) in text.lines().enumerate() {
                    if l.trim_start().starts_with("pub use zenoh") {
                        leaks.push(format!("{}:{} zenoh 형을 밖에 낸다: {}", f.display(), n + 1, l.trim()));
                    }
                }
                continue;
            }
            for (n, l) in uses {
                leaks.push(format!("{}:{} {l}", f.display(), n));
            }
        }
        let toml = std::fs::read_to_string(base.join("Cargo.toml")).unwrap_or_default();
        for (name, line) in deps(&toml, "[dependencies]") {
            if c != "oxhubd" {
                leaks.push(format!("{}/Cargo.toml [dependencies] 에 {name}", base.display()));
            } else if !ALLOWED.contains(&name.as_str()) || !line.contains(PIN) {
                leaks.push(format!("oxhubd [dependencies] {line} — 셋({ALLOWED:?})만 {PIN} 로"));
            }
        }
    }
    assert!(scanned > 0, "소스 파일을 못 찾았다(안 쟀다)");
    assert!(owner_uses > 0, "zgate 에서 zenoh 사용을 하나도 못 봤다 — 주인 경로가 낡았다");
    assert!(leaks.is_empty(), "정:RULE-HOOK-DIRECT — zenoh 크레이트는 oxhubd hook/zgate 에서만 쓴다 · 샌 곳 {}:\n{}", leaks.len(), leaks.join("\n"));
}
