// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:PUB.op.PUBLISH_TRACKS · 연:RULE-TRK-FIELDS-BY-TYPE · model: claude-opus-5-5
//! 1층 — 발행 항목의 칸은 종류(kind)에 맞아야 한다 · audio 항목의 `source` 는 body 오류(1002)다. 판정은 `trk::item_misfit` 한 곳(발행 길목이 부른다 · context 202611/20261124b 발견 2).

use oxsfud::trk::item_misfit;
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
