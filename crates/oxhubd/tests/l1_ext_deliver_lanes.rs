// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:RULE-BUS-PUBLIC · 정:RULE-HOOK-FACTS-SUB · model: claude-opus-5-5
//! 1층 — node 사이 3rd 넘기기(`q/node/{node}/ext`)의 몸은 갈래(`lane`) 둘이다: 채팅 묶음(`chat` — 메시지 목록)과 사실 묶음(`facts` — `{"facts": [...], "dropped": N?}`). 사실 묶음 몸은 3rd 가 받는 몸 그대로이고 `live` 칸이 없으며 버림이 없으면 `dropped` 칸도 없다.

use oxhubd::hook::facts::Fact;
use oxhubd::hook::zgate::send::{ExtDeliver, FactsBundle};
use serde_json::json;

#[test]
fn chat_lane_keeps_its_messages() {
    let d = ExtDeliver::Chat { key: "acme".into(), cid: "c1".into(), messages: Vec::new() };
    let v = serde_json::to_value(&d).expect("encode");
    assert_eq!(v["lane"], "chat", "정:RULE-CHAT-EXT-SUB 4 — 넘기기 몸의 갈래");
    assert_eq!(serde_json::from_value::<ExtDeliver>(v).expect("decode"), d);
    assert_eq!((d.key(), d.cid()), ("acme", "c1"));
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct OldExtDeliver {
    key: String,
    cid: String,
    messages: Vec<oxsig::msg::ChatMessage>,
}

#[test]
fn mixed_versions_keep_the_chat_lane() {
    let old = serde_json::to_vec(&OldExtDeliver { key: "acme".into(), cid: "c1".into(), messages: Vec::new() }).expect("old encode");
    assert_eq!(
        serde_json::from_slice::<ExtDeliver>(&old).expect("new reads old"),
        ExtDeliver::Chat { key: "acme".into(), cid: "c1".into(), messages: Vec::new() },
        "정:RULE-CHAT-EXT-SUB 4 — 판을 하나씩 올리는 동안 옛 판 node 의 넘기기 몸(lane 없음)은 채팅 갈래로 읽힌다"
    );
    let new = serde_json::to_vec(&ExtDeliver::Chat { key: "acme".into(), cid: "c1".into(), messages: Vec::new() }).expect("new encode");
    assert_eq!(
        serde_json::from_slice::<OldExtDeliver>(&new).expect("old reads new"),
        OldExtDeliver { key: "acme".into(), cid: "c1".into(), messages: Vec::new() },
        "정:RULE-CHAT-EXT-SUB 4 — 새 판의 채팅 넘기기 몸은 옛 형(모르는 칸 lane 을 무시)으로 읽힌다"
    );
    assert!(serde_json::from_slice::<ExtDeliver>(br#"{"lane":"facts","key":"a","cid":"c"}"#).is_err(), "형상 — 사실 갈래는 묶음이 있어야 한다");
    assert!(serde_json::from_slice::<ExtDeliver>(br#"{"key":"a","cid":"c"}"#).is_err(), "형상 — 채팅 갈래는 메시지 목록이 있어야 한다");
}

#[test]
fn facts_lane_carries_the_bundle_body_as_is() {
    let fact = Fact { fact_id: "019ab697e8000000h1".into(), kind: "room_created".into(), t: 1, room_id: "r1".into(), body: json!({ "room_id": "r1" }) };
    let d = ExtDeliver::Facts { key: "acme".into(), cid: "c1".into(), bundle: FactsBundle { facts: vec![fact], dropped: None } };
    let v = serde_json::to_value(&d).expect("encode");
    assert_eq!(v["lane"], "facts");
    let b = &v["bundle"];
    assert_eq!(b["facts"][0]["type"], "room_created", "S-rule-RULE-HOOK-FACTS-SUB-5① — 묶음 몸의 사실 꼴은 봉투(type = 카탈로그 이름)");
    assert!(b.get("live").is_none(), "S-rule-RULE-HOOK-FACTS-SUB-5② — live 칸 0");
    assert!(b.get("dropped").is_none(), "S-rule-RULE-HOOK-FACTS-SUB-5③ — 버린 것이 없으면 dropped 칸 0");
    let with = FactsBundle { facts: Vec::new(), dropped: Some(8) };
    assert_eq!(serde_json::to_value(&with).expect("encode"), json!({ "facts": [], "dropped": 8 }), "S-rule-RULE-HOOK-FACTS-SUB-5⑦ — 빈 묶음 몸 {{\"facts\": [], \"dropped\": N}}");
    assert_eq!(serde_json::from_value::<ExtDeliver>(v).expect("decode"), d);
}
