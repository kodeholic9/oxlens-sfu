// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-VERSION-ISSUE · 정:IMPL-ROOM-EMIT · 정:IMPL-ROOM-PERMISSION · 정:RULE-AFF-ONE-SET · 연:ROOM.type.Version · 연:ROOM.type.MemberInfo · 연:RULE-ROOM-HIDDEN · 정:RULE-ROOM-SERVER-SCOPE · model: claude-opus-5-5
//! ROOM — 방 그릇: 정체 · 수명(TTL 둘) · 명단(등록 기준 — 투명 포함) · 권한 오버라이드 표 · 판번호(증가와 통지 enqueue 를 한 임계 구역에서) · 슬롯 둘 · 발언권 제어기.

pub mod core;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use common::bus::Notice;
use dashmap::DashMap;
use oxsig::room::{MediaCodecs, MemberInfo, Permission, RoomSummary, Version};
use oxsig::trk::MediaKind;
use serde_json::Value;

use crate::bus::Bus;
use crate::flr::shell::Floor;
use crate::link::{lock, Down};

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub user_id: String,
    pub role: u8,
    pub select: bool,
    pub participant_type: u8,
    pub metadata: Option<Value>,
    pub permission: Permission,
    pub hidden: bool,
    pub session_id: String,
    pub joined_at: u64,
}

impl Member {
    pub fn info(&self) -> MemberInfo {
        MemberInfo {
            user_id: self.user_id.clone(),
            role: self.role,
            select: self.select,
            participant_type: self.participant_type,
            metadata: self.metadata.clone(),
            permission: Some(self.permission),
        }
    }

    pub fn listed(&self) -> MemberInfo {
        let mut i = self.info();
        if self.permission.is_default() {
            i.permission = None;
        }
        i
    }
}

#[derive(Default)]
pub struct Inner {
    pub members: BTreeMap<String, Member>,
    pub overrides: BTreeMap<String, Permission>,
    pub seq: u64,
    pub empty_since: u64,
    pub ever_joined: bool,
    pub stall_cool: BTreeMap<String, u64>,
    pub closed: bool,
}

impl Inner {
    pub fn participants(&self) -> Vec<MemberInfo> {
        self.members.values().filter(|m| !m.hidden).map(Member::listed).collect()
    }

    pub fn user_count(&self) -> u32 {
        self.members.values().filter(|m| !m.hidden).count() as u32
    }

    pub fn rec(&self) -> bool {
        self.members.values().any(|m| m.participant_type == 1)
    }
}

pub struct Slot {
    pub kind: MediaKind,
    pub subs: ArcSwap<Vec<Weak<Down>>>,
    pub speaker: ArcSwapOption<String>,
    pub bound_at: AtomicU64,
    pub priming: AtomicU64,
}

impl Slot {
    fn new(kind: MediaKind) -> Slot {
        Slot { kind, subs: ArcSwap::from_pointee(Vec::new()), speaker: ArcSwapOption::empty(), bound_at: AtomicU64::new(0), priming: AtomicU64::new(0) }
    }

    pub fn attach(&self, d: &Arc<Down>) {
        let w = Arc::downgrade(d);
        self.subs.rcu(|cur| {
            let mut v: Vec<Weak<Down>> = cur.iter().filter(|x| x.strong_count() > 0 && !x.ptr_eq(&w)).cloned().collect();
            v.push(w.clone());
            v
        });
    }

    pub fn detach(&self, d: &Arc<Down>) {
        let w = Arc::downgrade(d);
        self.subs.rcu(|cur| cur.iter().filter(|x| x.strong_count() > 0 && !x.ptr_eq(&w)).cloned().collect::<Vec<_>>());
    }

    pub fn speaker(&self) -> Option<String> {
        self.speaker.load_full().map(|s| s.as_ref().clone())
    }
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub media: MediaCodecs,
    pub unused_ttl: Option<u32>,
    pub departure_ttl: Option<u32>,
    pub epoch: String,
    pub inner: Mutex<Inner>,
    pub floor: Arc<Floor>,
    pub audio: Slot,
    pub video: Slot,
    bus: Arc<Bus>,
}

impl Room {
    pub fn slot(&self, kind: MediaKind) -> &Slot {
        match kind {
            MediaKind::Audio => &self.audio,
            MediaKind::Video => &self.video,
        }
    }

    pub fn version(&self) -> Version {
        Version { epoch: self.epoch.clone(), seq: lock(&self.inner).seq }
    }

