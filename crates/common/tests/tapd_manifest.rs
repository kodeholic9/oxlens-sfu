// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-OPS-REC-UNIT · model: claude-opus-5-5
//! 녹음 유닛 매니페스트 가드 — `crates/oxtapd` 는 옆 저장소 oxlens-sdk-peer 를 경로로 물어 기본 빌드(루트 워크스페이스)에서 빠진 제 워크스페이스다 · 의존 판 · 패키지 칸 · lint · dev 프로필의 원천은 루트 `[workspace.*]` 하나이고 oxtapd 는 그것을 그대로 옮겨 적는다 · 어긋나면 여기서 빨강.

use std::path::{Path, PathBuf};

use toml::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(p: &Path) -> Value {
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    text.parse::<Value>().unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn spec_of(v: &Value) -> (Option<String>, Option<String>, Option<bool>, Vec<String>) {
    match v {
        Value::String(s) => (Some(s.clone()), None, None, vec![]),
        Value::Table(t) => (
            t.get("version").and_then(Value::as_str).map(str::to_string),
            t.get("path").and_then(Value::as_str).map(str::to_string),
            t.get("default-features").and_then(Value::as_bool),
            t.get("features").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default(),
        ),
        _ => (None, None, None, vec![]),
    }
}

#[test]
fn tapd_mirrors_the_root_workspace() {
    let r = load(&root().join("Cargo.toml"));
    let t = load(&root().join("crates/oxtapd/Cargo.toml"));
    let ws = &r["workspace"];
    let members: Vec<&str> = ws["members"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let exclude: Vec<&str> = ws.get("exclude").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    assert!(!members.contains(&"crates/oxtapd") && exclude.contains(&"crates/oxtapd"), "정:IMPL-OPS-REC-UNIT oxtapd 는 기본 빌드 밖 {members:?} {exclude:?}");
    assert!(t.get("workspace").is_some(), "oxtapd 는 제 워크스페이스");
    for k in ["edition", "license", "authors"] {
        assert_eq!(t["package"].get(k), ws["package"].get(k), "패키지 칸 {k} 이 루트와 다르다");
    }
    assert_eq!(t.get("lints"), ws.get("lints"), "lint 가 루트와 다르다");
    assert_eq!(t.get("profile").and_then(|p| p.get("dev")), r.get("profile").and_then(|p| p.get("dev")), "dev 프로필이 루트와 다르다");
    let wdeps = ws["dependencies"].as_table().expect("루트 [workspace.dependencies]");
    let mut seen = 0;
    for sec in ["dependencies", "dev-dependencies"] {
        for (name, v) in t[sec].as_table().expect("의존 표").iter() {
            let (ver, path, dflt, feats) = spec_of(v);
            if name == "oxpeer" {
                assert_eq!(path.as_deref(), Some("../../../oxlens-sdk-peer/crates/oxpeer"), "oxpeer 는 옆 저장소 경로");
                continue;
            }
            let w = wdeps.get(name).unwrap_or_else(|| panic!("{sec}.{name} 이 루트 [workspace.dependencies] 에 없다 — 루트에 먼저 둔다"));
            let (wver, wpath, wdflt, wfeats) = spec_of(w);
            match wpath {
                Some(wp) => assert_eq!(path, Some(format!("../{}", wp.trim_start_matches("crates/"))), "{sec}.{name} 경로"),
                None => {
                    assert_eq!(ver, wver, "{sec}.{name} 판이 루트와 다르다");
                    assert_eq!(dflt, wdflt, "{sec}.{name} default-features 가 루트와 다르다");
                    assert!(wfeats.iter().all(|f| feats.contains(f)), "{sec}.{name} 기능 {feats:?} 이 루트 {wfeats:?} 를 다 품지 않는다");
                }
            }
            seen += 1;
        }
    }
    assert!(seen >= 8, "의존을 못 읽었다(안 쟀다) {seen}");
}
