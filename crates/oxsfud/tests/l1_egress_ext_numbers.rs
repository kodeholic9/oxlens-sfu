// author: kodeholic (powered by Claude)
// spec: v1.12 · 정:RULE-DOC-COND-INDEX-SERVER · 정:IMPL-SUB-SUBSCRIBER-TABLE · 연:RULE-LINK-EGRESS-REWRITE · model: claude-opus-5-5
//! 1층 제어 층 · 소스 가드 — 하향 RTP 의 헤더 확장은 구독자 쪽 번호로 다시 짓는다: `2pc` 구독 줄은 입장 응답 `server_config.extmap` 의 번호를, `1pc` 구독 줄은 그 사람이 `ROOM_JOIN` 에 실은 번호표의 번호를 든다 · mid 확장 값은 구독 줄의 `mid` · PT 는 구독 줄의 것 · 발행자 패킷의 확장 영역은 옮겨 싣지 않는다(context 202611/20261124b 서버 규칙 갈래).

mod support;

use std::sync::{Arc, Once};

use common::envelope::CallKind;
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::{self};
use oxsfud::room::Room;
use oxsfud::sub::ext::{ExtInput, build};
use oxsfud::sub::{URI_ABS_SEND_TIME, URI_AUDIO_LEVEL, URI_MID, URI_TWCC, ext_table};
use oxsig::msg::{Duplex, ExtMap, Kind, PcMode};
use serde_json::{Value, json};

use support::*;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up (a joining session builds transports)");
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn ctx(user: &str, pc: PcMode) -> Ctx {
    Ctx { user: user.into(), sid: format!("sid-{user}"), session: format!("ses-{user}"), pc_mode: pc, source_tag: false, home: None, claims: None, over: None }
}

fn mid_value(out: &[u8], id: u8) -> Option<Vec<u8>> {
    let words = u16::from_be_bytes([out[2], out[3]]) as usize;
    let mut i = 4;
    while i < 4 + words * 4 && i < out.len() {
        let b = out[i];
        if b == 0 {
            i += 1;
            continue;
        }
        let (eid, len) = (b >> 4, (b & 0x0f) as usize + 1);
        if eid == id {
            return Some(out[i + 1..i + 1 + len].to_vec());
        }
        i += 1 + len;
    }
    None
}

#[test]
fn downlink_extension_numbers_are_the_subscribers_own() {
    let _g = serial();
    media();
    let r = create(&tag("ext"));
    let (two, one) = (tag("ext-two"), tag("ext-one"));
    let j2: Value = ops::rooms::join(&ctx(&two, PcMode::Two), &json!({ "room_id": r.id })).expect("stand check — a 2pc session joins");
    let mine = vec![
        ExtMap { id: 3, uri: URI_MID.into() },
        ExtMap { id: 8, uri: URI_AUDIO_LEVEL.into() },
        ExtMap { id: 9, uri: URI_ABS_SEND_TIME.into() },
        ExtMap { id: 7, uri: URI_TWCC.into() },
    ];
    let res = ops::rooms::join(&ctx(&one, PcMode::One), &json!({ "room_id": r.id, "extmap": mine }));
    assert!(res.is_ok(), "stand check — a 1pc session joins with its own number table: {res:?}");
    let u = subscriber(&tag("ext-pub"));
    enter(&u.p, &r);
    let (v, _) = published(&u.p, &r.id, Kind::Video, Duplex::Full);
    list(&v, true);
    ops::media::announce_add(&v);
    let s = sfu();
    let table_of = |user: &str| s.peer(user).and_then(|p| s.table(&p)).expect("stand check — the subscriber table");
    let advertised: Vec<ExtMap> = serde_json::from_value(j2["server_config"]["extmap"].clone()).expect("stand check — server_config.extmap is a number table");
    let row2 = row_of(&table_of(&two), Kind::Video, &v.track_id);
    assert_eq!(**row2.ext.load(), ext_table(&advertised), "S-cond-RULE-DOC-COND-INDEX-SERVER-10 a 2pc row stamps the numbers of server_config.extmap");
    let row1 = row_of(&table_of(&one), Kind::Video, &v.track_id);
    let t1 = **row1.ext.load();
    assert_eq!((t1.mid, t1.audio_level, t1.abs_send_time, t1.twcc), (Some(3), Some(8), Some(9), Some(7)), "S-cond-RULE-DOC-COND-INDEX-SERVER-10 a 1pc row stamps the numbers that session sent in ROOM_JOIN");
    assert_ne!(t1, **row2.ext.load(), "stand check — the two sessions' tables differ, so the check tells them apart");
    for row in [&row1, &row2] {
        let t = **row.ext.load();
        let mut out = Vec::new();
        build(&t, &ExtInput { mid: row.mid.as_bytes(), audio_level: None, t_us: 0, twcc_seq: Some(1) }, &mut out);
        assert_eq!(mid_value(&out, t.mid.expect("mid number")), Some(row.mid.as_bytes().to_vec()), "S-cond-RULE-DOC-COND-INDEX-SERVER-10 the mid extension carries the subscriber row's mid");
    }
    let lines = product_lines();
    let down: Vec<&Src> = lines.iter().filter(|l| l.file == "fwd/mod.rs" && l.func == "send_down").collect();
    let has = |x: &str| down.iter().any(|l| l.text.contains(x));
    assert!(has("let table = row.ext.load();") && has("ExtInput { mid: row.mid.as_bytes()"), "S-cond-RULE-DOC-COND-INDEX-SERVER-10 the egress builds the extension from the row's table and the row's mid");
    assert!(has("core::pt_byte1(pkt.data[1], row.pt.load(Ordering::Relaxed))"), "S-cond-RULE-DOC-COND-INDEX-SERVER-10 the egress PT is the subscriber row's");
    let copies: Vec<String> = down.iter().filter(|l| l.text.contains("extend_from_slice(&pkt.data")).map(|l| l.text.trim().to_string()).collect();
    assert_eq!(copies, vec!["out.extend_from_slice(&pkt.data[pkt.hlen..]);".to_string()], "S-cond-RULE-DOC-COND-INDEX-SERVER-10 the publisher's extension area is not carried — only the payload");
}
