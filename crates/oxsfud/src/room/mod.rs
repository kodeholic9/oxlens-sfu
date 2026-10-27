// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-ROOM-SERVER-SCOPE · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-PERMISSION · 정:IMPL-ROOM-VERSION-ISSUE · 정:IMPL-ROOM-EMIT · 정:IMPL-ROOM-STALL · 정:CAND-ROOM-TTL-PERMANENT-MARK · 정:CAND-ROOM-STALL-COOL-UNIT · 정:CAND-ROOM-JOIN-AGAIN-EDGE · 연:ROOM.type.MemberInfo · 연:ROOM.type.Version · 연:RULE-ROOM-HIDDEN-ONLY-FOUR · 연:RULE-ROOM-HIDDEN-NO-SEQ · 연:RULE-ROOM-SEQ-CONTENT · 연:RULE-ROOM-MEDIA-CODECS · 연:RULE-ROOM-PERMISSION-MAP · 연:ROOM.ledger.TStall · 연:ROOM.S · model: claude-opus-5-5
//! 방 — 빈 그릇과 수명(TTL 둘 · 영구 표지 `u32::MAX`) · 명단(투명도 등록) · `version{epoch, seq}` · 권한 오버라이드 표 · 개인 트랙 목록 · 슬롯 둘(핫패스) · 발언권 판정 그릇.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxsig::types::{Kind, MediaCodecs, MemberInfo, Permission, RoomSnapshot, RoomSummary, TrackEntry, Version};

use crate::fwd::hot::RoomHot;

pub const PERMANENT: u32 = u32::MAX;
pub const SWEEP_MS: u64 = 5_000;
pub const STALL_MS: u64 = 5_000;
pub const T_STALL_MS: u64 = 30_000;
pub const MAX_ROOMS: usize = 100;

#[derive(Debug, Clone)]
pub struct Member {
    pub info: MemberInfo,
    pub hidden: bool,
    pub session: String,
}

pub trait TrackLayer: Clone {
    fn entry(&self, room: &str) -> TrackEntry;
    fn slot(room: &str, kind: Kind, video_codec: &str) -> TrackEntry;
}

pub struct Room<T: TrackLayer> {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub audio_codec: String,
    pub video_codec: String,
    pub unused_ttl: u32,
    pub departure_ttl: u32,
    pub created_mono: u64,
    pub empty_since: u64,
    pub ever_joined: bool,
    pub members: BTreeMap<String, Member>,
    pub seq: u64,
    pub epoch: String,
    pub overrides: BTreeMap<String, Permission>,
    pub tracks: BTreeMap<String, T>,
    pub hot: Arc<RoomHot>,
    pub floor: crate::flr::Floor,
    pub stall_cool: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Keep,
    MarkEmpty,
    Expire,
}

