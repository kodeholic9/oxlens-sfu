// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-PUB-ADD · 정:RULE-PUB-STATE · model: claude-opus-5-5
//! 1층 제어 층 · 소스 가드 — 발행 상태는 Created → Intended(`PUBLISH_TRACKS` 가 받아들임) → Active(첫 RTP)로만 간다 · 트랙(논리)을 먼저 만들고 업링크 스트림(물리)을 나중에 붙여 물리마다 제 트랙이 있다(고아 물리 0)(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use oxsfud::r#pub::PublishState;
use serde_json::json;

use support::*;

#[test]
fn a_publish_goes_created_intended_active_and_leaves_no_orphan_physical() {
    let _g = serial();
    let r = room(&tag("pst"));
    let u = subscriber(&tag("pst-u"));
    enter(&u.p, &r);
    oxsfud::aff::set_pub_room(&u.p, Some(&r.id));
    let body = json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "tracks": [{ "kind": "audio", "ssrc": 0x5a5a_0001u32, "mid": "0", "pt": 111, "duplex": "full" }] });
    let res = oxsfud::ops::media::publish(&ctx_of(&u.p), &body);
    assert!(res.is_ok(), "stand check — PUBLISH_TRACKS add accepted: {res:?}");
    let st = sfu().pubs.get(&u.p.user).and_then(|c| c.by_ssrc(0x5a5a_0001)).expect("stand check — the stream is registered");
    assert_eq!(*st.state.lock().expect("state"), PublishState::Intended, "SV-STATE-PUBLISH-STATE accepting the publish takes the stream to Intended");
    let phys = st.physical();
    assert_eq!(phys.len(), 1, "stand check — one physical uplink stream for a plain audio publish");
    assert!(phys.iter().all(|t| Arc::ptr_eq(&t.stream, &st)), "SV-STATE-PUBLISH-STATE every physical belongs to its logical track — no orphan physical");
    oxsfud::ops::media::activate(&st);
    assert_eq!(*st.state.lock().expect("state"), PublishState::Active, "SV-STATE-PUBLISH-STATE the first RTP takes the stream to Active");
    let lines = product_lines();
    let publish: Vec<&Src> = lines.iter().filter(|l| l.file == "ops/media.rs" && l.func == "publish").collect();
    let at = |needle: &str| publish.iter().position(|l| l.text.contains(needle)).unwrap_or_else(|| panic!("stand check — publish has {needle}"));
    let order = [at("PubStream::new(StreamInit"), at("set_state(PublishState::Intended)"), at("PubTrack::new(st.clone()")];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "SV-STATE-PUBLISH-STATE the logical track is made (and Intended) before its physical is attached: {order:?}");
}
