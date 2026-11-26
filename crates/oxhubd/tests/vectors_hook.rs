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
fn judge_request_vectors() {
    use oxhubd::hook::zgate::judge::{Ask, judge_request};
    let v = load();
    let p = "ox/c/pub/v1/";
    for c in cases(&v, "judge_request") {
        let tail = c["tail"].as_str().expect("tail");
        let want = match c["ask"]["kind"].as_str().expect("kind") {
            "list" => Ask::List,
            "detail" => Ask::Detail(c["ask"]["room"].as_str().expect("room").to_string()),
            "ctl" => Ask::Ctl(c["ask"]["room"].as_str().expect("room").to_string()),
            _ => Ask::Outside,
        };
        assert_eq!(judge_request(&format!("{p}{tail}"), p), want, "정:RULE-HOOK-DIRECT · 원천 HOOK.model.JudgeRequest — 열린 키 셋만 답한다 · {tail}");
    }
    assert_eq!(judge_request("ox/c/int/q/rooms", p), Ask::Outside, "S-sym-RULE-BUS-PUBLIC-3① — pub/v1 밖은 답하지 않는다");
}

#[test]
fn judge_declare_vectors() {
    use oxhubd::hook::zgate::judge::{DeclareVerdict, Lane, judge_declare};
    let v = load();
    let p = "ox/c/pub/v1/";
    for c in cases(&v, "judge_declare") {
        let tail = c["tail"].as_str().expect("tail");
        let cn = c["cn"].as_str().expect("cn");
        let key = format!("{p}{tail}");
        let want = match c["verdict"]["kind"].as_str().expect("kind") {
            "own" => DeclareVerdict::Own(Lane::of(c["verdict"]["lane"].as_str().expect("lane")).expect("lane")),
            "other_account" => DeclareVerdict::OtherAccount(key.clone()),
            _ => DeclareVerdict::Outside(key.clone()),
        };
        assert_eq!(judge_declare(&key, p, cn), want, "정:RULE-HOOK-DIRECT · 원천 HOOK.model.JudgeDeclare — 선언 키 판정 · {tail} · {cn}");
    }
}

#[test]
fn carry_step_vectors() {
    use oxhubd::hook::carry::{Step, Try, step};
    use oxhubd::hook::zgate::send::End;
    let v = load();
    for c in cases(&v, "carry_step") {
        let t = match c["t"].as_str().expect("t") {
            "ack" => Try::Ended(End::Ack),
            "fail" => Try::Ended(End::Fail),
            "timeout" => Try::Ended(End::Timeout),
            "closed" => Try::Ended(End::Closed),
            _ => Try::Refused,
        };
        let tries = u32::try_from(c["tries"].as_u64().expect("tries")).expect("u32");
        let max = u32::try_from(c["max"].as_u64().expect("max")).expect("u32");
        let want = match c["step"]["kind"].as_str().expect("kind") {
            "done" => Step::Done,
            "give_up" => Step::GiveUp,
            _ => Step::Next(u32::try_from(c["step"]["tries"].as_u64().expect("next tries")).expect("u32")),
        };
        assert_eq!(step(t, tries, max), want, "S-rule-RULE-HOOK-FACTS-SUB-6①②③④⑤ · PL-KEY-HOOK-EXT-MAX-TRIES · 원천 HOOK.model.CarryStep — ACK 면 끝 · ACK 아닌 끝은 시도 +1 · 상한이면 버림 · 못 넣음은 세지 않음 · {c}");
    }
}

#[test]
fn backoff_vectors() {
    let v = load();
    for c in cases(&v, "backoff") {
        let failures = u32::try_from(c["failures"].as_u64().expect("failures")).expect("u32");
        let ms: Vec<u32> = c["ms"].as_array().expect("ms").iter().filter_map(|x| x.as_u64().and_then(|n| u32::try_from(n).ok())).collect();
        let want = c["wait"].as_u64().expect("wait");
        assert_eq!(oxhubd::hook::carry::backoff(failures, &ms).as_millis(), u128::from(want), "PL-KEY-HOOK-RETRY-BACKOFF-MS① · S-rule-RULE-HOOK-FACTS-SUB-6⑥ · 원천 HOOK.model.Backoff — n 번째 값 · 넘으면 마지막 값 · {c}");
    }
}

#[test]
fn next_wait_vectors() {
    use oxhubd::hook::carry::{Wait, next_wait};
    let v = load();
    for c in cases(&v, "next_wait") {
        let conns = usize::try_from(c["conns"].as_u64().expect("conns")).expect("usize");
        let free = c["free"].as_bool().expect("free");
        let ready = c["ready_in_ms"].as_i64();
        let want = match c["wait"]["kind"].as_str().expect("kind") {
            "until" => Wait::Until(std::time::Duration::from_millis(c["wait"]["in_ms"].as_u64().expect("in_ms"))),
            _ => Wait::Event,
        };
        assert_eq!(next_wait(conns, free, ready), want, "정:RULE-BUS-PUBLIC zenoh 넘기기 · 원천 HOOK.model.NextWait — 연결이 없거나 창이 다 찼으면 사건으로만 깬다 · {c}");
    }
}

#[test]
fn pick_vectors() {
    let v = load();
    for c in cases(&v, "pick") {
        let inflight: Vec<usize> = c["inflight"].as_array().expect("inflight").iter().filter_map(|x| x.as_u64().and_then(|n| usize::try_from(n).ok())).collect();
        let window = usize::try_from(c["window"].as_u64().expect("window")).expect("usize");
        let rr = c["rr"].as_u64().expect("rr");
        let want = c["pick"].as_u64().map(|i| usize::try_from(i).expect("usize"));
        assert_eq!(oxhubd::hook::carry::pick(&inflight, window, rr), want, "S-rule-RULE-HOOK-FACTS-SUB-7① · 원천 HOOK.model.Pick — 창이 찬 연결은 건너뛰고 · 모두 차면 보내지 않는다 · {c}");
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
