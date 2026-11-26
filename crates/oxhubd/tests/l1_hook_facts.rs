// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-HOOK-FACTS · 정:RULE-BUS-PUBLIC · 정:RULE-BUS-EVENTS · 정:RULE-HOOK-FACTS-SUB · 정:RULE-HOOK-STATUS · 정:RULE-OPS-DROPS · model: claude-opus-5-5
//! 1층 — 방을 맡은 hub 의 사실 짓기: 방 통지 가르기(재료 표지는 맡은 hub 만 · 맡지 않으면 버림 · 못 읽는 재료 · A 평면 통지는 배달 + 맡은 방이면 사실 하나) · 사실 봉투(칸 다섯 · 받은 시각 · 본문은 A 평면 그대로 · 사실 id 는 받은 차례로 커진다) · 계정 큐(고르기 · 상한 버림 계수 · 묶음 · 버린 묶음 되돌려 세기 계수) · 늘 관심(생성 Call 전에 선언 · 실패면 거둠 · 따로 센다) · 대외 방 목록 엄격 갈래.

use common::envelope::{FloorIdle, FloorTaken, Material, Notice, OP_FACT_MATERIAL};
use oxhubd::bus::{Bus, BusCfg};
use oxhubd::hook::Hook;
use oxhubd::hook::facts::{Route, notice_kind, route};
use oxsig::Code;
use oxsig::msg::RoomSummary;
use serde_json::json;
use std::sync::atomic::Ordering;

fn notice(op: u16, body: serde_json::Value) -> Notice {
    Notice { op, body, exclude: Vec::new(), room_id: Some("r1".into()), sfu_id: Some("e1".into()) }
}

fn summary() -> RoomSummary {
    RoomSummary { room_id: "r1".into(), name: "r1".into(), capacity: 10, user_count: 1, created_at: 1, rec: false, record: false }
}

#[test]
fn route_material_only_on_the_owner() {
    let m = Material::RoomCreated(summary());
    let n = m.notice("r1", "e1");
    assert_eq!(route(&n, true), Route::Build(m.clone()), "정:RULE-BUS-EVENTS hub 만 읽는 갈래 — 방을 맡은 hub 는 재료로 사실을 짓는다");
    assert_eq!(route(&n, false), Route::NotOwner, "S-sym-RULE-BUS-PUBLIC-2② — 맡지 않은 hub 는 짓지 않는다(버림)");
    let bad = notice(OP_FACT_MATERIAL, json!({ "type": "room_created", "body": 7 }));
    assert_eq!(route(&bad, true), Route::Malformed, "정:RULE-BUS-EVENTS 봉투의 꼴 — 못 읽는 재료는 bus_malformed");
    let other = notice(0xF002, json!({}));
    assert_eq!(route(&other, true), Route::Deliver(None), "정:RULE-BUS-EVENTS — 구간의 다른 번호는 재료가 아니다(배달 길의 모르는 op 로 센다)");
}

#[test]
fn route_a_plane_notices() {
    let joined = notice(0x0701, json!({ "type": "joined", "user_id": "u1" }));
    assert_eq!(route(&joined, true), Route::Deliver(Some("participant_joined")), "정:RULE-BUS-PUBLIC — 배달하고 맡은 방이면 사실 하나");
    assert_eq!(route(&joined, false), Route::Deliver(None), "S-sym-RULE-BUS-PUBLIC-2② — 맡지 않은 hub 는 배달만");
    for (op, body, want) in [
        (0x0701, json!({ "type": "left" }), Some("participant_left")),
        (0x0702, json!({ "action": "add" }), Some("track_published")),
        (0x0702, json!({ "action": "remove" }), Some("track_unpublished")),
        (0x0703, json!({ "type": "muted" }), Some("track_state")),
        (0x0705, json!({ "type": "permission" }), Some("permission_changed")),
        (0x0704, json!({ "type": "room_closed" }), None),
        (0x0702, json!({ "action": "update" }), None),
        (0x0501, json!({}), None),
    ] {
        assert_eq!(notice_kind(&notice(op, body.clone())), want, "정:RULE-HOOK-FACTS 카탈로그 — A 평면 통지 → 이름 {op:#06x} {body}");
    }
}

