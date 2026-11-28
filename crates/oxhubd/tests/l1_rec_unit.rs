// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-SES-TOKEN-ISSUE · 정:IMPL-ROOM-LIFE · 정:RULE-OPS-SURFACE · 정:IMPL-CFG-SYSTEM-FILE · 연:ROOM.http.CREATE_ROOM · model: claude-opus-5-5
//! 1층 녹음 유닛의 감독 — `role = "rec"` 유닛은 `/healthz/ready` 에 세지 않는다(미디어 서버는 센다 — 대조) · 슈퍼바이저가 띄우고 멈추는 유닛 목록에는 든다 · 게이트웨이가 node 토큰(`rec-{node}` · 참가자 종류 1 · 투명 · 유닛 열쇠)을 지어 넘긴다 · 방 세션 토큰(`rec-{node}-{room}`)은 유닛 열쇠로 풀린 그 node 의 node 토큰 · 그 node 의 방에만 · 지원 종류(rec 유닛 없음 = 빈 집합 · 못 읽음 = 기본 셋 · 읽음 = 그 집합) · 방 생성의 `record` 가르기(모르는 이름 1002 · 중복 없이 요청 차례).

use std::time::Duration;

use common::system::{ApiKey, Unit, UnitKind};
use oxhubd::ops::supervisor::{Role, SupCfg, Supervisor, UnitState, grant_room_session, rec_room_user, rec_user, supported_kinds};
use oxhubd::room::record_kinds_of;
use oxhubd::ses::token::Accounts;
use oxsig::Code;
use serde_json::json;

fn unit(id: &str, role: &str) -> Unit {
    Unit { id: id.into(), kind: UnitKind::Process, order: 3, role: role.into(), addr: None, cmd: vec!["true".into()], enabled: true }
}

fn sup(units: &[Unit]) -> std::sync::Arc<Supervisor> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SupCfg {
        node: "n1".into(),
        hub_inst: "i1".into(),
        build_label: None,
        backoff_ms: vec![100],
        intensity_max: 5,
        intensity_window: Duration::from_secs(60),
        hello_timeout: Duration::from_secs(1),
        extra_args: Vec::new(),
        sfu_args: Vec::new(),
        rec: None,
    };
    let s = Supervisor::new(units, cfg, tx);
    s.mark_state(Role::Bus, UnitState::Running, "l1");
    s
}

fn accounts(jwt: Option<&str>) -> Accounts {
    Accounts { keys: vec![ApiKey { key: "k".into(), secret: "s".into(), name: None, participant_types: vec![0, 1], hidden_allowed: true, ops_allowed: false, hook_allowed: false, subscribe: None, facts: None }], jwt_secret: jwt.map(str::to_string) }
}

#[test]
fn a_rec_unit_that_is_not_running_does_not_fail_ready() {
    let s = sup(&[unit("r1", "rec")]);
    assert_eq!(s.process_units().iter().map(|u| (u.id.as_str(), u.role)).collect::<Vec<_>>(), vec![("r1", Role::Rec)], "the supervisor starts and stops the rec unit");
    assert!(s.down_units().is_empty(), "정:RULE-OPS-SURFACE S-sym-IMPL-OPS-REC-UNIT-2 — a rec unit not Running does not take the node out of ready");
    let c = sup(&[unit("n1", "sfu"), unit("r1", "rec")]);
    assert_eq!(c.down_units(), vec!["n1".to_string()], "대조 — a media server that is not Running does");
}

#[test]
fn the_hub_base_is_the_listen_and_base_path_with_loopback_for_any() {
    use oxhubd::cfg::hub_base;
    assert_eq!(hub_base("0.0.0.0:1974", "/media", false), "http://127.0.0.1:1974/media", "정:IMPL-CFG-SYSTEM-FILE --hub-base — 모든 주소면 같은 갈래의 루프백");
    assert_eq!(hub_base("[::]:1974", "/media", true), "https://[::1]:1974/media", "[hub.tls] 면 https");
    assert_eq!(hub_base("10.0.0.5:8443", "/", true), "https://10.0.0.5:8443", "listen · base_path 그대로");
}

#[test]
fn the_gateway_signs_a_hidden_node_token_that_only_the_unit_key_opens() {
    let acc = accounts(None);
    let tok = acc.sign_unit(&rec_user("n1"), 3_600).expect("signed");
    let id = acc.verify_user(&tok).expect("valid");
    assert_eq!((id.user_id.as_str(), id.claims.participant_type, id.claims.hidden), ("rec-n1", 1, true), "정:IMPL-OPS-REC-UNIT 신원 — node 토큰 rec-{{node}} · participant_type 1 · hidden");
    assert_eq!(acc.verify_unit(&tok), Ok("rec-n1".to_string()), "정:IMPL-SES-TOKEN-ISSUE 유닛 열쇠로 풀리는 node 토큰");
    let none = Accounts { keys: Vec::new(), jwt_secret: None };
    let bare = none.sign_unit("rec-n1", 60).expect("the unit key lives in the hub process — no account secret is needed to sign");
    assert_eq!(acc.verify_unit(&bare), Ok("rec-n1".to_string()), "any verifier in the same hub process opens the unit-signed token");
}

