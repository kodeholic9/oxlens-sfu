// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-PUB-ADD · 정:IMPL-PUB-ACCEPT · model: claude-opus-5-5
//! 1층 제어 층 — 업링크 스트림(물리)은 등록된 트랙(논리)에만 붙는다: 트랙이 없는 SSRC 로 온 첫 RTP 는 물리를 만들지 않고 버려지며(`rtp_unknown_ssrc`), 시뮬캐스트는 `PUBLISH_TRACKS` 로 트랙이 선 뒤 첫 RTP(rid)에서야 물리 하나가 그 트랙 밑에 붙는다 · 고아 물리 0(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use oxsfud::fwd::Rtp;
use oxsfud::link::{ExtIds, Transport};
use oxsfud::r#pub::{PubStream, learn};
use serde_json::json;

use support::*;

const MID: u8 = 9;
const RID: u8 = 10;

fn rtp_with(tr: &Arc<Transport>, ssrc: u32, mid: &str, rid: &str) -> Option<Arc<oxsfud::r#pub::PubTrack>> {
    let mut ext = vec![(MID << 4) | (mid.len() as u8 - 1)];
    ext.extend_from_slice(mid.as_bytes());
    ext.push((RID << 4) | (rid.len() as u8 - 1));
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
    learn(tr, &rtp)
}

fn indexed(tr: &Transport, ssrc: u32) -> bool {
    tr.ingress.load().contains_key(&ssrc)
}

fn registered(user: &str, st: &Arc<PubStream>) -> bool {
    let s = sfu();
    let in_ctx = s.pubs.get(user).is_some_and(|c| c.streams().iter().any(|x| Arc::ptr_eq(x, st)));
    let in_set = s.tracks.lock().map(|g| g.get(&st.track_id).is_some_and(|x| Arc::ptr_eq(x, st))).unwrap_or(false);
    in_ctx && in_set
}

#[test]
fn an_uplink_stream_only_ever_hangs_under_a_registered_track() {
    let _g = serial();
    let r = room(&tag("plf"));
    let u = subscriber(&tag("plf-u"));
    enter(&u.p, &r);
    oxsfud::aff::set_pub_room(&u.p, Some(&r.id));
    let tr = u.p.publish.clone();
    tr.ext_ids.store(Arc::new(ExtIds { mid: MID, rid: RID, ..ExtIds::default() }));
    let (early, sim) = (0x6b6b_0001u32, 0x6b6b_0011u32);
    assert!(rtp_with(&tr, early, "1", "h").is_none(), "S-sym-IMPL-PUB-ADD-2 RTP for an SSRC no track claims makes no uplink stream");
    assert!(!indexed(&tr, early), "S-sym-IMPL-PUB-ADD-2 nothing is indexed for that SSRC — no orphan physical");
    let body = json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "rid_extmap_id": RID, "mid_extmap_id": MID, "tracks": [{ "kind": "video", "ssrc": 0, "mid": "1", "pt": 96, "codec": "H264", "fmtp": "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f", "simulcast": true, "duplex": "full" }] });
    let res = oxsfud::ops::media::publish(&ctx_of(&u.p), &body);
    assert!(res.is_ok(), "stand check — the simulcast publish is accepted: {res:?}");
    let st = sfu().tracks.lock().ok().and_then(|g| g.values().find(|s| s.user == u.p.user && s.simulcast).cloned()).expect("stand check — the simulcast track is registered");
    assert!(st.physical().is_empty(), "stand check — a simulcast publish defers its physical to the first RTP");
    assert!(rtp_with(&tr, sim, "7", "h").is_none() && st.physical().is_empty(), "S-sym-IMPL-PUB-ADD-2 RTP whose mid names no simulcast track makes no uplink stream");
    let t = rtp_with(&tr, sim, "1", "h").expect("S-sym-IMPL-PUB-ADD-2 the first RTP of the registered simulcast track attaches its physical");
    assert!(Arc::ptr_eq(&t.stream, &st) && registered(&u.p.user, &t.stream), "S-sym-IMPL-PUB-ADD-2 the new physical hangs under the registered track");
    assert_eq!((st.physical().len(), indexed(&tr, sim)), (1, true), "S-sym-IMPL-PUB-ADD-2 exactly one physical is attached and indexed");
    let ing = tr.ingress.load();
    let orphans: Vec<u32> = ing.iter().filter(|(_, t)| !registered(&u.p.user, &t.stream)).map(|(k, _)| *k).collect();
    assert!(orphans.is_empty(), "S-sym-IMPL-PUB-ADD-2 every indexed physical belongs to a registered track: {orphans:x?}");
    let lines = product_lines();
    let rx: Vec<&Src> = lines.iter().filter(|l| l.file == "fwd/mod.rs" && l.text.contains("crate::r#pub::learn(tr, &rtp)")).collect();
    assert_eq!(rx.len(), 1, "S-sym-IMPL-PUB-ADD-2 the ingress path learns an unknown SSRC in one place only: {:?}", rx.iter().map(|l| at(l)).collect::<Vec<_>>());
    let after = lines.iter().skip_while(|l| !(l.file == "fwd/mod.rs" && l.line == rx[0].line)).take(6).any(|l| l.text.contains("drop_count(\"rtp_unknown_ssrc\")"));
    assert!(after, "S-sym-IMPL-PUB-ADD-2 an SSRC that learn does not claim is dropped as rtp_unknown_ssrc");
}
