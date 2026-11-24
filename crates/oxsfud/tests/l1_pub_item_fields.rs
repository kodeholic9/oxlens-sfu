// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:PUB.op.PUBLISH_TRACKS · 연:RULE-TRK-FIELDS-BY-TYPE · model: claude-opus-5-5
//! 1층 — 발행 항목의 칸은 종류(kind)에 맞아야 한다 · audio 항목의 `source` · `simulcast: true` 는 body 오류(1002)다 · 1pc 다운링크 SSRC 와 겹친 신고는 3008 이고 겹친 값을 낸다(`details.ssrc`). 판정은 `trk::item_misfit` 한 곳(발행 길목이 부른다 · context 202611/20261124b 발견 2).

use std::collections::BTreeSet;

use oxsfud::trk::{item_misfit, ssrc_collision};
use oxsig::msg::PublishTrack;
use serde_json::json;

fn item(v: serde_json::Value) -> PublishTrack {
    serde_json::from_value(v).expect("publish item")
}

#[test]
fn audio_source_is_a_misfit() {
    let base = json!({"kind": "audio", "ssrc": 1, "mid": "0", "pt": 111});
    assert_eq!(item_misfit(&item(base.clone())), None, "C-sym-R-ROOM-BITS-SYMPTOMS-2 audio without source fits");
    for src in ["camera", "screen"] {
        let mut v = base.clone();
        v["source"] = json!(src);
        assert_eq!(item_misfit(&item(v)), Some("source"), "C-sym-R-ROOM-BITS-SYMPTOMS-2 audio carrying source {src} is a body error");
    }
}

#[test]
fn video_source_fits() {
    for src in [None, Some("camera"), Some("screen")] {
        let mut v = json!({"kind": "video", "ssrc": 2, "mid": "1", "pt": 96, "codec": "VP8"});
        if let Some(s) = src {
            v["source"] = json!(s);
        }
        assert_eq!(item_misfit(&item(v)), None, "video with source {src:?} fits");
    }
}

#[test]
fn audio_simulcast_true_is_a_misfit() {
    let mut v = json!({"kind": "audio", "ssrc": 3, "mid": "0", "pt": 111});
    v["simulcast"] = json!(false);
    assert_eq!(item_misfit(&item(v.clone())), None, "audio simulcast:false fits");
    v["simulcast"] = json!(true);
    assert_eq!(item_misfit(&item(v)), Some("simulcast"), "audio carrying simulcast:true is a body error");
    let mut vid = json!({"kind": "video", "ssrc": 0, "mid": "1", "pt": 96, "codec": "VP8"});
    vid["simulcast"] = json!(true);
    assert_eq!(item_misfit(&item(vid)), None, "video simulcast fits");
}

#[test]
fn ssrc_collision_names_the_value() {
    let down: BTreeSet<u32> = [0x1111, 0x2222].into_iter().collect();
    let a = item(json!({"kind": "audio", "ssrc": 0x3333, "mid": "0", "pt": 111}));
    let v = item(json!({"kind": "video", "ssrc": 0x4444, "rtx_ssrc": 0x2222, "mid": "1", "pt": 96, "codec": "VP8"}));
    let w = item(json!({"kind": "audio", "ssrc": 0x1111, "mid": "2", "pt": 111}));
    assert_eq!(ssrc_collision(&[a.clone()], &down), None, "no overlap");
    assert_eq!(ssrc_collision(&[a.clone(), v], &down), Some(0x2222), "CP-OP-PUBLISH-TRACKS④ rtx_ssrc overlap names the value");
    assert_eq!(ssrc_collision(&[a, w], &down), Some(0x1111), "CP-OP-PUBLISH-TRACKS④ ssrc overlap names the value");
}
