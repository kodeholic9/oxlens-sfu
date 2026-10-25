// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-ROOM-LIFE 정:IMPL-ROOM-DERIVED 정:IMPL-ROOM-JOIN 정:IMPL-ROOM-PERMISSION 정:IMPL-ROOM-VERSION-ISSUE 정:IMPL-ROOM-EMIT 정:IMPL-ROOM-STALL 정:CAND-ROOM-TTL-PERMANENT-MARK 정:CAND-ROOM-STALL-COOL-UNIT 정:RULE-CONC-SIX 연:RULE-ROOM-MEMBERINFO-SHAPE 연:RULE-ROOM-PERMISSION-AXIS 연:RULE-ROOM-HIDDEN-ONLY-FOUR 연:RULE-ROOM-SEQ-CONTENT 연:RULE-ROOM-ONE-COUNTER 연:RULE-ROOM-MAX-100 연:RULE-ROOM-BITS-OUTLIVE 연:DEV-ROOM-ME-APART · model: claude-opus-5-5
//! 방 — 정체 + 명단 + 권한 오버라이드 표 + 트랙 목록 + `seq` 하나(한 락) · 수명(unused · departure · 영구 표지) · 파생값은 명단에서 판다.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxsig::msg::{Kind, MediaCodecs, MemberInfo, Permission, RoomSummary, Source, Version};
use serde_json::Value;

pub const DEFAULT_ROLE: u8 = 255;
pub const MAX_ROOMS: usize = 100;
pub const PERMANENT: u32 = u32::MAX;
pub const RECORDER: u8 = 1;
pub const STALL_COOL_MS: u64 = 30_000;

pub fn all_perm() -> Permission {
    Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true }
}

pub fn is_all(p: &Permission) -> bool {
    p.publish_audio && p.publish_video && p.publish_screen && p.floor_request
}

#[derive(Debug, Clone)]
pub struct Member {
    pub user: String,
    pub role: u8,
    pub select: bool,
    pub ptype: u8,
    pub hidden: bool,
    pub metadata: Option<Value>,
    pub perm: Permission,
    pub joined_at: u64,
}