impl<T: TrackLayer> Room<T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(id: &str, name: &str, capacity: u32, codecs: &MediaCodecs, unused_ttl: u32, departure_ttl: u32, epoch: &str, hot: Arc<RoomHot>, floor: crate::flr::Floor, now_mono: u64) -> Room<T> {
        Room {
            id: id.to_string(),
            name: name.to_string(),
            capacity,
            created_at: common::time::wall_ms(),
            audio_codec: codecs.audio.clone().unwrap_or_else(|| hot.audio_codec.to_string()),
            video_codec: codecs.video.clone().unwrap_or_else(|| hot.video_codec.to_string()),
            unused_ttl,
            departure_ttl,
            created_mono: now_mono,
            empty_since: now_mono,
            ever_joined: false,
            members: BTreeMap::new(),
            seq: 0,
            epoch: epoch.to_string(),
            overrides: BTreeMap::new(),
            tracks: BTreeMap::new(),
            hot,
            floor,
            stall_cool: BTreeMap::new(),
        }
    }

    pub fn media_codecs(&self) -> MediaCodecs {
        MediaCodecs { audio: Some(self.audio_codec.clone()), video: Some(self.video_codec.clone()) }
    }

    pub fn version(&self) -> Version {
        Version { epoch: self.epoch.clone(), seq: self.seq }
    }

    pub fn bump(&mut self) -> Version {
        self.seq += 1;
        self.version()
    }

    pub fn user_count(&self) -> u32 {
        self.members.values().filter(|m| !m.hidden).count() as u32
    }

    pub fn registered(&self) -> usize {
        self.members.len()
    }

    pub fn rec(&self) -> bool {
        self.members.values().any(|m| m.info.participant_type == 1)
    }

    pub fn summary(&self) -> RoomSummary {
        RoomSummary { room_id: self.id.clone(), name: self.name.clone(), capacity: self.capacity, user_count: self.user_count(), created_at: self.created_at, rec: self.rec() }
    }

    pub fn participants(&self) -> Vec<MemberInfo> {
        self.members.values().filter(|m| !m.hidden).map(|m| m.info.clone()).collect()
    }

    pub fn track_entries(&self) -> Vec<TrackEntry> {
        let mut v: Vec<TrackEntry> = vec![T::slot(&self.id, Kind::Audio, &self.video_codec), T::slot(&self.id, Kind::Video, &self.video_codec)];
        v.extend(self.tracks.values().map(|t| t.entry(&self.id)));
        v
    }

    pub fn snapshot(&self, me: &str) -> Option<RoomSnapshot> {
        let m = self.members.get(me)?;
        Some(RoomSnapshot { participants: self.participants(), me: m.info.clone(), tracks: self.track_entries(), version: self.version() })
    }

    pub fn full_for(&self, hidden: bool) -> bool {
        !hidden && self.user_count() >= self.capacity
    }

    pub fn permission_of(&self, user: &str, token: Permission) -> Permission {
        self.overrides.get(user).copied().unwrap_or(token)
    }

    pub fn set_override(&mut self, user: &str, p: Permission) {
        if p == Permission::default() {
            self.overrides.remove(user);
        } else {
            self.overrides.insert(user.to_string(), p);
        }
    }

    pub fn sweep(&mut self, now_mono: u64) -> Sweep {
        if !self.members.is_empty() {
            self.empty_since = 0;
            return Sweep::Keep;
        }
        if self.empty_since == 0 {
            self.empty_since = now_mono;
            return Sweep::MarkEmpty;
        }
        let ttl = if self.ever_joined { self.departure_ttl } else { self.unused_ttl };
        if ttl == PERMANENT {
            return Sweep::Keep;
        }
        if now_mono.saturating_sub(self.empty_since) >= u64::from(ttl) * 1_000 { Sweep::Expire } else { Sweep::Keep }
    }

    pub fn stall_allowed(&mut self, user: &str, now_ms: u64) -> bool {
        match self.stall_cool.get(user) {
            Some(until) if *until > now_ms => false,
            _ => {
                self.stall_cool.insert(user.to_string(), now_ms + T_STALL_MS);
                true
            }
        }
    }
}

pub fn permission_bits_changed(old: &Permission, new: &Permission) -> Vec<&'static str> {
    let mut v = Vec::new();
    if old.publish_audio != new.publish_audio {
        v.push("publish_audio");
    }
    if old.publish_video != new.publish_video {
        v.push("publish_video");
    }
    if old.publish_screen != new.publish_screen {
        v.push("publish_screen");
    }
    if old.floor_request != new.floor_request {
        v.push("floor_request");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct NoTrack;

    impl TrackLayer for NoTrack {
        fn entry(&self, room: &str) -> TrackEntry {
            Self::slot(room, Kind::Audio, "")
        }

        fn slot(room: &str, kind: Kind, _video_codec: &str) -> TrackEntry {
            TrackEntry { type_: oxsig::types::TrackEntryType::Slot, room_id: room.into(), track_id: room.into(), kind, codec: None, fmtp: None, user_id: None, source: None, active: None, muted: None, simulcast: None, scalability: None }
        }
    }

    fn room(unused: u32, dep: u32) -> Room<NoTrack> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let hot = Arc::new(RoomHot {
            id: Arc::from("r"),
            audio: crate::fwd::hot::Slot::new(Kind::Audio, "r"),
            video: crate::fwd::hot::Slot::new(Kind::Video, "r"),
            audio_codec: Arc::from("opus"),
            video_codec: Arc::from("H264"),
            floor_tx: tx,
        });
        Room::new("r", "n", 10, &MediaCodecs::default(), unused, dep, "e", hot, crate::flr::Floor::new(10, 30), 1_000)
    }

    #[test]
    fn ttl_from_creation_and_permanent() {
        let mut r = room(30, 5);
        assert_eq!(r.sweep(20_000), Sweep::Keep);
        assert_eq!(r.sweep(31_000), Sweep::Expire);
        let mut p = room(PERMANENT, PERMANENT);
        assert_eq!(p.sweep(10_000_000), Sweep::Keep);
        let mut z = room(30, 0);
        z.ever_joined = true;
        z.empty_since = 0;
        assert_eq!(z.sweep(50_000), Sweep::MarkEmpty, "비었다는 첫 관측 회차에는 만료 판정을 하지 않는다");
        assert_eq!(z.sweep(55_000), Sweep::Expire);
    }

    #[test]
    fn stall_cool_per_user() {
        let mut r = room(30, 5);
        assert!(r.stall_allowed("a", 0));
        assert!(!r.stall_allowed("a", 10_000));
        assert!(r.stall_allowed("b", 10_000));
        assert!(r.stall_allowed("a", 30_000));
    }
}
