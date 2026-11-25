// author: kodeholic (powered by Claude)
// spec: v1.12 · 연:RULE-DOC-COND-INDEX · 연:TRK.type.TrackEntry · 연:SUB.type.MlineTable · model: claude-opus-5-5
//! 1층 제어 층 — 트랙 항목은 세 층으로 갈린다(클라 연동 갈래에서 넘겨받은 연동규격서 색인 행): 모든 항목에 `type` · 슬롯 항목에는 `user_id` · `muted` · `active` 가 없다 · 같은 트랙을 두 세션이 받아도 트랙 항목은 같고 배정은 연결마다의 표에만 있다 · 반이중 발행은 방 트랙 목록에 안 들고 `RESUME.publications[]` 에만 있다(context 202611/20261124b).

mod support;

use common::envelope::CallKind;
use oxsfud::ops::{self};
use oxsig::msg::{Duplex, Kind, TrackEntryType};

use support::*;

#[test]
fn track_entries_assignment_tables_and_publications_are_three_layers() {
    let _g = serial();
    let r = room(&tag("tel"));
    let (b, c, u) = (subscriber(&tag("tel-b")), subscriber(&tag("tel-c")), subscriber(&tag("tel-u")));
    for p in [&b.p, &c.p, &u.p] {
        enter(p, &r);
    }
    let (full, _) = published(&u.p, &r.id, Kind::Audio, Duplex::Full);
    list(&full, true);
    ops::media::announce_add(&full);
    let (half, _) = published(&u.p, &r.id, Kind::Video, Duplex::Half);
    let entries = ops::rooms::tracks_of(&r);
    let json = serde_json::to_value(&entries).expect("entries");
    assert!(json.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|e| e.get("type").is_some())), "C-cond-RULE-DOC-COND-INDEX-21① every track entry carries type: {json}");
    let slots: Vec<_> = entries.iter().filter(|e| e.type_ == TrackEntryType::Slot).collect();
    assert!(!slots.is_empty(), "stand check — the room has slot entries");
    assert!(slots.iter().all(|e| e.user_id.is_none() && e.muted.is_none() && e.active.is_none()), "C-cond-RULE-DOC-COND-INDEX-21① slot entries carry no user_id · muted · active: {slots:?}");
    assert!(entries.iter().any(|e| e.track_id == full.track_id && e.type_ == TrackEntryType::Individual), "stand check — the full-duplex track is an individual entry");
    let s = sfu();
    let (tb, tc) = (b.t.snapshot(&s.epoch), c.t.snapshot(&s.epoch));
    let (rb, rc) = (row_of(&b.t, Kind::Audio, &full.track_id), row_of(&c.t, Kind::Audio, &full.track_id));
    assert_ne!(rb.ssrc, rc.ssrc, "C-cond-RULE-DOC-COND-INDEX-21② the same track rides a different row in each session's table (downlink SSRC differs)");
    assert_ne!(serde_json::to_value(&tb).ok(), serde_json::to_value(&tc).ok(), "C-cond-RULE-DOC-COND-INDEX-21② the two sessions' assignment tables differ");
    let entry = serde_json::to_value(entries.iter().find(|e| e.track_id == full.track_id).expect("entry")).expect("entry json");
    assert!(entry.get("mid").is_none() && entry.get("assign").is_none() && entry.get("ssrc").is_none(), "C-cond-RULE-DOC-COND-INDEX-21② the track entry is the same for everyone — it carries no assignment: {entry}");
    assert!(!entries.iter().any(|e| e.track_id == half.track_id), "C-cond-RULE-DOC-COND-INDEX-21④ a half-duplex publication is not a track entry in the room list");
    let res = ops::rooms::resume(&envelope(CallKind::Resume, Some(&u.p.user), Some(&u.p.sid))).expect("stand check — RESUME answers");
    let pubs: Vec<(String, String)> = res["publications"].as_array().map(|a| a.iter().map(|p| (p["track_id"].as_str().unwrap_or("").to_string(), p["duplex"].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
    assert!(pubs.iter().any(|(t, d)| t == &half.track_id && d == "half"), "C-cond-RULE-DOC-COND-INDEX-21④ RESUME.publications[] lists the half-duplex publication: {pubs:?}");
}