impl Member {
    pub fn info(&self) -> MemberInfo {
        MemberInfo {
            user_id: self.user.clone(),
            role: Some(self.role),
            select: self.select,
            participant_type: self.ptype,
            metadata: self.metadata.clone(),
            permission: if is_all(&self.perm) { None } else { Some(self.perm.clone()) },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Listed {
    pub track_id: String,
    pub user: String,
    pub kind: Kind,
    pub source: Option<Source>,
    pub active: bool,
    pub muted: bool,
    pub simulcast: bool,
    pub fmtp: Option<String>,
}

#[derive(Debug, Default)]
pub struct State {
    pub members: BTreeMap<String, Member>,
    pub overrides: HashMap<String, Permission>,
    pub tracks: Vec<Listed>,
    pub seq: u64,
    pub open: bool,
    pub stall_cool: HashMap<String, u64>,
}

impl State {
    pub fn user_count(&self) -> u32 {
        self.members.values().filter(|m| !m.hidden).count() as u32
    }

    pub fn hidden_count(&self) -> u32 {
        self.members.values().filter(|m| m.hidden).count() as u32
    }

    pub fn rec(&self) -> bool {
        self.members.values().any(|m| m.ptype == RECORDER)
    }

    pub fn participants(&self) -> Vec<MemberInfo> {
        self.members.values().filter(|m| !m.hidden).map(Member::info).collect()
    }

    pub fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn perm_for(&self, user: &str, token: Option<&Permission>) -> Permission {
        self.overrides.get(user).cloned().or_else(|| token.cloned()).unwrap_or_else(all_perm)
    }

    pub fn track(&self, track_id: &str) -> Option<&Listed> {
        self.tracks.iter().find(|t| t.track_id == track_id)
    }

    pub fn take_track(&mut self, track_id: &str) -> Option<Listed> {
        let i = self.tracks.iter().position(|t| t.track_id == track_id)?;
        Some(self.tracks.remove(i))
    }

    pub fn upsert_track(&mut self, l: Listed) {
        match self.tracks.iter_mut().find(|t| t.track_id == l.track_id) {
            Some(t) => *t = l,
            None => self.tracks.push(l),
        }
    }

    pub fn stall_allowed(&mut self, user: &str, now: u64) -> bool {
        let last = self.stall_cool.get(user).copied().unwrap_or(0);
        if last != 0 && now.saturating_sub(last) < STALL_COOL_MS {
            return false;
        }
        self.stall_cool.insert(user.to_string(), now);
        true
    }
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub codecs: MediaCodecs,
    pub unused_ttl: u32,
    pub departure_ttl: u32,
    pub epoch: String,
    pub st: std::sync::Mutex<State>,
    empty_since: AtomicU64,
    ever_joined: AtomicBool,
    pub floor: crate::flr::Floor,
}

pub enum Sweep {
    Live,
    FirstSeenEmpty,
    Waiting,
    Expired,
}

pub struct Spec {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub codecs: MediaCodecs,
    pub unused_ttl: u32,
    pub departure_ttl: u32,
    pub queue_max: u8,
    pub t2_secs: u16,
}

impl Room {
    pub fn new(s: Spec, epoch: &str, now: u64) -> Room {
        let floor = crate::flr::Floor::new(&s.id, s.queue_max, s.t2_secs);
        Room {
            id: s.id,
            name: s.name,
            capacity: s.capacity,
            created_at: now,
            codecs: s.codecs,
            unused_ttl: s.unused_ttl,
            departure_ttl: s.departure_ttl,
            epoch: epoch.to_string(),
            st: std::sync::Mutex::new(State { open: true, ..Default::default() }),
            empty_since: AtomicU64::new(now.max(1)),
            ever_joined: AtomicBool::new(false),
            floor,
        }
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn version(&self, seq: u64) -> Version {
        Version { epoch: self.epoch.clone(), seq }
    }

    pub fn codec_of(&self, kind: Kind) -> &str {
        match kind {
            Kind::Audio => self.codecs.audio.as_deref().unwrap_or("opus"),
            Kind::Video => self.codecs.video.as_deref().unwrap_or("H264"),
        }
    }

    pub fn joined(&self) {
        self.ever_joined.store(true, Ordering::Release);
        self.empty_since.store(0, Ordering::Release);
    }

    pub fn summary(&self) -> RoomSummary {
        let st = self.lock();
        RoomSummary {
            room_id: self.id.clone(),
            name: self.name.clone(),
            capacity: self.capacity,
            user_count: st.user_count(),
            created_at: self.created_at,
            rec: st.rec(),
        }
    }

    pub fn sweep(&self, now: u64) -> Sweep {
        if !self.lock().members.is_empty() {
            return Sweep::Live;
        }
        let since = self.empty_since.load(Ordering::Acquire);
        if since == 0 {
            let _ = self.empty_since.compare_exchange(0, now.max(1), Ordering::AcqRel, Ordering::Acquire);
            return Sweep::FirstSeenEmpty;
        }
        let ttl = if self.ever_joined.load(Ordering::Acquire) { self.departure_ttl } else { self.unused_ttl };
        if ttl == PERMANENT {
            return Sweep::Waiting;
        }
        if now.saturating_sub(since) >= u64::from(ttl) * 1000 { Sweep::Expired } else { Sweep::Waiting }
    }
}

pub enum JoinCheck {
    Ok,
    Full,
}

pub fn capacity_check(st: &State, hidden: bool, capacity: u32) -> JoinCheck {
    if !hidden && st.user_count() >= capacity { JoinCheck::Full } else { JoinCheck::Ok }
}

pub fn parse_version(s: &str) -> Option<(String, u64)> {
    let (e, q) = s.rsplit_once(':')?;
    Some((e.to_string(), q.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(unused: u32, dep: u32) -> Room {
        let s = Spec {
            id: "r1".into(),
            name: "r1".into(),
            capacity: 10,
            codecs: MediaCodecs { audio: Some("opus".into()), video: Some("H264".into()) },
            unused_ttl: unused,
            departure_ttl: dep,
            queue_max: 10,
            t2_secs: 30,
        };
        Room::new(s, "e1", 1000)
    }

    #[test]
    fn ttl_first_tick_only_observes() {
        let r = room(0, 0);
        assert!(matches!(r.sweep(1000), Sweep::Expired), "생성 시각이 기준 · 0초는 다음 sweep 에서 즉시");
        let r = room(PERMANENT, PERMANENT);
        assert!(matches!(r.sweep(10_000_000), Sweep::Waiting), "영구 표지는 거두지 않는다");
        let r = room(60, 0);
        r.joined();
        assert!(matches!(r.sweep(5000), Sweep::FirstSeenEmpty), "모두 퇴장 뒤 첫 tick 은 판정하지 않는다");
        assert!(matches!(r.sweep(5001), Sweep::Expired));
    }

    #[test]
    fn hidden_counts_for_life_not_for_user_count() {
        let r = room(0, 0);
        let mut st = r.lock();
        st.members.insert(
            "bot".into(),
            Member { user: "bot".into(), role: 255, select: false, ptype: 1, hidden: true, metadata: None, perm: all_perm(), joined_at: 0 },
        );
        assert_eq!(st.user_count(), 0);
        assert!(st.rec());
        assert!(st.participants().is_empty());
        drop(st);
        assert!(matches!(r.sweep(9_999_999), Sweep::Live), "S-sym-IMPL-ROOM-LIFE-1 등록 기준 — 투명 녹화만 남은 방은 비어 있지 않다");
    }

    #[test]
    fn version_parse() {
        assert_eq!(parse_version("abc:12"), Some(("abc".into(), 12)));
        assert_eq!(parse_version("abc"), None);
    }
}
