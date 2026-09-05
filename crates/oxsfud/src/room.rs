// author: kodeholic (powered by Claude)
//! 방·명단 — 정§4-1 방 수명 전이 · §4-2 `RoomMember`. 명단의 정본이 여기다(hub 의 것은 배달용 사본).

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use dashmap::DashMap;
use oxsig::body::room::PARTICIPANT_RECORDER;
use oxsig::schema::{MemberInfo, Version};
use serde::Serialize;

use crate::media::floor::{FloorController, T2_DEFAULT_SECS};
use crate::media::slot::SlotSet;
use crate::version::Seq;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub role: u8,
    pub select: bool,
    pub participant_type: u8,
    pub joined_at_ms: u64,
}

impl Member {
    pub fn is_recorder(&self) -> bool {
        self.participant_type == PARTICIPANT_RECORDER
    }
}

/// 정§4-1 — 정체 + 수명 + 명단 + `seq`. 발언권·슬롯은 뒤 판.
pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at_ms: u64,
    pub unused_ttl_secs: Option<u64>,
    pub departure_ttl_secs: Option<u64>,
    members: Mutex<BTreeMap<String, Member>>,
    pub seq: Seq,
    /// 정§14-1 — `seq` 증가와 그 통지의 enqueue 는 이 락 안에서(방 단위).
    pub gate: Mutex<()>,
    /// 0 = 사람 있음. 생성 직후는 `created_at`(unused 기준 시각).
    empty_since_ms: AtomicU64,
    ever_joined: AtomicBool,
    /// 정§8-1 — 반이중은 방 공용 m-line 을 돌려쓴다. audio 는 방과 수명이 같고 video 는 첫 화자가 만든다.
    pub slots: SlotSet,
    /// 정§9 — 방마다 발언권 제어기 하나.
    pub floor: FloorController,
}

/// 연§5-3 목록 항목.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub user_count: u32,
    pub created_at: u64,
    pub rec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSpec {
    pub room_id: String,
    pub name: String,
    pub capacity: u32,
    pub unused_ttl_secs: Option<u64>,
    pub departure_ttl_secs: Option<u64>,
}

