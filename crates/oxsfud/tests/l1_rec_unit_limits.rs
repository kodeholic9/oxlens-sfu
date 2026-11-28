// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 연:RULE-ROOM-MAX-ROOMS · 연:RULE-ROOM-BUDGET-SPLIT · 연:RULE-ROOM-JOIN-NO-TYPE-CHECK · model: claude-opus-5-5
//! 1층 제어 층 — 녹화 참가자(`participant_type 1` · 투명)도 입장 한도는 보통 참가자와 같다: 20 방까지 들고 21 번째는 `4004` · 표의 슬롯 몫 · 구독 몫은 정책 · 장부 값 그대로다(녹음 유닛은 방마다 세션이라 예외가 없다 — 유닛 표지 · 250 · 500 · 550 은 걷었다 · context 202611/20261128a 결정 1 · B-1).

mod support;

use std::sync::{Arc, Once};

use common::envelope::{CallKind, Claims};
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::ops;
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::rooms::MAX_ROOMS;
use oxsfud::room::Room;
use oxsfud::sub::SUB_2PC;
use oxsig::msg::PcMode;
use serde_json::{Value, json};

use support::*;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn recorder(user: &str) -> Ctx {
    let claims = Claims { participant_type: 1, hidden: true, metadata: None, permission: None };
    Ctx { user: user.into(), sid: format!("sid-{user}"), session: format!("ses-{user}"), pc_mode: PcMode::Two, source_tag: false, home: None, claims: Some(claims), over: None }
}

fn join(c: &Ctx, r: &Room) -> Result<Value, oxsig::Failure> {
    ops::rooms::join(c, &json!({ "room_id": r.id }))
}

#[test]
fn a_recorder_gets_the_same_room_limit_as_any_participant() {
    let _g = serial();
    media();
    let c = recorder(&tag("pt1-rec"));
    let rooms: Vec<Arc<Room>> = (0..=MAX_ROOMS).map(|i| create(&tag(&format!("pt1-r{i}")))).collect();
    for r in rooms.iter().take(MAX_ROOMS) {
        assert!(join(&c, r).is_ok(), "stand check — twenty rooms join inside the limits");
    }
    let f = join(&c, &rooms[MAX_ROOMS]).expect_err("연:RULE-ROOM-MAX-ROOMS a participant_type 1 · hidden recorder is refused at the 21st room");
    assert_eq!(f.code, 4004, "연:RULE-ROOM-JOIN-NO-TYPE-CHECK a recorder (kind 1 · hidden) gets 4004 at 20 like anyone — kind and hidden do not pick the limits: {f:?}");
    assert_eq!(f.details, Some(json!({ "limit": MAX_ROOMS, "current": MAX_ROOMS })), "연:RULE-ROOM-MAX-ROOMS details are counts against the one room limit: {f:?}");
    let p = sfu().peer(&c.user).expect("stand check — the recorder's Peer");
    let t = sfu().table(&p).expect("stand check — the recorder's table");
    assert_eq!((t.slot_budget, t.limit()), (sfu().policy.media.slot_mid_reserve as usize, SUB_2PC), "연:RULE-ROOM-BUDGET-SPLIT a recorder's slot share and subscribe share are the policy and ledger values");
}
