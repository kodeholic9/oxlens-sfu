// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-DOC-COND-INDEX · 정:IMPL-ROOM-VERSION-ISSUE · 연:RULE-ROOM-VERSION-CLIENT-RULES · model: claude-opus-5-5
//! 1층 소스 가드 — 방 `version` 의 발급자는 sfud 방 모듈 하나다: 운영 소스에서 `Version {` 값을 짓는 자리는 `oxsfud/src/room/` 뿐 · hub 는 방별 `version` 커서를 두지 않는다(hub 소스의 `version` 은 `if_version` 넘김 · zgate 핸드셰이크 판 · `--version` · 판 어긋남 코드뿐)(context 202611/20261124b 서버 규칙 갈래).

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

fn product(krate: &str) -> Vec<(String, usize, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(krate).join("src");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut out = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&root).map(|x| x.display().to_string()).unwrap_or_default();
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for (n, l) in text.split("#[cfg(test)]").next().unwrap_or("").lines().enumerate() {
            if l.trim_start().starts_with("//") {
                continue;
            }
            out.push((format!("{krate}/src/{rel}"), n + 1, l.to_string()));
        }
    }
    out
}

#[test]
fn only_the_media_server_room_issues_a_version() {
    let mut all = Vec::new();
    for k in ["common", "oxsig", "oxsfud", "oxhubd", "oxadmin", "oxtapd"] {
        all.extend(product(k));
    }
    assert!(all.len() > 10_000, "stand check — the product sources were read ({})", all.len());
    let issuers: Vec<String> = all
        .iter()
        .filter(|(_, _, l)| l.contains("Version {") && !l.contains("struct Version"))
        .map(|(f, n, _)| format!("{f}:{n}"))
        .collect();
    assert!(!issuers.is_empty(), "stand check — the room version is issued somewhere");
    let stray: Vec<&String> = issuers.iter().filter(|f| !f.starts_with("oxsfud/src/room/")).collect();
    assert!(stray.is_empty(), "C-cond-RULE-DOC-COND-INDEX-14③ only the media server room module issues a room version: {stray:?}");
    let hub: Vec<String> = product("oxhubd")
        .into_iter()
        .filter(|(f, _, l)| {
            let low = l.to_lowercase();
            low.contains("version")
                && !low.contains("if_version")
                && !low.contains("versionmismatch")
                && !low.contains("--version")
                && !low.contains("\"version\"]")
                && !f.starts_with("oxhubd/src/hook/zgate/")
        })
        .map(|(f, n, l)| format!("{f}:{n} {}", l.trim()))
        .collect();
    assert!(hub.is_empty(), "C-cond-RULE-DOC-COND-INDEX-14③ the hub keeps no per-room version cursor: {hub:?}");
}
