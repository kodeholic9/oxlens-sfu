// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · model: claude-opus-5-5
//! 1층 벡터 — `server/out/vectors/hook.json`(원천 HOOK.model.*)의 hub 몫을 제품 함수(`oxhubd::hook::facts` · `oxhubd::hook::queue`)에 그대로 넣는다: A 평면 통지 → 사실 이름 · 사실 id 도장 · 사실 id 꼴 · 계정 고르기 · 계정 큐. 판 묶음이 없거나 비면 빨강. 발언권 전이(`floor_fact`)는 미디어 서버 몫이고 zenoh 입구 키(`judge_*`)는 입구 몫이다.

use std::path::{Path, PathBuf};

use oxhubd::hook::facts::{Stamp, fact_id, of_notice, wants};
use oxhubd::hook::queue::{Put, Queue};
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

fn cases<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    let a = v[key].as_array().unwrap_or_else(|| panic!("판 묶음 {key} 없음"));
    assert!(!a.is_empty(), "판 묶음 {key} 비었다");
    a
}

#[test]
fn of_notice_vectors() {
    let v = load();
    for c in cases(&v, "of_notice") {
        let op = c["op"].as_str().expect("op");
        let sub = c["sub"].as_str();
        assert_eq!(of_notice(op, sub), c["fact"].as_str(), "정:RULE-HOOK-FACTS 카탈로그 · 원천 HOOK.model.OfNotice — {op} {sub:?}");
    }
}

#[test]
fn stamp_vectors() {
    let v = load();
    for c in cases(&v, "stamp") {
        let name = c["name"].as_str().unwrap_or("");
        let mut s = match c["start"].as_object() {
            Some(o) => Stamp::at(o["ms"].as_u64().expect("ms"), u16::try_from(o["counter"].as_u64().expect("counter")).expect("u16")),
            None => Stamp::default(),
        };
        for st in c["steps"].as_array().expect("steps") {
            let got = s.next(st["now"].as_u64().expect("now"));
            let want = (st["got"]["ms"].as_u64().expect("ms"), u16::try_from(st["got"]["counter"].as_u64().expect("c")).expect("u16"));
            assert_eq!(got, want, "정:RULE-BUS-PUBLIC 사실 id · 원천 HOOK.model.Stamp — 차례 키는 늘 커진다 · {name}");
        }
    }
}

#[test]
fn fact_id_vectors() {
    let v = load();
    for c in cases(&v, "fact_id") {
        let ms = c["ms"].as_u64().expect("ms");
        let counter = u16::try_from(c["counter"].as_u64().expect("counter")).expect("u16");
        assert_eq!(fact_id(ms, counter, c["inst"].as_str().expect("inst")), c["id"].as_str().expect("id"), "정:RULE-BUS-PUBLIC 사실 id — ms 12 hex + 카운터 4 hex + 기동 신원");
    }
}

#[test]
fn wants_vectors() {
    let v = load();
    for c in cases(&v, "wants") {
        let facts: Vec<String> = c["facts"].as_array().expect("facts").iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        let kind = c["type"].as_str().expect("type");
        assert_eq!(wants(&facts, kind), c["wants"].as_bool().expect("wants"), "S-rule-RULE-HOOK-FACTS-SUB-2①② — {facts:?} {kind}");
    }
}

#[test]
fn queue_vectors() {
    let v = load();
    for c in cases(&v, "queue") {
        let name = c["name"].as_str().unwrap_or("");
        let mut q: Queue<String> = Queue::new(usize::try_from(c["max"].as_u64().expect("max")).expect("usize"));
        for st in c["steps"].as_array().expect("steps") {
            let step = st["step"].as_array().expect("step");
            match step[0].as_str().expect("verb") {
                "put" => {
                    let got = q.put(step[1].as_str().expect("id").to_string(), usize::try_from(step[2].as_u64().expect("bytes")).expect("usize"));
                    let want = if st["got"] == "dropped_oldest" { Put::DroppedOldest } else { Put::Queued };
                    assert_eq!(got, want, "S-rule-RULE-HOOK-WEBHOOK-5③ — 상한이면 오래된 것부터 버린다 · {name}");
                }
                "take" => {
                    let got = q.take(usize::try_from(step[1].as_u64().expect("max")).expect("usize"));
                    match st["got"].as_object() {
                        None => assert!(got.is_none(), "정:RULE-BUS-PUBLIC 계정 큐 — 줄도 버림 수도 없으면 묶음 없음 · {name}"),
                        Some(o) => {
                            let b = got.unwrap_or_else(|| panic!("묶음이 나와야 한다 · {name}"));
                            let ids: Vec<&str> = o["ids"].as_array().expect("ids").iter().filter_map(|x| x.as_str()).collect();
                            assert_eq!(b.items.iter().map(String::as_str).collect::<Vec<_>>(), ids, "S-rule-RULE-HOOK-FACTS-SUB-5⑥ — 묶음은 앞에서부터 바이트 상한까지 · 큰 것은 혼자 · {name}");
                            assert_eq!(b.dropped, o["dropped"].as_u64().expect("dropped"), "S-rule-RULE-HOOK-FACTS-SUB-5④⑦ — 버림 수를 싣고 0 으로 · 줄이 비어도 버린 수가 있으면 빈 묶음 · {name}");
                        }
                    }
                }
                "lost" => q.lost(step[1].as_u64().expect("facts"), step[2].as_u64().expect("carried")),
                other => panic!("모르는 걸음 {other}"),
            }
            let queued: Vec<&str> = st["queued"].as_array().expect("queued").iter().filter_map(|x| x.as_str()).collect();
            assert_eq!(q.ids(String::as_str), queued, "형상 — 줄 · {name}");
            assert_eq!(q.dropped(), st["dropped"].as_u64().expect("dropped"), "S-rule-RULE-HOOK-WEBHOOK-4④ — 버린 묶음은 사실 수와 실었던 dropped 를 되돌려 더한다 · {name}");
        }
    }
}
