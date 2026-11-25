// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-BUS-PRESENCE · 정:RULE-BUS-KEYSPACE · model: claude-opus-5-5
//! 1층 소스 가드 — 버스 토큰은 `node` · `room` · `sess` · `ext` 넷뿐이다: 운영 소스의 `declare_token` 자리마다 키가 그 넷의 키 짓기 함수에서 나온다 · 방 명단 토큰은 없다(명단의 권위는 sfud 스냅샷 하나)(context 202611/20261124b 서버 규칙 갈래).

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

const KINDS: [&str; 4] = ["node_token(", "room_token(", "sess_token(", "ext_token("];

#[test]
fn bus_tokens_are_node_room_sess_ext_only() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for e in std::fs::read_dir(&crates).expect("crates dir").flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "oxmesh" {
            continue;
        }
        rs_files(&e.path().join("src"), &mut files);
    }
    assert!(files.len() > 50, "stand check — the product sources were read ({})", files.len());
    let mut sites = 0;
    let mut kinds = BTreeSet::new();
    let mut bad = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let lines: Vec<&str> = text.split("#[cfg(test)]").next().unwrap_or("").lines().collect();
        for (n, l) in lines.iter().enumerate() {
            let near = lines[n.saturating_sub(2)..=n].join(" ");
            if !l.contains(".declare_token(") || near.contains("fn declare_token") {
                continue;
            }
            sites += 1;
            let hit: Vec<&str> = KINDS.iter().copied().filter(|k| near.contains(&format!("keys.{k}"))).collect();
            if hit.len() == 1 {
                kinds.insert(hit[0]);
            } else {
                bad.push(format!("{}:{} {}", f.display(), n + 1, l.trim()));
            }
        }
    }
    assert!(sites >= 4, "stand check — the token declarations were found ({sites})");
    assert!(bad.is_empty(), "S-sym-RULE-BUS-PRESENCE-4 every bus token key comes from node · room · sess · ext — no roster token: {bad:?}");
    assert_eq!(kinds.len(), 4, "S-sym-RULE-BUS-PRESENCE-4 the four token kinds are all in use: {kinds:?}");
}
