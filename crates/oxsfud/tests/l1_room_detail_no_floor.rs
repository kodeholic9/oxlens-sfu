// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-FLR-SNAPSHOT · 연:RULE-ROOM-DETAIL-NO-FLOOR · model: claude-opus-5-5
//! 1층 제어 층 — 클라 경로의 방 조회(`GET /rooms/{id}?tracks=1` 이 미디어 서버에 묻는 `GetRoom`)에는 발언권을 싣지 않는다: 화자가 발언권을 쥐고 있는 동안에도 응답의 칸은 방 · 명단 · 트랙 · 판번호뿐이고 발언권 이름(화자 · 큐 · 상태 · 우선순위 · 시작 시각)이 어디에도 없다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::envelope::CallKind;
use oxsfud::flr::core::{Msg, Mt};
use oxsfud::ops;
use oxsfud::ops::floor::{self, Entry};
use oxsfud::room::Room;
use serde_json::{Value, json};

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn keys(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.insert(k.clone());
                keys(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

#[test]
fn the_client_room_view_carries_no_floor_even_while_someone_speaks() {
    let _g = serial();
    let r = create(&tag("rdf"));
    let (a, b) = (tag("rdf-a"), tag("rdf-b"));
    for u in [&a, &b] {
        r.join(member(u), None).expect("stand check — registered");
        floor::run(&r, Entry::of("ws_join", u), |f, _| Some(f.ws_join(u, true)));
        floor::run(&r, Entry::of("ws_pub_select", u), |f, _| Some(f.ws_pub_select(u)));
    }
    for (u, p) in [(&a, 5), (&b, 3)] {
        let m = Msg::new(Mt::Request, None, p, 0);
        floor::run(&r, Entry::of("server_rx", u), |f, t2| Some(f.server_rx(u, &m, true, t2)));
    }
    let snap = floor::snapshot(&r);
    assert!(snap["speaker"].as_str() == Some(a.as_str()) && snap["queue"].as_array().is_some_and(|q| q.len() == 1), "stand check — a speaks and b waits: {snap}");
    let mut env = envelope(CallKind::GetRoom, Some(&a), None);
    env.body = json!({ "room_id": r.id, "tracks": true, "me": true });
    let got = ops::rooms::get(&env).expect("stand check — the room view answers");
    let mut all = BTreeSet::new();
    keys(&got, &mut all);
    let top: BTreeSet<String> = got.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let want: BTreeSet<String> = ["room_id", "name", "capacity", "user_count", "participants", "me", "created_at", "record", "media_codecs", "version", "tracks"].into_iter().map(String::from).collect();
    assert_eq!(top, want, "S-sym-RULE-FLR-SNAPSHOT-2① the client room view has room · roster · tracks · version only");
    let floor_names = ["speaker", "queue", "floor", "priority", "first_rtp_at", "last_rtp_at", "timers", "state"];
    let leaked: Vec<&str> = floor_names.iter().copied().filter(|n| all.contains(*n)).collect();
    assert!(leaked.is_empty(), "S-sym-RULE-FLR-SNAPSHOT-2① no floor name anywhere in the client room view while a speaks: {leaked:?}");
}
