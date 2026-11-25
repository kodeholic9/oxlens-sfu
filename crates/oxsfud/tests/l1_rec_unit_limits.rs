// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:IMPL-OPS-REC-UNIT · 연:RULE-ROOM-MAX-ROOMS · 연:RULE-ROOM-BUDGET-SPLIT · 연:COND-ROOM-BUDGET · 연:RULE-ROOM-JOIN-NO-TYPE-CHECK · 연:RULE-SES-TOKEN-CLAIMS · model: claude-opus-5-5
//! 1층 제어 층 — 유닛 표지(`claims.unit` — 감독자만 서명)가 있는 참가자의 방 수 상한은 250 이다(표 슬롯 몫 500 · 줄 상한 550): 250 방까지 들고 251 번째는 그 방만 `4004 {250,250}` · 이미 든 방의 줄은 그대로 · 250 방 표 직렬화와 입장 응답이 WS 프레임 1 MiB 의 절반 아래(`room_id` 16 바이트) · 같은 종류(`participant_type 1` · 투명)여도 표지가 없으면 20 방 · 정책 몫 그대로다(context 202611/20261124b 이슈 I2 · 부장님 결재 20261126 — 1,000 방은 RESUME 응답이 1 MiB 에 닿아 250 으로 낮춤).

mod support;

use std::sync::{Arc, Once};

use common::envelope::{CallKind, Claims};
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::ops;
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::rooms::UNIT_MAX_ROOMS;
use oxsfud::room::Room;
use oxsfud::sub::SUB_2PC;
use oxsig::msg::PcMode;
use serde_json::{Value, json};

use support::*;

static MEDIA: Once = Once::new();

const HALF_FRAME: usize = oxsig::frame::MAX_FRAME_BYTES / 2;
const ROOM_ID_BYTES: usize = 16;

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up (a new session needs transports)");
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn padded(base: &str) -> String {
    format!("{base}{}", "r".repeat(ROOM_ID_BYTES.saturating_sub(base.len())))
}

fn recorder(user: &str, unit: bool) -> Ctx {
    let claims = Claims { participant_type: 1, hidden: true, metadata: None, permission: None, unit };
    Ctx { user: user.into(), sid: format!("sid-{user}"), session: format!("ses-{user}"), pc_mode: PcMode::Two, source_tag: false, home: None, claims: Some(claims), over: None }
}

fn join(c: &Ctx, r: &Room) -> Result<Value, oxsig::Failure> {
    ops::rooms::join(c, &json!({ "room_id": r.id }))
}

