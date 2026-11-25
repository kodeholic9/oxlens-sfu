// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-SIM-MODES · 정:IMPL-PUB-RID-SPATIAL · model: claude-opus-5-5
//! 1층 제어 층 — `auto_layer = off`(기본)가 끄는 것은 자동 판단뿐이다: 그 모드에서도 시뮬캐스트 트랙 항목은 `scalability` 를 싣고, `SUBSCRIBE_LAYER` 는 구독 줄의 수동 상한 · 멈춤을 그대로 바꾼다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsfud::fwd::Rtp;
use oxsfud::link::{ExtIds, Transport};
use oxsfud::r#pub::learn;
use oxsig::msg::Kind;
use serde_json::json;

use support::*;

const MID: u8 = 9;
const RID: u8 = 10;

fn first_rtp(tr: &Arc<Transport>, ssrc: u32, rid: &str) {
    let mut ext = vec![MID << 4, b'1', (RID << 4) | (rid.len() as u8 - 1)];
    ext.extend_from_slice(rid.as_bytes());
    while ext.len() % 4 != 0 {
        ext.push(0);
    }
    let mut raw = vec![0x90, 96, 0, 1, 0, 0, 0, 0];
    raw.extend_from_slice(&ssrc.to_be_bytes());
    raw.extend_from_slice(&[0xbe, 0xde]);
    raw.extend_from_slice(&((ext.len() / 4) as u16).to_be_bytes());
    let hlen = raw.len() + ext.len();
    raw.extend_from_slice(&ext);
    raw.extend_from_slice(&[0u8; 20]);
    let rtp = Rtp { raw: &raw, pt: 96, marker: false, seq: 1, ts: 0, ssrc, hlen, ext: Some((0xbede, &raw[16..hlen])) };
    assert!(learn(tr, &rtp).is_some(), "stand check — the first RTP of rid {rid} attaches its physical");
}

#[test]
fn with_auto_layer_off_the_manual_layer_and_scalability_still_work() {
    let _g = serial();
    assert_eq!(oxsfud::sim::mode(), oxsfud::sim::Mode::Off, "stand check — the default policy runs auto_layer = off");
    assert!(!oxsfud::ops::auto_layer_on(), "stand check — the automatic decision is off");
    let r = room(&tag("smo"));
    let (u, b) = (subscriber(&tag("smo-u")), subscriber(&tag("smo-b")));
    enter(&u.p, &r);
    enter(&b.p, &r);
    oxsfud::aff::set_pub_room(&u.p, Some(&r.id));
    let tr = u.p.publish.clone();
    tr.ext_ids.store(Arc::new(ExtIds { mid: MID, rid: RID, ..ExtIds::default() }));
    let body = json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "rid_extmap_id": RID, "mid_extmap_id": MID, "tracks": [{ "kind": "video", "ssrc": 0, "mid": "1", "pt": 96, "codec": "H264", "fmtp": "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f", "simulcast": true, "duplex": "full" }] });
    let res = oxsfud::ops::media::publish(&ctx_of(&u.p), &body);
    assert!(res.is_ok(), "stand check — the simulcast publish is accepted: {res:?}");
    let st = sfu().pubs.get(&u.p.user).and_then(|c| c.streams().into_iter().find(|s| s.simulcast)).expect("stand check — the simulcast track");
    first_rtp(&tr, 0x7c7c_0001, "h");
    first_rtp(&tr, 0x7c7c_0002, "l");
    oxsfud::ops::media::announce_add(&st);
    let entry = oxsfud::trk::entry(&st);
    assert_eq!(entry.scalability.as_deref(), Some(oxsfud::trk::SCALABILITY), "S-sym-RULE-SIM-MODES-1 with auto_layer off the simulcast track entry still carries scalability");
    let row = row_of(&b.t, Kind::Video, &st.track_id);
    let ask = |v: serde_json::Value| {
        let body = json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "targets": [v] });
        let res = oxsfud::ops::media::subscribe_layer(&ctx_of(&b.p), &body);
        assert!(res.is_ok(), "stand check — SUBSCRIBE_LAYER is answered: {res:?}");
    };
    ask(json!({ "track_id": st.track_id, "spatial": 0 }));
    assert_eq!(row.cap.load(Ordering::Acquire), 0, "S-sym-RULE-SIM-MODES-1 with auto_layer off SUBSCRIBE_LAYER sets the row's manual cap (l)");
    assert_eq!(oxsfud::sim::want(&row), 0, "S-sym-RULE-SIM-MODES-1 the wanted layer follows the manual cap");
    ask(json!({ "track_id": st.track_id, "spatial": 9 }));
    assert_eq!(row.cap.load(Ordering::Acquire), oxsfud::fwd::LAYER_TOP, "S-sym-RULE-SIM-MODES-1 a cap above the top is cut to the top, not refused");
    assert!(holds(&b.t, &st.track_id, &r.id), "stand check — b's table holds the track before the pause");
    ask(json!({ "track_id": st.track_id, "paused": true }));
    assert!(!holds(&b.t, &st.track_id, &r.id), "S-sym-RULE-SIM-MODES-1 with auto_layer off SUBSCRIBE_LAYER pauses the track — b's table stops carrying it");
    ask(json!({ "track_id": st.track_id, "paused": false }));
    assert!(holds(&b.t, &st.track_id, &r.id), "S-sym-RULE-SIM-MODES-1 and resumes it");
}