fn hook(accounts: &[(&str, &[&str])], max: u32) -> Hook {
    Hook::new("hubA", accounts.iter().map(|(k, f)| (k.to_string(), f.iter().map(|s| s.to_string()).collect())).collect(), max)
}

#[test]
fn fact_envelope_and_id_order() {
    let h = hook(&[], 10);
    let body = json!({ "type": "joined", "user_id": "u1", "version": { "epoch": "e", "seq": 3 } });
    let f = h.build("r1", "participant_joined", body.clone(), 1_764_000_000_000);
    let v = serde_json::to_value(&f).expect("encode");
    let keys: Vec<&str> = v.as_object().expect("obj").keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 5, "S-rule-RULE-HOOK-FACTS-1① — 봉투 칸 다섯: {keys:?}");
    for k in ["fact_id", "type", "t", "room_id", "body"] {
        assert!(v.get(k).is_some(), "S-rule-RULE-HOOK-FACTS-1① — 칸 {k}");
    }
    assert_eq!((v["type"].as_str(), v["t"].as_u64()), (Some("participant_joined"), Some(1_764_000_000_000)), "S-rule-RULE-HOOK-FACTS-1② — type = 카탈로그 이름 · t = 받은 시각");
    assert_eq!(v["body"], body, "S-rule-RULE-HOOK-FACTS-1③ — 본문은 A 평면 그대로");
    let g = h.build("r1", "participant_left", json!({}), 1_764_000_000_000);
    let back = h.build("r1", "participant_left", json!({}), 1_763_999_999_000);
    assert!(f.fact_id < g.fact_id && g.fact_id < back.fact_id, "정:RULE-BUS-PUBLIC 사실 id — 받은 차례로 커진다(같은 ms · 벽시계 역행)");
    assert!(f.fact_id.ends_with("hubA"), "정:RULE-BUS-PUBLIC 사실 id — 짓는 hub 의 기동 신원");
    let m = Material::FloorGranted(FloorTaken { speaker: "u1".into(), sources: None, seq: 5, room: "r1".into(), talk: Some("t9".into()) });
    let fm = h.of_material("r1", &m, 1);
    assert_eq!((fm.kind.as_str(), fm.body["speaker"].as_str(), fm.body["talk"].as_str()), ("floor_granted", Some("u1"), Some("t9")), "S-rule-RULE-HOOK-FACTS-1④ — 발언권은 MBCP 필드 이름 그대로 JSON");
    let fi = h.of_material("r1", &Material::FloorReleased(FloorIdle { prev: Some("u1".into()), seq: 6, room: "r1".into() }), 2);
    assert!(fi.body.get("talk").is_none(), "정:RULE-HOOK-FACTS — 비움 사실에 발언 id 없음");
}

#[test]
fn account_queues() {
    let h = hook(&[("floor", &["floor_granted"]), ("all", &["*"])], 2);
    let f1 = h.build("r1", "participant_joined", json!({}), 1);
    assert_eq!(h.offer(f1), 1, "S-rule-RULE-HOOK-FACTS-SUB-2①② — 이름 목록 계정은 그 이름만 ·\"*\" 계정은 모두");
    for t in 2..5 {
        h.offer(h.build("r1", "floor_granted", json!({}), t));
    }
    assert_eq!(h.queued("floor"), 2);
    assert_eq!(h.queue_full.load(Ordering::Relaxed), 3, "S-rule-RULE-HOOK-WEBHOOK-5③ — 상한이면 오래된 것부터 버리고 fact_queue_full 을 센다");
    let b = h.take("all", 65_536).expect("bundle");
    assert_eq!((b.facts.len(), b.dropped), (2, Some(2)), "S-rule-RULE-HOOK-FACTS-SUB-5④ — 다음 묶음 dropped = 직전 묶음 뒤 버린 수");
    assert_eq!(h.take("all", 65_536), None, "정:RULE-BUS-PUBLIC 계정 큐 — 줄도 버림도 없으면 묶음 없음");
    h.lost("all", &b);
    assert_eq!(h.given_up.load(Ordering::Relaxed), 2, "정:RULE-OPS-DROPS fact_given_up — 끝내 못 보낸 사실을 센다");
    let again = h.take("all", 65_536).expect("empty bundle");
    assert_eq!((again.facts.len(), again.dropped), (0, Some(4)), "S-rule-RULE-HOOK-WEBHOOK-4④ — 버린 묶음의 사실 수와 실었던 dropped 를 되돌려 다음 묶음이 알린다");
    assert_eq!(h.take("nobody", 65_536), None, "형상 — 계정 큐가 없는 키");
}

