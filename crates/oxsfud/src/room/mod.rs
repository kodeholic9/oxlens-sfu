// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-ROOM-SERVER-SCOPE · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-JOIN · 정:IMPL-ROOM-PERMISSION · 정:IMPL-ROOM-VERSION-ISSUE · 정:CAND-ROOM-STALL-COOL-UNIT · 연:ROOM.type.MemberInfo · 연:ROOM.type.RoomSummary · 연:ROOM.type.Permission · 연:ROOM.type.Version · 연:RULE-ROOM-HIDDEN-ONLY-FOUR · 연:RULE-ROOM-HIDDEN-NO-SEQ · 연:RULE-ROOM-BITS-PER-ROOM · 연:RULE-ROOM-BITS-OUTLIVE · 연:RULE-ROOM-SEQ-CONTENT · 연:RULE-ROOM-ONE-COUNTER · 연:RULE-ROOM-MAX-100 · 연:COND-ROOM-SIMPLE-U64 · model: claude-opus-5-5
//! 영역 ROOM — 방이라는 그릇(정체 · 수명 TTL 둘 · 명단 · 권한 오버라이드 표 · 판번호 · 트랙 층). 저장하지 않고 명단에서 판다(`user_count` · `rec`).

pub mod core;

use std::collections::HashMap;
use std::sync::Arc;

use oxsig::{MediaCodecs, MemberInfo, Permission, RoomSummary, Version};

use crate::r#pub::PubTrack;

pub const MAX_ROOMS_PER_SERVER: usize = 100;
pub const RECORDER: u8 = 1;
pub const ROLE_DEFAULT: u8 = 255;

#[derive(Debug, Clone)]
pub struct Member {
    pub user: String,
    pub role: Option<u8>,
    pub select: bool,
    pub participant_type: u8,
    pub metadata: Option<serde_json::Value>,
    pub hidden: bool,
    pub permission: Permission,
    pub joined_ms: u64,
    pub ready_at: u64,
}

impl Member {
    pub fn info(&self) -> MemberInfo {
        MemberInfo {
            user_id: self.user.clone(),
            role: Some(self.role.unwrap_or(ROLE_DEFAULT)),
            select: self.select,
            participant_type: self.participant_type,
            metadata: self.metadata.clone(),
            permission: (!self.permission.is_default()).then_some(self.permission),
        }
    }
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub unused_ttl_secs: u32,
    pub departure_ttl_secs: u32,
    pub media: MediaCodecs,
    pub seq: u64,
    pub members: Vec<Member>,
    pub overrides: HashMap<String, Permission>,
    pub tracks: Vec<Arc<PubTrack>>,
    pub empty_since: u64,
    pub ever_joined: bool,
    pub stall_cool: HashMap<String, u64>,
    pub created_mono: u64,
}

impl Room {
    #[allow(clippy::too_many_arguments)]
    pub fn new(id: &str, name: &str, capacity: u32, unused: u32, departure: u32, media: MediaCodecs, now_wall: u64, now_mono: u64) -> Room {
        Room {
            id: id.to_string(),
            name: name.to_string(),
            capacity,
            created_at: now_wall,
            unused_ttl_secs: unused,
            departure_ttl_secs: departure,
            media,
            seq: 0,
            members: Vec::new(),
            overrides: HashMap::new(),
            tracks: Vec::new(),
            empty_since: now_mono.max(1),
            ever_joined: false,
            stall_cool: HashMap::new(),
            created_mono: now_mono,
        }
    }

    pub fn member(&self, u: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.user == u)
    }

    pub fn member_mut(&mut self, u: &str) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.user == u)
    }

    pub fn has(&self, u: &str) -> bool {
        self.member(u).is_some()
    }

    pub fn participants(&self) -> Vec<MemberInfo> {
        self.members.iter().filter(|m| !m.hidden).map(Member::info).collect()
    }

    pub fn user_count(&self) -> u32 {
        self.members.iter().filter(|m| !m.hidden).count() as u32
    }

    pub fn hidden_count(&self) -> u32 {
        self.members.iter().filter(|m| m.hidden).count() as u32
    }

    pub fn rec(&self) -> bool {
        self.members.iter().any(|m| m.participant_type == RECORDER)
    }

    pub fn version(&self, epoch: &str) -> Version {
        Version { epoch: epoch.to_string(), seq: self.seq }
    }

    pub fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn summary(&self) -> RoomSummary {
        RoomSummary { room_id: self.id.clone(), name: self.name.clone(), capacity: self.capacity, user_count: self.user_count(), created_at: self.created_at, rec: self.rec() }
    }

    pub fn video_codec(&self) -> String {
        self.media.video.clone().unwrap_or_else(|| "H264".into())
    }

    pub fn listed(&self, track_id: &str) -> Option<&Arc<PubTrack>> {
        self.tracks.iter().find(|t| t.track_id == track_id)
    }

    pub fn unlist(&mut self, track_id: &str) -> Option<Arc<PubTrack>> {
        let i = self.tracks.iter().position(|t| t.track_id == track_id)?;
        Some(self.tracks.remove(i))
    }

    pub fn list(&mut self, t: Arc<PubTrack>) {
        if !self.tracks.iter().any(|x| x.track_id == t.track_id) {
            self.tracks.push(t);
        }
    }

    pub fn effective_permission(&self, user: &str, token: Option<Permission>) -> Permission {
        self.overrides.get(user).copied().or(token).unwrap_or(Permission::ALL)
    }
}
