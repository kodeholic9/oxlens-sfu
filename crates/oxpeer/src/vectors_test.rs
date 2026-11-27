// author: kodeholic (powered by Claude)
//! 1층 — 생성 상태기 Rust 판 ≡ TS 판(202611/20261106c A4).
//! 벡터(tests/vectors/*.json)는 규격 짓기의 재생 시험이 TS 판 생성 코드에 넣은 호출과 그 결과다(context/spec 의 tools/vectors.mts · npm run sync:peer 가 옮긴다).
//! 같은 호출을 Rust 판에 넣어 결과(라벨 · 다음 상태 · 보낸 것 · 효과 · 이음매 · 타이머)가 같은지 본다 — 기대값을 손으로 쓰지 않는다.
//! 벡터의 IR sha 가 상태기 머리의 IR sha 와 다르면 낡은 벡터다 — 실패로 드러낸다(TOOLS 짓기 뒤 sync:peer 로 다시 옮긴다).

use serde_json::{Value, json};

use crate::generated as gen_;

fn vectors(name: &str) -> Value {
    let path = format!("{}/tests/vectors/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e} — context/spec 에서 TOOLS 짓기 뒤 npm run sync:peer"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn ir_sha(src: &str) -> &str {
    let at = src.find("IR sha256 ").expect("상태기 머리에 IR sha 가 없다") + "IR sha256 ".len();
    &src[at..at + 16]
}

/// 벡터 하나하나를 부르고 결과를 TS 판 꼴(JSON)로 견준다 — 다른 것은 처음 열 개만 보인다.
fn check(name: &str, src: &str, call: impl Fn(&str, &Value, &[Value]) -> Result<Value, String>) {
    let v = vectors(name);
    assert_eq!(v["ir_sha"].as_str(), Some(ir_sha(src)), "{name}: 벡터가 상태기보다 낡았다(IR sha) — TOOLS 짓기 뒤 sync:peer");
    let rows = v["v"].as_array().expect("v");
    assert!(!rows.is_empty(), "{name}: 벡터가 비었다");
    let mut bad = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let e = row["e"].as_str().unwrap_or("");
        let args = row["a"].as_array().cloned().unwrap_or_default();
        match call(e, &row["s"], &args) {
            Ok(got) if got == row["r"] => {}
            Ok(got) => bad.push(format!("#{i} {e} — Rust {got}\n        TS {}", row["r"])),
            Err(why) => bad.push(format!("#{i} {e} — {why}")),
        }
    }
    assert!(bad.is_empty(), "{name}: {} 중 {} 다르다\n  {}", rows.len(), bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n  "));
}

/// 영역 상태기 하나 — Step 을 TS 판 꼴로(out 한 줄 = {ws|http: op, part, 칸 …})
macro_rules! axis {
    ($test:ident, $file:literal, $m:ident) => {
        #[test]
        fn $test() {
            check($file, include_str!(concat!("generated/", stringify!($m), ".rs")), |e, s, a| {
                let st: gen_::$m::St = serde_json::from_value(s.clone()).map_err(|x| format!("앞 상태: {x}"))?;
                let Some(step) = gen_::$m::call_json(e, &st, a)? else { return Ok(Value::Null) };
                let mut v = serde_json::to_value(&step).map_err(|x| x.to_string())?;
                v["out"] = Value::Array(
                    step.out
                        .iter()
                        .map(|t| {
                            let mut o = serde_json::to_value(t).unwrap_or(Value::Null);
                            if o.is_null() {
                                o = json!({});
                            }
                            o[t.kind()] = t.op().into();
                            o["part"] = t.part().into();
                            o
                        })
                        .collect(),
                );
                Ok(v)
            });
        }
    };
}

axis!(session_rust_eq_ts, "session", session_client);
axis!(publish_rust_eq_ts, "publish", publish_client);
axis!(aff_rust_eq_ts, "aff", aff_client);
axis!(room_rust_eq_ts, "room", room_client);
axis!(link_rust_eq_ts, "link", link_client);
axis!(sub_rust_eq_ts, "sub", sub_client);
axis!(trk_rust_eq_ts, "trk", trk_client);

#[test]
fn floor_rust_eq_ts() {
    use gen_::floor_client as F;
    check("floor", include_str!("generated/floor_client.rs"), |e, s, a| {
        let cl: F::Cl = serde_json::from_value(s.clone()).map_err(|x| format!("앞 상태: {x}"))?;
        let Some(step) = F::call_json(e, &cl, a)? else { return Ok(Value::Null) };
        let mut v = serde_json::to_value(&step).map_err(|x| x.to_string())?;
        v["out"] = Value::Array(
            step.out
                .iter()
                .map(|t| {
                    let mut o = serde_json::to_value(t).unwrap_or(Value::Null);
                    if o.is_null() {
                        o = json!({});
                    }
                    o["t"] = t.op().into();
                    o
                })
                .collect(),
        );
        Ok(v)
    });
}