async fn bus() -> std::sync::Arc<Bus> {
    let (b, _rx) = Bus::open("n1", "e1", 1_000, BusCfg { listen: vec!["127.0.0.1:0".into()], advertise: vec![], connect: vec![], namespace: "hookt".into() })
        .await
        .expect("bus");
    b.declare_self().await.expect("self");
    b
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_interest_and_strict_list() {
    let b = bus().await;
    assert!(b.own_room("r1").await, "정:RULE-BUS-EVENTS 방을 맡은 hub 의 늘 관심 — 선언");
    assert!(!b.own_room("r1").await, "형상 — 같은 방을 두 번 맡지 않는다");
    assert!(b.owns("r1") && b.interested("r1") == 1, "정:RULE-BUS-EVENTS — 늘 관심은 한 몫으로 센다");
    b.interest_room("r1", 1).await;
    b.disown_room("r1").await;
    assert!(!b.owns("r1") && b.interested("r1") == 1, "정:RULE-BUS-EVENTS — 로컬 멤버 참조계수와 따로 센다");
    let mut env = common::envelope::Envelope::new(common::envelope::CallKind::CreateRoom);
    env.body = json!({ "room_id": "r2", "name": "r2" });
    let r = b.call_local(&env).await;
    assert!(matches!(r, common::envelope::Reply::Fail { .. }), "형상 — 유닛 없는 생성은 실패");
    assert!(!b.owns("r2") && b.interested("r2") == 0, "정:RULE-BUS-EVENTS — 생성 Call 이 실패하면 늘 관심을 거둔다");
    let loose = oxhubd::room::list(&b).await.expect("client list");
    assert_eq!(loose.total, 0, "형상 — 클라 길 목록은 못 닿은 node 를 경고하고 나머지로");
    let strict = oxhubd::hook::query::rooms(&b).await;
    assert_eq!(strict.map_err(|f| f.code), Err(Code::SfuUnavailable.num()), "S-rule-RULE-HOOK-STATUS-1① — 대외 목록은 부분 목록을 내지 않고 그 Call 의 실패 코드 그대로");
    for bad in ["a/b", "x*", ""] {
        let d = oxhubd::hook::query::detail(&b, bad).await;
        assert_eq!(d.map(|_| ()).map_err(|f| f.code), Err(Code::InvalidPayload.num()), "정:RULE-BUS-KEYSPACE 금지 문자 — room_id 를 받는 입구(방 상세)는 INVALID_PAYLOAD · {bad:?}");
        let p = oxhubd::hook::query::permission(&b, bad, "u1", oxsig::msg::Permission::ALL).await;
        assert_eq!(p.map_err(|f| f.code), Err(Code::InvalidPayload.num()), "정:RULE-BUS-KEYSPACE 금지 문자 — room_id 를 받는 입구(권한 제어)는 INVALID_PAYLOAD · {bad:?}");
    }
    let d = oxhubd::hook::query::detail(&b, "nowhere").await;
    assert_eq!(d.map(|_| ()).map_err(|f| f.code), Err(Code::RoomNotFound.num()), "S-rule-RULE-HOOK-STATUS-5② — 없는 방은 ROOM_NOT_FOUND");
    b.declare_facts("acme", "c1").await.expect("facts token");
    assert_eq!(b.facts_declared(), 1);
    b.close().await;
    assert_eq!(b.facts_declared(), 0, "정:RULE-BUS-PRESENCE — 닫을 때 사실 구독 연결 토큰도 놓는다(ext 와 같은 꼴)");
}

#[test]
fn every_hub_follows_facts_tokens() {
    let main = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs")).expect("main.rs");
    let at = main.find("bus.follow_facts().await").expect("stand check — main.rs follows int/facts");
    let line_start = main[..at].rfind('\n').map_or(0, |i| i + 1);
    assert!(
        main[line_start..at].trim_start().starts_with("if let Err(e) = "),
        "정:RULE-BUS-PUBLIC zenoh 넘기기 · 정:RULE-BUS-KEYSPACE int/facts — 모든 hub 가 조건 없이 사실 구독 토큰을 구독한다: {}",
        &main[line_start..at + 30]
    );
}
