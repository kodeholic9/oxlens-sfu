// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-ROOM-LIFE · 정:RULE-BUS-PRESENCE · 정:IMPL-OPS-DESTROY-REAP · model: claude-opus-5-5
//! 1층 제어 층 — 전수 검사 셋째 묶음(context 202611/20261115b)의 버스 몫. 프로세스 안 라우터(듣는 주소 127.0.0.1:0)에 sfud 버스를 붙이고 `room` 토큰을 라우터 쪽 구독으로 본다: 방 만들기가 선언하고 · 지우기가 철회하고 · 같은 id 로 다시 만들면 다시 서고 · 재선언은 방 집합의 방 전부(선언이 빠진 방 포함)를 다시 선언한다. 버스는 프로세스에 하나라 이 파일에는 버스를 쓰는 시험만 둔다. 기다림은 구독의 다음 사건을 받는 것이고 시한(`WAIT`)은 상한일 뿐이다.

mod support;

use std::sync::{Arc, OnceLock};

use common::envelope::CallKind;
use common::keyspace::Keys;
use oxmesh::{Mesh, TokenEvent, TokenSub};
use oxsfud::ops;
use oxsfud::room::Room;
use serde_json::json;

use support::*;

const NS: &str = "l1";

static ROUTER: OnceLock<Mesh> = OnceLock::new();

fn router() -> &'static Mesh {
    ROUTER.get_or_init(|| {
        let s = sfu();
        drive(async {
            let mut cfg = oxmesh::Config::router("l1-router");
            cfg.listen = vec!["127.0.0.1:0".into()];
            let mesh = Mesh::open(cfg).await.expect("in-process router");
            let at = mesh.bound().first().expect("router bound address").to_string();
            oxsfud::bus::Bus::open(&s.node, &s.epoch, NS, &at).await.expect("sfud bus on the in-process router");
            mesh
        })
    })
}

fn key(id: &str) -> String {
    let s = sfu();
    Keys::new(NS).room_token(id, &s.node, &s.epoch)
}

fn subscribe() -> TokenSub {
    let m = router();
    drive(m.token_sub(&Keys::new(NS).room_all(), false)).expect("token subscription")
}

async fn seen(sub: &mut TokenSub, key: &str, put: bool) -> bool {
    loop {
        match tokio::time::timeout(WAIT, sub.recv()).await {
            Ok(Some(TokenEvent::Put(k))) if put && k == key => return true,
            Ok(Some(TokenEvent::Delete(k))) if !put && k == key => return true,
            Ok(Some(_)) => continue,
            _ => return false,
        }
    }
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    let res = drive(ops::rooms::create(&e));
    assert!(res.is_ok(), "stand check — create {id}: {res:?}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn destroy(r: &Arc<Room>) {
    drive(ops::rooms::destroy_room(r, "SV-IMPL-ROOM-LIFE", "l1"));
}

fn holds_token(r: &Room) -> bool {
    r.st.lock().map(|s| s.token.is_some()).unwrap_or(false)
}

#[test]
fn t610_3_make_destroy_remake_puts_withdraws_and_puts_the_room_token() {
    let _g = serial();
    router();
    let id = tag("t6103");
    let k = key(&id);
    let mut sub = subscribe();
    let a = create(&id);
    assert!(drive(seen(&mut sub, &k, true)), "stand check — making the room puts its room token");
    assert!(holds_token(&a), "정:IMPL-ROOM-LIFE the room holds its own room token");
    destroy(&a);
    assert!(!holds_token(&a), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 a removed room holds no room token");
    assert!(drive(seen(&mut sub, &k, false)), "정:IMPL-ROOM-LIFE 정:IMPL-OPS-DESTROY-REAP S-sym-IMPL-ROOM-LIFE-5 removing the room withdraws its room token");
    let b = create(&id);
    assert!(!Arc::ptr_eq(&a, &b), "stand check — the second make is a new room");
    assert!(
        drive(seen(&mut sub, &k, true)),
        "정:IMPL-ROOM-LIFE 정:IMPL-OPS-DESTROY-REAP S-sym-IMPL-ROOM-LIFE-5 making the same id again puts the room token again"
    );
    destroy(&a);
    assert!(holds_token(&b), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 a late removal of the old room leaves the new room's token");
    destroy(&b);
    assert!(drive(seen(&mut sub, &k, false)), "정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 removing the new room withdraws its token");
}

#[test]
fn t610_4_redeclare_declares_every_room_in_the_room_set() {
    let _g = serial();
    router();
    let missed = room(&tag("t6104-missed"));
    let held = create(&tag("t6104-held"));
    assert!(!holds_token(&missed) && holds_token(&held), "stand check — one room never got its token (a failed declare) and one holds it");
    let mut sub = subscribe();
    drive(ops::rooms::redeclare());
    let (km, kh) = (key(&missed.id), key(&held.id));
    let got_missed = drive(seen(&mut sub, &km, true));
    assert!(
        got_missed && holds_token(&missed),
        "정:RULE-BUS-PRESENCE 정:IMPL-ROOM-LIFE S-sym-IMPL-ROOM-LIFE-5 redeclaring takes the rooms from the room set — a room whose declare had failed is declared too"
    );
    assert!(holds_token(&held), "정:RULE-BUS-PRESENCE a room that held its token holds the redeclared one");
    destroy(&missed);
    destroy(&held);
    let gone = drive(async { seen(&mut sub, &km, false).await && seen(&mut sub, &kh, false).await });
    assert!(gone, "stand check — removing both rooms withdraws both tokens");
}
