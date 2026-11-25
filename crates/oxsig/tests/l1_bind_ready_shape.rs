// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-DOC-COND-INDEX · 연:SES.op · 연:RULE-SUB-TABLE-PROC · model: claude-sonnet-5-5
//! 1층 본문 칸 대조 — `BIND` 요청은 재접속 자격이 `token` · `session_id` 둘뿐이고(제3의 자격 칸 0), `READY` 요청에는 `transport` 칸이 없으며, `TRACK_EVENT` 본문은 `version` 없이 만들어지지 않는다(context 202611/20261124b 클라 연동 갈래).

use oxsig::msg::{BindReq, ReadyReq, TrackEventBody};
use serde_json::{json, Value};

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn bind_req_carries_only_token_and_session_id_as_credentials() {
    let wire = json!({"token": "t", "session_id": "s", "client_ver": 1, "pc_mode": "2pc", "source_tag": true, "refresh_token": "x", "reconnect_secret": "y"});
    let req: BindReq = serde_json::from_value(wire).expect("bind req parses");
    let back = serde_json::to_value(&req).expect("bind req serializes");
    assert_eq!(keys(&back), ["client_ver", "pc_mode", "session_id", "source_tag", "token"], "C-cond-RULE-DOC-COND-INDEX-15④ BindReq has no third credential field — the unknown refresh_token and reconnect_secret are not representable");
}

#[test]
fn ready_req_has_no_transport_field() {
    let wire = json!({"room_id": "r1", "join_id": "j1", "arev": 7, "transport": "publish"});
    let req: ReadyReq = serde_json::from_value(wire).expect("ready req parses");
    let back = serde_json::to_value(&req).expect("ready req serializes");
    assert_eq!(keys(&back), ["arev", "join_id", "room_id"], "C-cond-RULE-DOC-COND-INDEX-20③ ReadyReq carries room_id · join_id · arev only — no transport field to send");
}

#[test]
fn track_event_body_cannot_exist_without_version() {
    let no_version = json!({"action": "add", "room_id": "r1", "tracks": []});
    assert!(serde_json::from_value::<TrackEventBody>(no_version).is_err(), "C-cond-RULE-DOC-COND-INDEX-20③ a TRACK_EVENT body without version does not parse — a version-less reassignment cannot be built");
    let with_version = json!({"action": "add", "room_id": "r1", "tracks": [], "version": {"epoch": "e", "seq": 1}});
    assert!(serde_json::from_value::<TrackEventBody>(with_version).is_ok(), "stand check — the same body with version parses");
}
