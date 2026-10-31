// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-AFF-ONE-SET · 정:IMPL-AFF-PUB-ROOM · 정:IMPL-AFF-PUB-MOVE · 연:AFF.type.Affiliation · 연:RULE-AFF-PUB-SELECT-CODEC-RECHECK · 정:RULE-AFF-SERVER-SCOPE · 연:RULE-AFF-ONE-PUB-ROOM · 연:RULE-AFF-PUB-IN-SUB · 연:RULE-AFF-TWO-AXES · 연:RULE-AFF-PARTIAL · model: claude-opus-5-5
//! 소속 — 입장 방 집합(= 구독 집합) 하나와 `pub_room`(RCU 값 · 매 패킷 이 값으로 방을 정한다) · 발행 방을 옮길 때의 재검사 둘(코덱 · 권한 비트).

use std::sync::Arc;

use oxsig::{Code, Failure};
use oxsig::msg::{Affiliation, Kind, Source};

use crate::link::Peer;
use crate::r#pub::PubStream;
use crate::room::Room;

pub fn pub_room(p: &Peer) -> Option<String> {
    p.publish.pub_room.load_full().map(|r| r.as_str().to_string())
}

pub fn set_pub_room(p: &Peer, room: Option<&str>) {
    let v = room.map(|r| Arc::new(r.to_string()));
    p.publish.pub_room.store(v.clone());
    if !Arc::ptr_eq(&p.publish, &p.subscribe) {
        p.subscribe.pub_room.store(v);
    }
}

pub fn affiliation(p: &Peer) -> Affiliation {
    Affiliation { sub_rooms: p.rooms(), pub_room: pub_room(p) }
}

pub fn bit_allows(perm: &oxsig::msg::Permission, kind: Kind, source: Option<Source>) -> bool {
    match (kind, source) {
        (Kind::Audio, _) => perm.publish_audio,
        (Kind::Video, Some(Source::Screen)) => perm.publish_screen,
        (Kind::Video, _) => perm.publish_video,
    }
}

pub fn bit_name(kind: Kind, source: Option<Source>) -> &'static str {
    match (kind, source) {
        (Kind::Audio, _) => "publish_audio",
        (Kind::Video, Some(Source::Screen)) => "publish_screen",
        (Kind::Video, _) => "publish_video",
    }
}

pub fn recheck(to: &Room, user: &str, streams: &[Arc<PubStream>], token: Option<oxsig::msg::Permission>) -> Result<(), Failure> {
    for s in streams {
        if !crate::trk::matches_room(s.kind, s.codec.as_deref().or(Some(if s.kind == Kind::Audio { "opus" } else { "" })), s.fmtp.as_deref(), &to.media_codecs) {
            return Err(crate::trk::mismatch(s.kind, &to.media_codecs));
        }
    }
    let perm = to.permission_for(user, token);
    let denied: Vec<&str> = streams.iter().filter(|s| !bit_allows(&perm, s.kind, s.source)).map(|s| bit_name(s.kind, s.source)).collect();
    if !denied.is_empty() {
        return Err(Code::NotAuthorized.failure().with_details(serde_json::json!({ "denied": denied })));
    }
    Ok(())
}
