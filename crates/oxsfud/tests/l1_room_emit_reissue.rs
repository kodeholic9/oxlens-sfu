// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-EMIT · 정:IMPL-SUB-TABLE · 연:RULE-ROOM-NOTIFY-VERSION-FOUR · model: claude-opus-5-5
//! 1층 제어 층 · 소스 가드 — mid 고갈이 풀려 그 사람의 줄이 새로 서면 그것은 그 연결의 배정표 변화일 뿐이다: 그 사람의 표만 `arev` 가 한 번 오르고(그 표 하나가 `MLINE_EVENT` 로 간다) 방 `seq` 는 그대로이며 다른 사람의 표는 안 바뀐다 — 방 통지(`TRACK_EVENT`)를 내지 않는다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::Arc;

use oxsfud::ops;
use oxsfud::r#pub::PubStream;
use oxsig::msg::{Duplex, Kind};
use serde_json::json;

use support::*;

#[test]
fn a_mid_freed_from_exhaustion_moves_only_that_connections_table() {
    let _g = serial();
    let r = room(&tag("rei"));
    let (b, c, u) = (subscriber(&tag("rei-b")), subscriber(&tag("rei-c")), subscriber(&tag("rei-u")));
    for p in [&b.p, &c.p, &u.p] {
        enter(p, &r);
    }
    let limit = b.t.limit();
    let mut streams: Vec<Arc<PubStream>> = Vec::new();
    for _ in 0..limit + 2 {
        let (st, _) = published(&u.p, &r.id, Kind::Audio, Duplex::Full);
        list(&st, true);
        ops::media::announce_add(&st);
        streams.push(st);
    }
    let held: Vec<&Arc<PubStream>> = streams.iter().filter(|s| holds(&b.t, &s.track_id, &r.id)).collect();
    let waiting: Vec<&Arc<PubStream>> = streams.iter().filter(|s| remembers_unplaced(&b.t, &s.track_id)).collect();
    assert!(!held.is_empty() && !waiting.is_empty(), "stand check — b's table is exhausted: {} held · {} waiting (limit {limit})", held.len(), waiting.len());
    let (freed, next) = (held[held.len() - 1].track_id.clone(), waiting.clone());
    let (arev0, seq0, c_arev0) = (b.t.arev(), r.version().seq, c.t.arev());
    let c_rows0: Vec<String> = c.t.loaded().into_iter().map(|(t, _, _)| t).collect();
    let body = json!({ "room_id": r.id, "join_id": format!("j-{}", r.id), "targets": [{ "track_id": freed, "paused": true }] });
    assert!(ops::media::subscribe_layer(&ctx_of(&b.p), &body).is_ok(), "stand check — b pauses one held track");
    assert!(!holds(&b.t, &freed, &r.id), "stand check — the paused track left b's table");
    let placed: Vec<&&Arc<PubStream>> = next.iter().filter(|s| holds(&b.t, &s.track_id, &r.id)).collect();
    assert_eq!(placed.len(), 1, "S-sym-IMPL-ROOM-EMIT-2③ the freed mid is re-issued to exactly one waiting track");
    assert_eq!(b.t.arev(), arev0 + 1, "S-sym-IMPL-ROOM-EMIT-2③ C-cond-RULE-DOC-COND-INDEX-21③ that person's table moves one revision — one MLINE_EVENT (arev+1)");
    assert_eq!(r.version().seq, seq0, "S-sym-IMPL-ROOM-EMIT-2③ C-cond-RULE-DOC-COND-INDEX-21③ the room seq does not move — a re-issue is not a room delta");
    let c_rows1: Vec<String> = c.t.loaded().into_iter().map(|(t, _, _)| t).collect();
    assert_eq!((c.t.arev(), c_rows1), (c_arev0, c_rows0), "S-sym-IMPL-ROOM-EMIT-2③ nobody else's table changes");
    let lines = product_lines();
    let layer: Vec<&Src> = lines.iter().filter(|l| l.file == "ops/media.rs" && l.func == "subscribe_layer").collect();
    assert!(layer.iter().any(|l| l.text.contains("resync_peer(&peer, true)")), "S-sym-IMPL-ROOM-EMIT-2③ the pause resyncs that one connection with its MLINE_EVENT");
    assert!(!layer.iter().any(|l| l.text.contains("track_event(")), "S-sym-IMPL-ROOM-EMIT-2③ C-cond-RULE-DOC-COND-INDEX-21③ the pause path sends no room TRACK_EVENT to the others");
    let settle: Vec<&Src> = lines.iter().filter(|l| l.file == "ops/media.rs" && l.func == "settle").collect();
    assert!(settle.iter().any(|l| l.text.contains("super::user_notice(&p.user, Op::MlineEvent, &body, None);")), "S-sym-IMPL-ROOM-EMIT-2③ a changed table goes to that one person as MLINE_EVENT");
}
