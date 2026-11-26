// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-CFG-SYSTEM-FILE · 정:RULE-CFG-THREE-PLACES · model: claude-opus-5-5
//! 1층 형상 — 2층이 띄우는 시스템 파일(`dev/system.node-*.toml`)이 엄격 파싱(`System::load` — hub 기동 길)을 지난다. 칸을 늘려도 옛 파일이 거절되지 않는다(새 칸은 선택값). 파일이 하나도 없으면 빨강.

use std::path::Path;

use common::system::System;

#[test]
fn dev_node_system_files_boot() {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dev");
    let mut files: Vec<_> = std::fs::read_dir(&dev)
        .expect("dev 디렉터리")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("system.node-") && n.ends_with(".toml")))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "형상 — dev/system.node-*.toml 이 없다");
    for f in &files {
        let s = System::load(f).unwrap_or_else(|e| panic!("정:IMPL-CFG-SYSTEM-FILE — {} 기동 실패: {e}", f.display()));
        assert!(s.zenoh_listen().is_some(), "형상 — {} 는 zenoh 입구를 연다", f.display());
    }
}