#[test]
fn an_account_signed_token_is_not_a_node_token() {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    let acc = accounts(Some("j"));
    let now = common::now_ms() / 1000;
    let forged = json!({ "sub": "rec-n1", "participant_type": 1, "hidden": true, "iat": now, "exp": now + 60 });
    for (who, secret) in [("account secret", "s"), ("jwt_secret", "j")] {
        let t = encode(&Header::new(Algorithm::HS256), &forged, &EncodingKey::from_secret(secret.as_bytes())).expect("signed");
        assert!(acc.verify_user(&t).is_ok(), "stand check — the {who} token is still a valid user token");
        assert_eq!(acc.verify_unit(&t), Err(Code::TokenInvalid), "정:IMPL-SES-TOKEN-ISSUE 녹음 유닛 방 세션 토큰 — a token signed with the {who} that names rec-n1 is not a node token (TOKEN_INVALID)");
    }
}

#[test]
fn a_room_session_token_is_granted_to_the_node_token_for_a_room_on_that_node_only() {
    assert_eq!(grant_room_session("n1", "rec-n1", "r1", Some("n1")), Ok("rec-n1-r1".to_string()), "정:IMPL-SES-TOKEN-ISSUE 녹음 유닛 방 세션 토큰 — sub rec-{{node}}-{{room_id}}");
    assert_eq!(rec_room_user("n1", "r1"), "rec-n1-r1");
    assert_eq!(grant_room_session("n1", "rec-n2", "r1", Some("n1")), Err(Code::TokenInvalid), "another node's token is refused");
    assert_eq!(grant_room_session("n1", "alice", "r1", Some("n1")), Err(Code::TokenInvalid), "a user token is refused");
    assert_eq!(grant_room_session("n1", "rec-n1", "r1", Some("n2")), Err(Code::RoomNotFound), "a room that lives on another node is ROOM_NOT_FOUND");
    assert_eq!(grant_room_session("n1", "rec-n1", "r1", None), Err(Code::RoomNotFound), "a room that does not exist is ROOM_NOT_FOUND");
    let acc = accounts(None);
    let tok = acc.sign_unit(&rec_room_user("n1", "r1"), 60).expect("signed");
    let id = acc.verify_user(&tok).expect("the room session token binds");
    assert_eq!((id.user_id.as_str(), id.claims.participant_type, id.claims.hidden), ("rec-n1-r1", 1, true), "정:IMPL-OPS-REC-UNIT 방마다 신원 — participant_type 1 · hidden");
}

#[test]
fn supported_record_kinds_come_from_the_rec_unit_and_are_open_until_first_read() {
    assert_eq!(supported_kinds(false, None), Some(Vec::new()), "정:IMPL-OPS-REC-UNIT 결과물 소비자 — no rec unit in the system file is the empty set");
    assert_eq!(supported_kinds(false, Some(&["talk".to_string()])), Some(Vec::new()));
    assert_eq!(supported_kinds(true, None), None, "never read since the gateway started — not checked (a passing state is not a lasting refusal)");
    assert_eq!(supported_kinds(true, Some(&["talk".to_string(), "x".to_string()])), Some(vec!["talk".to_string(), "x".to_string()]), "read — the unit's registered set");
    assert_eq!(sup(&[unit("n1", "sfu")]).record_kinds(), Some(Vec::new()), "stand check — the supervisor without a rec unit");
    assert_eq!(sup(&[unit("r1", "rec")]).record_kinds(), None, "stand check — the supervisor with a rec unit not yet read");
}

#[test]
fn the_room_record_list_is_kept_in_request_order_without_repeats_and_unknown_names_refuse() {
    let kinds: Vec<String> = ["talk", "tracks", "raw"].iter().map(|s| s.to_string()).collect();
    let known = Some(kinds.as_slice());
    assert_eq!(record_kinds_of(None, known).ok(), Some(Vec::<String>::new()), "연:ROOM.http.CREATE_ROOM 없으면 녹음하지 않는다");
    assert_eq!(record_kinds_of(Some(&json!(null)), known).ok(), Some(Vec::<String>::new()));
    assert_eq!(record_kinds_of(Some(&json!(["raw", "talk", "raw"])), known).ok(), Some(vec!["raw".to_string(), "talk".to_string()]), "중복 없이 요청 차례로");
    for bad in [json!(["talk", "hls"]), json!(true), json!("talk"), json!([1])] {
        let f = record_kinds_of(Some(&bad), known).expect_err("refused");
        assert_eq!(f.code, 1002, "정:IMPL-ROOM-LIFE 생성 — {bad} is INVALID_PAYLOAD");
    }
    assert_eq!(record_kinds_of(Some(&json!(["talk"])), Some(&[])).map_err(|f| f.code), Err(1002), "the empty set refuses a non-empty record");
    assert!(record_kinds_of(Some(&json!([])), Some(&[])).is_ok_and(|v| v.is_empty()), "an empty record is fine without a rec unit");
    assert_eq!(record_kinds_of(Some(&json!(["hls", "talk", "hls"])), None).ok(), Some(vec!["hls".to_string(), "talk".to_string()]), "not yet read — names pass unchecked · still without repeats in request order");
    assert_eq!(record_kinds_of(Some(&json!(true)), None).map_err(|f| f.code), Err(1002), "not yet read — the shape is still checked");
}
