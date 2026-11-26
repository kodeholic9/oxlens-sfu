// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS-SUB · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/hook.json` 의 `facts_item`(계정 `facts` 항목 꼴 — 원천 HOOK.model.FactsItem)을 제품 함수(`common::envelope::facts_item_ok`)에 그대로 넣는다 · 카탈로그 열세 이름이 원천과 같은지 본다. 판 묶음이 없거나 비면 빨강.

use std::path::{Path, PathBuf};

use common::envelope::{FACT_CATALOG, facts_item_ok};
use serde_json::Value;

fn spec() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/spec")
}

fn load() -> Value {
    let p = spec().join("server/out/vectors/hook.json");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).expect("벡터 JSON")
}

#[test]
fn facts_item_vectors() {
    let v = load();
    let cases = v["facts_item"].as_array().expect("판 묶음 facts_item 없음");
    assert!(!cases.is_empty(), "판 묶음 facts_item 비었다");
    for c in cases {
        let item = c["item"].as_str().expect("item");
        let want = c["ok"].as_bool().expect("ok");
        assert_eq!(facts_item_ok(item), want, "정:IMPL-CFG-SYSTEM-FILE · 원천 HOOK.model.FactsItem — 항목 {item:?} 꼴");
    }
    let named: Vec<&str> = cases.iter().filter(|c| c["ok"] == true).filter_map(|c| c["item"].as_str()).filter(|s| *s != "*").collect();
    assert_eq!(named, FACT_CATALOG.to_vec(), "정:RULE-HOOK-FACTS 카탈로그 — 열세 이름이 원천과 같다");
}