impl Room {
    pub fn new(spec: RoomSpec, now_ms: u64) -> Self {
        let slots = SlotSet::new(&spec.room_id);
        let floor = FloorController::new(&spec.room_id, T2_DEFAULT_SECS);
        Self {
            slots,
            floor,
            id: spec.room_id,
            name: spec.name,
            capacity: spec.capacity,
            created_at_ms: now_ms,
            unused_ttl_secs: spec.unused_ttl_secs,
            departure_ttl_secs: spec.departure_ttl_secs,
            members: Mutex::new(BTreeMap::new()),
            seq: Seq::default(),
            gate: Mutex::new(()),
            empty_since_ms: AtomicU64::new(now_ms),
            ever_joined: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Member>> {
        self.members.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 녹화 참가자 제외 인원(연§5-3 `user_count`).
    pub fn user_count(&self) -> u32 {
        self.lock().values().filter(|m| !m.is_recorder()).count() as u32
    }
    pub fn has_recorder(&self) -> bool {
        self.lock().values().any(Member::is_recorder)
    }
    pub fn is_member(&self, user_id: &str) -> bool {
        self.lock().contains_key(user_id)
    }
    pub fn member(&self, user_id: &str) -> Option<Member> {
        self.lock().get(user_id).cloned()
    }
    pub fn is_occupied(&self) -> bool {
        !self.lock().is_empty()
    }
    /// 연§4-4 명단 — recorder 는 투명이라 뺀다.
    pub fn participants(&self) -> Vec<MemberInfo> {
        self.lock().iter().filter(|(_, m)| !m.is_recorder()).map(|(u, m)| MemberInfo { user_id: u.clone(), role: m.role, select: m.select }).collect()
    }
    /// 명단 스냅샷 — 배관 산출이 쓴다(§2-3 계약 4: 순회와 삭제를 겹치지 않는다).
    pub fn member_ids(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    /// 정§4-2 ③ 정원 — recorder 는 점유하지 않는다. `>=` 이상.
    pub fn is_full_for(&self, participant_type: u8) -> bool {
        participant_type != PARTICIPANT_RECORDER && self.user_count() >= self.capacity
    }

    /// 정§4-2 ⑥ 등록 — 첫 입장 표식(`ever_joined`·`empty_since=0`). 이미 있으면 false(재입장은 ② 축출이 먼저다).
    pub fn insert(&self, user_id: &str, m: Member) -> bool {
        let mut g = self.lock();
        if g.contains_key(user_id) {
            return false;
        }
        g.insert(user_id.to_owned(), m);
        self.ever_joined.store(true, Ordering::Release);
        self.empty_since_ms.store(0, Ordering::Release);
        true
    }

    /// 정§17-2 ② 명단 제거 — 없으면 false(거짓 성공 금지).
    pub fn remove(&self, user_id: &str) -> Option<Member> {
        self.lock().remove(user_id)
    }

    pub fn version(&self, epoch: &str) -> Version {
        self.seq.current(epoch)
    }

    pub fn summary(&self) -> RoomSummary {
        RoomSummary { room_id: self.id.clone(), name: self.name.clone(), capacity: self.capacity, user_count: self.user_count(), created_at: self.created_at_ms, rec: self.has_recorder() }
    }

    /// 정§4-1 tick — 빈 방의 유예·만료 판정. `true` = 만료(폭파 대상). "방금 비었다" tick 은 CAS 만 하고 판정하지 않는다.
    pub fn tick_expired(&self, now_ms: u64) -> bool {
        if self.is_occupied() {
            return false;
        }
        if self.empty_since_ms.compare_exchange(0, now_ms, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            return false;
        }
        let since = self.empty_since_ms.load(Ordering::Acquire);
        let ttl = if self.ever_joined.load(Ordering::Acquire) { self.departure_ttl_secs } else { self.unused_ttl_secs };
        ttl.is_some_and(|t| now_ms.saturating_sub(since) >= t * 1000)
    }
}

/// 방 등록부 — 생성(멱등)·조회·목록·삭제·sweep.
#[derive(Default)]
pub struct RoomRegistry {
    rooms: DashMap<String, std::sync::Arc<Room>>,
}

pub enum Created {
    New(std::sync::Arc<Room>),
    Existing(std::sync::Arc<Room>),
}

impl RoomRegistry {
    /// 연§5-4 멱등 — 있으면 그 방(name 무시).
    pub fn create(&self, spec: RoomSpec, now_ms: u64) -> Created {
        match self.rooms.entry(spec.room_id.clone()) {
            dashmap::Entry::Occupied(e) => Created::Existing(e.get().clone()),
            dashmap::Entry::Vacant(v) => Created::New(v.insert(std::sync::Arc::new(Room::new(spec, now_ms))).clone()),
        }
    }
    pub fn get(&self, room_id: &str) -> Option<std::sync::Arc<Room>> {
        self.rooms.get(room_id).map(|r| r.clone())
    }
    pub fn list(&self) -> Vec<RoomSummary> {
        let mut v: Vec<RoomSummary> = self.rooms.iter().map(|r| r.summary()).collect();
        v.sort_by(|a, b| a.room_id.cmp(&b.room_id));
        v
    }
    /// 정§4-1 만료·명시 삭제 ① — 사람 있으면 `None`(`3004` 는 호출자 몫).
    /// 스냅샷 — 타이머 tick 이 방마다 돈다(§2-3 계약 4).
    pub fn all(&self) -> Vec<std::sync::Arc<Room>> {
        self.rooms.iter().map(|e| e.clone()).collect()
    }

    pub fn remove_if_unoccupied(&self, room_id: &str) -> Option<std::sync::Arc<Room>> {
        self.rooms.remove_if(room_id, |_, r| !r.is_occupied()).map(|(_, r)| r)
    }
    /// 만료된 방 id — 폭파는 호출자가 `remove_if_unoccupied` + 통보 한 쌍으로.
    pub fn sweep(&self, now_ms: u64) -> Vec<String> {
        self.rooms.iter().filter(|r| r.tick_expired(now_ms)).map(|r| r.id.clone()).collect()
    }
    pub fn len(&self) -> usize {
        self.rooms.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxsig::body::room::PARTICIPANT_USER;

    fn spec(id: &str, unused: Option<u64>, departure: Option<u64>) -> RoomSpec {
        RoomSpec { room_id: id.into(), name: "n".into(), capacity: 2, unused_ttl_secs: unused, departure_ttl_secs: departure }
    }
    fn user(select: bool) -> Member {
        Member { role: 255, select, participant_type: PARTICIPANT_USER, joined_at_ms: 1 }
    }

    #[test]
    fn create_is_idempotent_and_name_ignored() {
        let reg = RoomRegistry::default();
        assert!(matches!(reg.create(spec("r", None, None), 10), Created::New(_)));
        let mut s = spec("r", None, None);
        s.name = "other".into();
        match reg.create(s, 20) {
            Created::Existing(r) => assert_eq!((r.name.as_str(), r.created_at_ms), ("n", 10)),
            Created::New(_) => panic!("must be idempotent"),
        }
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn capacity_counts_listeners_not_recorders() {
        let r = Room::new(spec("r", None, None), 0);
        assert!(r.insert("a", user(false)));
        assert!(!r.insert("a", user(true)));
        assert!(r.insert("rec", Member { participant_type: PARTICIPANT_RECORDER, ..user(true) }));
        assert!(!r.is_full_for(PARTICIPANT_USER));
        assert!(r.insert("b", user(true)));
        assert!(r.is_full_for(PARTICIPANT_USER));
        assert!(!r.is_full_for(PARTICIPANT_RECORDER));
        assert_eq!((r.user_count(), r.has_recorder(), r.participants().len()), (2, true, 2));
    }

    #[test]
    fn unused_ttl_counts_from_creation() {
        let r = Room::new(spec("r", Some(10), Some(1)), 1_000);
        assert!(!r.tick_expired(5_000));
        assert!(!r.tick_expired(10_999));
        assert!(r.tick_expired(11_000));
    }

    #[test]
    fn departure_ttl_first_tick_observes_only() {
        let r = Room::new(spec("r", Some(0), Some(0)), 0);
        r.insert("a", user(true));
        assert!(!r.tick_expired(100));
        r.remove("a");
        assert!(!r.tick_expired(200), "first empty tick only records");
        assert!(r.tick_expired(200), "0s TTL expires on the next sweep");
        r.insert("a", user(true));
        assert!(!r.tick_expired(300), "re-entry cancels the grace");
        let r2 = Room::new(spec("r2", None, None), 0);
        r2.insert("a", user(true));
        r2.remove("a");
        r2.tick_expired(1);
        assert!(!r2.tick_expired(u64::MAX / 2), "None = permanent");
    }

    #[test]
    fn registry_sweep_and_guarded_remove() {
        let reg = RoomRegistry::default();
        reg.create(spec("gone", Some(0), None), 1_000);
        reg.create(spec("busy", Some(0), None), 1_000);
        reg.get("busy").unwrap().insert("a", user(true));
        assert_eq!(reg.sweep(1_001), vec!["gone".to_owned()]);
        assert!(reg.remove_if_unoccupied("busy").is_none());
        assert!(reg.remove_if_unoccupied("gone").is_some());
        assert_eq!(reg.list().len(), 1);
    }
}
