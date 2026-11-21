// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/chat.json`(구독 항목 꼴 · 매칭)을 제품 함수(`common::extsub`)에 그대로 넣는다. 판 묶음이 없거나 비면 빨강.

use std::path::{Path, PathBuf};

use common::extsub::{By, SubItem, Subscribe, follows, matches_raw};
use serde_json::Value;

fn spec() -> PathBuf {
    if let Ok(p) = std::env::var("OXLENS_SPEC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../context/spec")
}

fn load() -> Value {
    let p = spec().join("server/out/vectors/chat.json");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("벡터 없음 {}: {e}", p.display()));
    serde_json::from_str(&t).expect("벡터 JSON")
}

fn cases<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = v[key].as_array().unwrap_or_else(|| panic!("판 묶음 {key} 없음"));
    assert!(!a.is_empty(), "판 묶음 {key} 비었다");
    a
}

fn form(it: &SubItem) -> (&'static str, Option<&str>, Option<&str>) {
    match it {
        SubItem::All => ("all", None, None),
        SubItem::Text => ("text", None, None),
        SubItem::Prefix(p) => ("prefix", Some(p.as_str()), None),
        SubItem::Kind(k) => ("kind", None, Some(k.as_str())),
    }
}

#[test]
fn ext_items_vectors() {
    let v = load();
    for c in cases(&v, "ext_items") {
        let s = c["item"].as_str().expect("item");
        let got = SubItem::parse(s);
        assert_eq!(got.is_some(), c["ok"].as_bool().expect("ok"), "정:RULE-CHAT-EXT-SUB 1 — 항목 {s:?} 꼴");
        if let Some(it) = got {
            let (f, p, k) = form(&it);
            assert_eq!((f, p, k), (c["form"].as_str().unwrap_or_default(), c["prefix"].as_str(), c["kind"].as_str()), "정:RULE-CHAT-EXT-SUB 1 — 항목 {s:?}");
        }
    }
}

#[test]
fn ext_match_vectors() {
    let v = load();
    for c in cases(&v, "ext_match") {
        let name = c["name"].as_str().unwrap_or_default();
        let sub: Vec<String> = c["subscribe"].as_array().expect("subscribe").iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect();
        let kind = c["kind"].as_str().expect("kind");
        let orig = c["orig"].as_str();
        assert_eq!(follows(kind), c["follows"].as_bool().expect("follows"), "{name} — 원본 따라 걸기");
        let want = c["hit"].as_object().map(|h| (h["by"].as_str().unwrap_or_default().to_string(), h["item"].as_u64().unwrap_or(u64::MAX) as usize));
        let got = matches_raw(&sub, kind, orig).map(|h| (h.by.as_str().to_string(), h.item));
        assert_eq!(got, want, "정:RULE-CHAT-EXT-SUB 1 — {name}");
        if let Ok(strict) = Subscribe::try_from(sub.clone()) {
            assert_eq!(strict.matches(kind, orig).map(|h| (h.by.as_str().to_string(), h.item)), want, "{name} — 설정으로 읽은 구독도 같은 판단");
        }
    }
    assert!(cases(&v, "ext_match").iter().any(|c| c["hit"]["by"] == By::Orig.as_str()), "형상 — 원본 갈래 판이 있다");
}
