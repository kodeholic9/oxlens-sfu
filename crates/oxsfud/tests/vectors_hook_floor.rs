// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/hook.json` 의 `floor_fact`(발언권 상태기 전이 → 사실 — 원천 HOOK.model.FloorFact)를 제품 함수(`ops::floor::fact_of`)에 그대로 넣는다. 판 묶음이 없거나 비면 빨강.

use std::path::{Path, PathBuf};

use oxsfud::flr::core::G;
use oxsfud::ops::floor::fact_of;
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

fn at(v: &Value) -> (G, Option<String>) {
    let g = match v["g"].as_str().expect("g") {
        "idle" => G::Idle,
        "taken" => G::Taken,
        "revoking" => G::Prevoke,
        other => panic!("모르는 g {other}"),
    };
    (g, v["holder"].as_str().map(str::to_string))
}

#[test]
fn floor_fact_vectors() {
    let v = load();
    let cases = v["floor_fact"].as_array().expect("판 묶음 floor_fact 없음");
    assert!(!cases.is_empty(), "판 묶음 floor_fact 비었다");
    let mut names = std::collections::BTreeSet::new();
    for c in cases {
        let (bg, bh) = at(&c["before"]);
        let (ag, ah) = at(&c["after"]);
        let want = c["fact"].as_str();
        let got = fact_of((bg, bh.as_deref()), (ag, ah.as_deref()));
        assert_eq!(
            got,
            want,
            "HK-FACT-FLOOR-GRANTED①③④⑤ · HK-FACT-FLOOR-RELEASED②④⑤ · HK-FACT-FLOOR-REVOKED④⑤ — {} {}",
            c["name"],
            c
        );
        names.insert(want.unwrap_or("none"));
    }
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        vec!["floor_granted", "floor_released", "floor_revoked", "none"],
        "stand check — the vector set covers the three facts and the non-facts"
    );
}
