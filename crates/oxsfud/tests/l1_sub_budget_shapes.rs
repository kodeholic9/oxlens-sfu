// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-SUB-TABLE · 연:ROOM.ledger.BundleCap · 연:COND-ROOM-BUDGET · 연:R-ROOM-WORST-SHAPE · model: claude-sonnet-5-5
//! 1층 구독 예산 형상 — 슬롯 방(무전) 20개를 들은 채 개인 트랙이 늘어도 구독 줄 수가 슬롯 2×방 + 개인 수이고 연결 몫(`1pc` 96 · `2pc` 128)을 안 넘으며, `1pc` 에서 통화 인원이 늘어도 전이중 줄은 상한(audio 25 · video 25)에서 멈춘다(context 202611/20261124b 클라 연동 갈래).

mod support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use common::envelope::{CallKind, Claims, Home};
use oxsfud::link::{Peer, Role, Transport};
use oxsfud::ops;
use oxsfud::room::Room;
use oxsfud::sub::{Table, FULL_AUDIO, FULL_VIDEO, SUB_1PC, SUB_2PC};
use oxsig::msg::{Duplex, Kind, PcMode};
use serde_json::{Value, json};

use support::*;

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn join(u: &Sub, r: &Room) -> Result<Value, oxsig::Failure> {
    ops::rooms::join(&ctx_of(&u.p), &json!({ "room_id": r.id }))
}

fn subscriber_one(user: &str) -> Sub {
    let s = sfu();
    let session = format!("ses-{user}");
    let publish = Transport::new(s.next_id(), user, &session, Role::Publish, 0, false);
    let subscribe = Transport::new(s.next_id(), user, &session, Role::Subscribe, 0, false);
    let t = Table::new(subscribe.clone(), PcMode::One, s.policy.media.slot_mid_reserve as usize);
    t.mark_seeded();
    s.tables.insert(t.clone());
    let p = Arc::new(Peer {
        user: user.into(),
        sid: format!("sid-{user}"),
        session,
        home: Mutex::new(Home { node: s.node.clone(), inst: String::new() }),
        pc_mode: PcMode::One,
        claims: Claims::default(),
        publish,
        subscribe,
        rooms: Mutex::new(BTreeMap::new()),
        created_at: common::now_ms(),
        ever_sent_join: AtomicBool::new(false),
    });
    s.peers.lock().expect("peers").insert(user.into(), p.clone());
    Sub { p, t }
}

fn add_callers(u: &Sub, room: &Room, n: usize, prefix: &str) {
    for i in 0..n {
        let c = subscriber(&tag(&format!("{prefix}{i}")));
        enter(&c.p, room);
        let (a, _) = published(&c.p, &room.id, Kind::Audio, Duplex::Full);
        list(&a, true);
        let (v, _) = published(&c.p, &room.id, Kind::Video, Duplex::Full);
        list(&v, true);
    }
    ops::media::resync_peer(&u.p, false);
}

fn slot_listener(u: &Sub, tagp: &str) -> Vec<Arc<Room>> {
    let rooms: Vec<Arc<Room>> = (0..20).map(|i| create(&tag(&format!("{tagp}{i}")))).collect();
    for (i, r) in rooms.iter().enumerate() {
        let res = join(u, r);
        assert!(res.is_ok(), "stand check — slot room {} of 20 joins: {res:?}", i + 1);
    }
    assert_eq!(u.t.rows().len(), 40, "stand check — twenty slot rooms load forty slot rows");
    rooms
}

#[test]
fn rows_are_two_per_slot_room_plus_individuals_and_never_pass_the_connection_share() {
    let _g = serial();
    for (mode, user, limit) in [("2pc", "bsh-2", SUB_2PC), ("1pc", "bsh-1", SUB_1PC)] {
        let u = if mode == "1pc" { subscriber_one(&tag(user)) } else { subscriber(&tag(user)) };
        assert_eq!(u.t.limit(), limit, "stand check — the {mode} connection share is the ledger value");
        let rooms = slot_listener(&u, &format!("{user}r"));
        add_callers(&u, &rooms[0], 5, &format!("{user}c"));
        let rows = u.t.rows().len();
        assert_eq!(rows, 2 * 20 + 2 * 5, "C-cond-RULE-DOC-COND-INDEX-6① {mode}: subscription m-lines = 2 × rooms + individuals (20 rooms · 10 individual tracks)");
        assert!(rows <= limit, "C-cond-RULE-DOC-COND-INDEX-6① {mode}: {rows} rows stay within the connection share {limit}");
        add_callers(&u, &rooms[0], 60, &format!("{user}d"));
        let rows = u.t.rows().len();
        assert!(rows <= limit, "C-cond-RULE-DOC-COND-INDEX-6① {mode}: even with sixty more callers {rows} rows stay within the connection share {limit}");
    }
}

#[test]
fn twenty_ptt_rooms_plus_a_fifteen_person_call_load_forty_plus_thirty_rows() {
    let _g = serial();
    let u = subscriber_one(&tag("bsh-w"));
    let rooms = slot_listener(&u, "bshw");
    add_callers(&u, &rooms[0], 15, "bshwc");
    assert_eq!(u.t.rows().len(), 40 + 30, "C-cond-RULE-DOC-COND-INDEX-16① twenty ptt rooms (40 slot rows) plus fifteen callers (30 individual rows) load 70 subscription m-lines with no 4005");
}

#[test]
fn on_1pc_full_duplex_rows_stop_at_the_cap_so_the_total_stays_under_the_connection_share() {
    let _g = serial();
    assert_eq!((FULL_AUDIO, FULL_VIDEO), (25, 25), "stand check — the full-duplex caps are the ledger values");
    let u = subscriber_one(&tag("bsh-c"));
    let rooms = slot_listener(&u, "bshc");
    add_callers(&u, &rooms[0], 15, "bshcs");
    let small = u.t.rows().len();
    add_callers(&u, &rooms[0], 60, "bshcb");
    let full = u.t.rows().len();
    assert!(full > small && full <= 40 + FULL_AUDIO + FULL_VIDEO, "C-cond-RULE-DOC-COND-INDEX-16② 1pc: sixty more callers grow rows from {small} only up to the cap: {full} ≤ 40 + {} + {}", FULL_AUDIO, FULL_VIDEO);
    assert_eq!(full, 40 + FULL_AUDIO + FULL_VIDEO, "C-cond-RULE-DOC-COND-INDEX-16② 1pc: the full-duplex rows stop exactly at audio 25 + video 25 — 40 + 50 = 90");
    assert!(full <= SUB_1PC, "C-cond-RULE-DOC-COND-INDEX-16② 1pc: the total {full} stays within the 1pc connection share {SUB_1PC} — no 4005");
}
