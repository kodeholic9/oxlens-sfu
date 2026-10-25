// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-TRK-FROM-PUB 정:IMPL-ROOM-EMIT 연:RULE-TRK-PUB-TO-TRACK 연:RULE-TRK-SLOT-LIFE 연:RULE-TRK-PER-PATH 연:RULE-TRK-ELEMENT-HAS-ROOM-ID 연:TRK.type.TrackEntry 연:R-TRK-MUTED-IN-SNAPSHOT 연:RULE-PUB-RID-TO-SPATIAL · model: claude-opus-5-5
//! 트랙 층 — 슬롯 둘(방과 수명이 같다) + 전이중 개인 트랙(쉬는 것 포함) · `TrackEntry` 는 전 필드를 싣고 `remove` 는 넷만.

pub mod codec;

use oxsig::msg::{Kind, TrackEntry, TrackEntryType};

use crate::r#pub::SCALABILITY;
use crate::room::{Listed, Room, State};

pub fn slot_id(room: &str, kind: Kind) -> String {
    match kind {
        Kind::Audio => format!("ptt-{room}-audio"),
        Kind::Video => format!("ptt-{room}-video"),
    }
}

pub fn slot(room: &Room, kind: Kind) -> TrackEntry {
    let c = room.codec_of(kind);
    TrackEntry {
        type_: TrackEntryType::Slot,
        room_id: room.id.clone(),
        track_id: slot_id(&room.id, kind),
        kind,
        codec: (kind == Kind::Video).then(|| c.to_string()),
        fmtp: (kind == Kind::Video).then(|| codec::row_fmtp(c).map(str::to_string)).flatten(),
        user_id: None,
        source: None,
        active: None,
        muted: None,
        simulcast: None,
        scalability: None,
    }
}

pub fn individual(room: &Room, l: &Listed) -> TrackEntry {
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: room.id.clone(),
        track_id: l.track_id.clone(),
        kind: l.kind,
        codec: (l.kind == Kind::Video).then(|| room.codec_of(l.kind).to_string()),
        fmtp: l.fmtp.clone(),
        user_id: Some(l.user.clone()),
        source: l.source,
        active: Some(l.active),
        muted: Some(l.muted),
        simulcast: Some(l.simulcast),
        scalability: l.simulcast.then(|| SCALABILITY.to_string()),
    }
}

pub fn removed(room_id: &str, l: &Listed) -> TrackEntry {
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: room_id.to_string(),
        track_id: l.track_id.clone(),
        kind: l.kind,
        codec: None,
        fmtp: None,
        user_id: None,
        source: None,
        active: None,
        muted: None,
        simulcast: None,
        scalability: None,
    }
}

pub fn all(room: &Room, st: &State) -> Vec<TrackEntry> {
    let mut v = vec![slot(room, Kind::Audio), slot(room, Kind::Video)];
    v.extend(st.tracks.iter().map(|l| individual(room, l)));
    v
}

pub fn receivable(st: &State, me: &str) -> Vec<Listed> {
    st.tracks.iter().filter(|l| l.active && l.user != me).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::msg::MediaCodecs;

    #[test]
    fn slots_always_two_with_room_codec() {
        let s = crate::room::Spec {
            id: "r-1".into(),
            name: "x".into(),
            capacity: 5,
            codecs: MediaCodecs { audio: Some("opus".into()), video: Some("VP8".into()) },
            unused_ttl: 0,
            departure_ttl: 0,
            queue_max: 10,
            t2_secs: 30,
        };
        let r = Room::new(s, "e", 1);
        let st = r.lock();
        let v = all(&r, &st);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].track_id, "ptt-r-1-audio");
        assert_eq!(v[1].codec.as_deref(), Some("VP8"));
        assert_eq!(v[1].fmtp, None);
    }
}
