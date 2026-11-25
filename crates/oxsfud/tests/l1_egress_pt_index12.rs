// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-DOC-COND-INDEX · 연:RULE-LINK-SDP-SUB-MLINE · 연:RULE-LINK-EGRESS-REWRITE · 정:IMPL-SUB-SUBSCRIBER-TABLE · model: claude-opus-5-5
//! 1층 제어 층 · 소스 가드 — 하향은 구독자 표로 다시 짓는다(클라 연동 갈래에서 넘겨받은 연동규격서 색인 행): 구독 줄의 PT 는 그 구독자가 협상한 값이라 발행 PT 와 다를 수 있고 · 헤더 확장 번호(mid · abs-send-time · transport-cc)는 구독자 번호 · mid 확장 값은 구독 줄의 `mid`(발행자 m-line 번호가 아니다) · 발행자 확장 영역은 옮기지 않는다(context 202611/20261124b).

mod support;

use std::sync::{Arc, Once};

use common::envelope::CallKind;
use oxsfud::fwd::rt::{self, StartCfg};
use oxsfud::ops::handle::Ctx;
use oxsfud::ops::{self};
use oxsfud::room::Room;
use oxsfud::sub::ext::{ExtInput, build};
use oxsfud::sub::{URI_ABS_SEND_TIME, URI_AUDIO_LEVEL, URI_MID, URI_TWCC};
use oxsig::msg::{Duplex, ExtMap, Kind, PcMode};
use serde_json::json;

use support::*;

static MEDIA: Once = Once::new();

fn media() {
    MEDIA.call_once(|| {
        let at = "127.0.0.1:0".parse().expect("addr");
        rt::start(StartCfg { bind: at, advertised: at, tcp_port: None, workers: 1, queue_limit: rt::DEFAULT_QUEUE_PACKETS }).expect("media");
    });
    assert!(rt::media().is_some(), "stand check — the media layer is up");
}

fn create(id: &str) -> Arc<Room> {
    let mut e = envelope(CallKind::CreateRoom, None, None);
    e.body = json!({ "room_id": id, "name": id });
    assert!(drive(ops::rooms::create(&e)).is_ok(), "stand check — create {id}");
    sfu().rooms.get(id).expect("stand check — the created room is in the room set")
}

fn ext_value(out: &[u8], id: u8) -> Option<Vec<u8>> {
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
fn the_downlink_is_rebuilt_on_the_subscribers_own_pt_and_numbers() {
    let _g = serial();
    media();
    let r = create(&tag("i12"));
    let user = tag("i12-one");
    let c = Ctx { user: user.clone(), sid: format!("sid-{user}"), session: format!("ses-{user}"), pc_mode: PcMode::One, source_tag: false, home: None, claims: None, over: None };
    let extmap = vec![
        ExtMap { id: 3, uri: URI_MID.into() },
        ExtMap { id: 8, uri: URI_AUDIO_LEVEL.into() },
        ExtMap { id: 9, uri: URI_ABS_SEND_TIME.into() },
        ExtMap { id: 7, uri: URI_TWCC.into() },
    ];
    let codecs = json!([{ "kind": "video", "pt": 109, "name": "H264", "fmtp": "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f", "rtx_pt": 110 },
                        { "kind": "audio", "pt": 63, "name": "opus" }]);
    let res = ops::rooms::join(&c, &json!({ "room_id": r.id, "extmap": extmap, "codecs": codecs }));
    assert!(res.is_ok(), "stand check — a 1pc session joins with its own codecs and numbers: {res:?}");
    let u = subscriber(&tag("i12-pub"));
    enter(&u.p, &r);
    let (v, _) = published(&u.p, &r.id, Kind::Video, Duplex::Full);
    list(&v, true);
    ops::media::announce_add(&v);
    let s = sfu();
    let t = s.peer(&user).and_then(|p| s.table(&p)).expect("stand check — the subscriber table");
    let row = row_of(&t, Kind::Video, &v.track_id);
    let pt = row.pt.load(std::sync::atomic::Ordering::Acquire);
    assert_eq!(v.pt, 102, "stand check — the publisher declared PT 102");
    assert!(pt == 109 && pt != v.pt, "C-cond-RULE-DOC-COND-INDEX-12① the subscriber row carries the subscriber's PT (109), not the publisher's ({}) — got {pt}", v.pt);
    let ext = **row.ext.load();
    assert_eq!((ext.mid, ext.audio_level, ext.abs_send_time, ext.twcc), (Some(3), Some(8), Some(9), Some(7)), "C-cond-RULE-DOC-COND-INDEX-12②④ the row stamps the subscriber's extension numbers — abs-send-time and transport-cc too");
    let mut out = Vec::new();
    build(&ext, &ExtInput { mid: row.mid.as_bytes(), audio_level: None, t_us: 1, twcc_seq: Some(5) }, &mut out);
    let got = ext_value(&out, 3);
    assert_eq!(got.as_deref(), Some(row.mid.as_bytes()), "C-cond-RULE-DOC-COND-INDEX-12③ the mid extension carries the subscriber row's mid");
    assert_ne!(row.mid, v.mid, "stand check — the subscriber's mid ({}) differs from the publisher's m-line ({}) so the check tells them apart", row.mid, v.mid);
    assert!(ext_value(&out, 7).is_some() && ext_value(&out, 9).is_some(), "C-cond-RULE-DOC-COND-INDEX-12④ transport-cc and abs-send-time are written under the subscriber's numbers");
    for (twcc, t_us) in [(None, 0), (Some(9), 77)] {
        let mut again = Vec::new();
        build(&ext, &ExtInput { mid: row.mid.as_bytes(), audio_level: None, t_us, twcc_seq: twcc }, &mut again);
        assert_eq!(ext_value(&again, 3).as_deref(), Some(row.mid.as_bytes()), "C-cond-RULE-DOC-COND-INDEX-13② every downlink packet carries the m-line's mid (not only the first ones)");
    }
    let lines = product_lines();
    let inputs: Vec<String> = lines.iter().filter(|l| l.text.contains("ExtInput {")).map(|l| format!("{} {}", at(l), l.text.trim())).collect();
    assert!(inputs.len() >= 2 && inputs.iter().all(|x| x.contains("mid: row.mid.as_bytes()")), "C-cond-RULE-DOC-COND-INDEX-13② every place that writes a downlink extension takes the mid from the row (media and RTCP-path packets): {inputs:?}");
    let down: Vec<&Src> = lines.iter().filter(|l| l.file == "fwd/mod.rs" && l.func == "send_down").collect();
    let has = |x: &str| down.iter().any(|l| l.text.contains(x));
    assert!(has("core::pt_byte1(pkt.data[1], row.pt.load(Ordering::Relaxed))"), "C-cond-RULE-DOC-COND-INDEX-12① the egress PT is the subscriber row's");
    assert!(has("let table = row.ext.load();") && has("ExtInput { mid: row.mid.as_bytes()"), "C-cond-RULE-DOC-COND-INDEX-12②③ the egress builds the extension from the row's table and mid");
    let copies: Vec<String> = down.iter().filter(|l| l.text.contains("extend_from_slice(&pkt.data")).map(|l| l.text.trim().to_string()).collect();
    assert_eq!(copies, vec!["out.extend_from_slice(&pkt.data[pkt.hlen..]);".to_string()], "C-cond-RULE-DOC-COND-INDEX-12③ the publisher's extension area (its mid value) is not carried — only the payload");
}