#[test]
fn a_unit_marked_recorder_holds_two_hundred_fifty_rooms_and_the_next_one_alone_is_listen_limit() {
    let _g = serial();
    media();
    assert_eq!(UNIT_MAX_ROOMS, 250, "stand check — the unit room limit is 250 (decision 20261126)");
    let c = recorder(&tag("unit-rec"), true);
    let stamp = tag("u");
    let rooms: Vec<Arc<Room>> = (0..=UNIT_MAX_ROOMS).map(|i| create(&padded(&format!("{stamp}-{i:04}-")))).collect();
    assert!(join(&c, &rooms[0]).is_ok(), "stand check — the first room makes the unit's Peer");
    let p = sfu().peer(&c.user).expect("stand check — the recorder's Peer");
    let t = sfu().table(&p).expect("stand check — the recorder's table");
    assert_eq!((t.slot_budget, t.limit()), (500, 550), "정:IMPL-OPS-REC-UNIT I2 the unit table — slot share 2 × 250 · row ceiling 500 + 25 + 25");
    if let Ok(mut g) = p.rooms.lock() {
        for r in &rooms[1..UNIT_MAX_ROOMS - 2] {
            g.insert(r.id.clone(), format!("j-{}", r.id));
        }
    }
    ops::media::resync_peer(&p, false);
    let mut reply = 0;
    for r in &rooms[UNIT_MAX_ROOMS - 2..UNIT_MAX_ROOMS] {
        let got = join(&c, r);
        assert!(got.is_ok(), "정:IMPL-OPS-REC-UNIT I2 the unit-marked recorder joins up to room 250 — no 4004 past 20 · no 4005 past the slot share 40 or the subscribe share {SUB_2PC}: {:?}", got.as_ref().err());
        reply = got.map(|v| serde_json::to_vec(&v).map(|b| b.len()).unwrap_or(0)).unwrap_or(0);
    }
    assert_eq!(p.rooms().len(), UNIT_MAX_ROOMS, "stand check — the unit is in 250 rooms");
    assert_eq!(t.loaded().len(), 2 * UNIT_MAX_ROOMS, "정:IMPL-OPS-REC-UNIT I2 two loaded slot rows per room — 500");
    let arev = t.arev();
    let f = join(&c, &rooms[UNIT_MAX_ROOMS]).expect_err("연:RULE-ROOM-MAX-ROOMS I2 the 251st room is refused");
    assert_eq!(f.code, 4004, "연:RULE-ROOM-MAX-ROOMS I2 the 251st room is LISTEN_LIMIT: {f:?}");
    assert_eq!(f.details, Some(json!({ "limit": 250, "current": 250 })), "연:RULE-ROOM-MAX-ROOMS I2 details are counts against the unit limit 250: {f:?}");
    assert!(!p.in_room(&rooms[UNIT_MAX_ROOMS].id) && !rooms[UNIT_MAX_ROOMS].has(&c.user), "연:RULE-ROOM-MAX-ROOMS I2 only that room is refused — the unit is not in it");
    assert_eq!((p.rooms().len(), t.loaded().len(), t.arev()), (UNIT_MAX_ROOMS, 2 * UNIT_MAX_ROOMS, arev), "연:RULE-ROOM-MAX-ROOMS I2 the rooms already held keep recording — 250 rooms · 500 loaded rows · the table did not change");
    let bytes = serde_json::to_vec(&t.snapshot("node-a")).expect("json").len();
    println!("I2 unit table of {} rows (room_id {ROOM_ID_BYTES} bytes) serializes to {bytes} bytes · the 250th join reply {reply} bytes — half frame {HALF_FRAME}", t.rows().len());
    assert!(bytes < HALF_FRAME && reply < HALF_FRAME, "연:COND-ROOM-BUDGET I2 the 250-room unit table ({bytes} bytes) and the 250th join reply ({reply} bytes) stay under half the WS frame cap {HALF_FRAME} with room_id {ROOM_ID_BYTES} bytes");
}

#[test]
fn the_same_recorder_kind_without_the_unit_mark_keeps_the_room_limit() {
    let _g = serial();
    media();
    let c = recorder(&tag("pt1-rec"), false);
    let rooms: Vec<Arc<Room>> = (0..21).map(|i| create(&tag(&format!("pt1-r{i}")))).collect();
    for r in rooms.iter().take(20) {
        assert!(join(&c, r).is_ok(), "stand check — twenty rooms join inside the limits");
    }
    let f = join(&c, &rooms[20]).expect_err("연:RULE-ROOM-MAX-ROOMS I2 a participant_type 1 · hidden token without the unit mark is refused at the 21st room");
    assert_eq!(f.code, 4004, "연:RULE-ROOM-MAX-ROOMS I2 a browser recorder (same kind · hidden · no unit mark) still gets 4004 at 20 — kind and hidden do not pick the limits: {f:?}");
    let p = sfu().peer(&c.user).expect("stand check — the recorder's Peer");
    let t = sfu().table(&p).expect("stand check — the recorder's table");
    assert_eq!((t.slot_budget, t.limit()), (sfu().policy.media.slot_mid_reserve as usize, SUB_2PC), "연:RULE-ROOM-BUDGET-SPLIT I2 without the unit mark the slot share and the subscribe share are the policy and ledger values");
}