    pub fn ver(&self, seq: u64) -> Version {
        Version { epoch: self.epoch.clone(), seq }
    }

    pub fn change<F>(&self, f: F) -> Option<u64>
    where
        F: FnOnce(&mut Inner, Version) -> Option<Notice>,
    {
        let mut g = lock(&self.inner);
        let next = g.seq + 1;
        let n = f(&mut g, self.ver(next))?;
        g.seq = next;
        self.bus.put_room(&self.id, n);
        Some(next)
    }

    pub fn emit(&self, n: Notice) {
        self.bus.put_room(&self.id, n);
    }

    pub fn emit_user(&self, user: &str, n: Notice) {
        self.bus.put_user(user, n);
    }

    pub fn member(&self, user: &str) -> Option<Member> {
        lock(&self.inner).members.get(user).cloned()
    }

    pub fn has(&self, user: &str) -> bool {
        lock(&self.inner).members.contains_key(user)
    }

    pub fn summary(&self) -> RoomSummary {
        let g = lock(&self.inner);
        RoomSummary { room_id: self.id.clone(), name: self.name.clone(), capacity: self.capacity, user_count: g.user_count(), created_at: self.created_at, rec: g.rec() }
    }

    pub fn member_ids(&self) -> Vec<String> {
        lock(&self.inner).members.keys().cloned().collect()
    }
}

pub struct Spec {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub media: MediaCodecs,
    pub unused_ttl: Option<u32>,
    pub departure_ttl: Option<u32>,
}

pub struct Rooms {
    pub map: DashMap<String, Arc<Room>>,
    pub epoch: String,
    pub bus: Arc<Bus>,
    pub queue_max: usize,
    pub t2_secs: u32,
}

impl Rooms {
    pub fn get(&self, id: &str) -> Option<Arc<Room>> {
        self.map.get(id).map(|r| r.clone())
    }

    pub fn create(&self, s: Spec) -> (Arc<Room>, bool) {
        if let Some(r) = self.get(&s.id) {
            return (r, false);
        }
        let now = common::clock::wall_ms();
        let room = Arc::new(Room {
            id: s.id.clone(),
            name: s.name,
            capacity: s.capacity,
            created_at: now,
            media: s.media.filled(),
            unused_ttl: s.unused_ttl,
            departure_ttl: s.departure_ttl,
            epoch: self.epoch.clone(),
            inner: Mutex::new(Inner { empty_since: now.max(1), ..Inner::default() }),
            floor: Floor::new(&s.id, &self.epoch, self.queue_max, self.t2_secs),
            audio: Slot::new(MediaKind::Audio),
            video: Slot::new(MediaKind::Video),
            bus: self.bus.clone(),
        });
        let e = self.map.entry(s.id).or_insert(room);
        (e.clone(), true)
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Room>> {
        self.map.remove(id).map(|(_, r)| r)
    }

    pub fn all(&self) -> Vec<Arc<Room>> {
        self.map.iter().map(|r| r.clone()).collect()
    }

    pub fn count(&self) -> usize {
        self.map.len()
    }

    pub fn seq_now(r: &Room) -> u64 {
        lock(&r.inner).seq
    }

    pub fn tick_empty(r: &Room, now: u64) -> bool {
        let mut g = lock(&r.inner);
        if !g.members.is_empty() {
            g.empty_since = 0;
            return false;
        }
        if g.empty_since == 0 {
            g.empty_since = now;
            return false;
        }
        let ttl = if g.ever_joined { r.departure_ttl } else { r.unused_ttl };
        match ttl {
            None => false,
            Some(t) => now.saturating_sub(g.empty_since) >= t as u64 * 1000,
        }
    }
}

pub fn priming_now(r: &Room) -> u64 {
    r.audio.priming.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: Permission) -> Member {
        Member { user_id: "u".into(), role: 255, select: true, participant_type: 0, metadata: None, permission: p, hidden: false, session_id: "s".into(), joined_at: 0 }
    }

    #[test]
    fn listed_drops_default_permission_but_me_keeps_it() {
        assert_eq!(m(Permission::default()).listed().permission, None, "연:RULE-ROOM-MEMBERINFO-SHAPE 기본이면 명단에 안 싣는다");
        assert!(m(Permission::default()).info().permission.is_some(), "연:CAND-ROOM-ME-APART me 는 권한 포함");
        let low = Permission { floor_request: false, ..Permission::default() };
        assert_eq!(m(low).listed().permission, Some(low));
    }
}
