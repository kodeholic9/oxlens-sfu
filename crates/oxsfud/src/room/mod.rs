// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-ROOM-LIFE · 정:IMPL-ROOM-DERIVED · 정:IMPL-ROOM-VERSION-ISSUE · 정:IMPL-ROOM-EMIT · 정:IMPL-ROOM-PERMISSION · 연:RULE-ROOM-HIDDEN · 연:ROOM.type.MemberInfo · model: claude-opus-5-5
//! 방 — 정체 · 명단(등록 기준 — 투명 포함) · 권한 오버라이드 표 · version(seq 증가와 통지 enqueue 를 한 임계 구역에서) · 수명(TTL 둘 · 영구 표지) · 발언권 판정 로직 · 슬롯 둘.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use common::envelope::Notice;
use oxsig::Op;
use oxsig::msg::{MediaCodecs, MemberInfo, ParticipantEventBody, ParticipantEventType, Permission, RoomSummary, Version};

use crate::flr::core::Floor;
use crate::fwd::SlotFan;

pub const PERMANENT: u32 = u32::MAX;
pub const SWEEP_MS: u64 = 5_000;

#[derive(Debug, Clone)]
pub struct Member {
    pub user: String,
    pub role: Option<u8>,
    pub select: bool,
    pub participant_type: u8,
    pub hidden: bool,
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
}

impl Member {
    pub fn info(&self) -> MemberInfo {
        MemberInfo {
            user_id: self.user.clone(),
            role: self.role,
            select: self.select,
            participant_type: self.participant_type,
            metadata: self.metadata.clone(),
            permission: if self.permission.is_default() { None } else { Some(self.permission) },
        }
    }

    pub fn me(&self) -> MemberInfo {
        let mut i = self.info();
        i.permission = Some(self.permission);
        i
    }
}

#[derive(Debug, Default)]
pub struct RoomState {
    pub members: BTreeMap<String, Member>,
    pub seq: u64,
    pub overrides: HashMap<String, Permission>,
    pub empty_since: u64,
    pub ever_joined: bool,
    pub empty_seen: bool,
    pub closed: bool,
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub unused_ttl: u32,
    pub departure_ttl: u32,
    pub media_codecs: MediaCodecs,
    pub epoch: String,
    pub st: Mutex<RoomState>,
    pub floor: Mutex<Floor>,
    pub slots: [Arc<SlotFan>; 2],
    pub stall_cool: Mutex<HashMap<String, u64>>,
}

pub struct RoomInit {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub unused_ttl: u32,
    pub departure_ttl: u32,
    pub media_codecs: MediaCodecs,
    pub epoch: String,
    pub floor: Floor,
}

impl Room {
    pub fn new(i: RoomInit) -> Arc<Room> {
        let now = common::now_ms();
        Arc::new(Room {
            id: i.id,
            name: i.name,
            capacity: i.capacity,
            created_at: now,
            unused_ttl: i.unused_ttl,
            departure_ttl: i.departure_ttl,
            media_codecs: i.media_codecs,
            epoch: i.epoch,
            st: Mutex::new(RoomState { seq: 0, empty_since: now, ..Default::default() }),
            floor: Mutex::new(i.floor),
            slots: [Arc::new(SlotFan::new(true)), Arc::new(SlotFan::new(false))],
            stall_cool: Mutex::new(HashMap::new()),
        })
    }

    pub fn slot(&self, kind: oxsig::msg::Kind) -> &Arc<SlotFan> {
        match kind {
            oxsig::msg::Kind::Audio => &self.slots[0],
            oxsig::msg::Kind::Video => &self.slots[1],
        }
    }

    pub fn remove(&self, user: &str) -> Option<Member> {
        self.st.lock().ok().and_then(|mut s| s.members.remove(user))
    }

    pub fn announce_left(&self, user: &str) -> u64 {
        self.bump_and_send(|v| {
            let body = ParticipantEventBody {
                type_: ParticipantEventType::Left,
                room_id: self.id.clone(),
                user_id: user.to_string(),
                role: None,
                select: None,
                participant_type: None,
                metadata: None,
                permission: None,
                version: v,
            };
            Some(notice(Op::ParticipantEvent, &body, vec![user.to_string()], &self.id))
        })
    }

    pub fn closed(&self) -> bool {
        self.st.lock().map(|s| s.closed).unwrap_or(true)
    }

    pub fn close(&self) {
        if let Ok(mut s) = self.st.lock() {
            s.closed = true;
        }
    }

    pub fn version(&self) -> Version {
        let seq = self.st.lock().map(|s| s.seq).unwrap_or(0);
        Version { epoch: self.epoch.clone(), seq }
    }

    pub fn ver(&self, seq: u64) -> Version {
        Version { epoch: self.epoch.clone(), seq }
    }

    pub fn member(&self, user: &str) -> Option<Member> {
        self.st.lock().ok().and_then(|s| s.members.get(user).cloned())
    }

    pub fn has(&self, user: &str) -> bool {
        self.st.lock().map(|s| s.members.contains_key(user)).unwrap_or(false)
    }

    pub fn registered(&self) -> usize {
        self.st.lock().map(|s| s.members.len()).unwrap_or(0)
    }

    pub fn user_count(&self) -> u32 {
        self.st.lock().map(|s| s.members.values().filter(|m| !m.hidden).count() as u32).unwrap_or(0)
    }

    pub fn hidden_count(&self) -> u32 {
        self.st.lock().map(|s| s.members.values().filter(|m| m.hidden).count() as u32).unwrap_or(0)
    }

    pub fn rec(&self) -> bool {
        self.st.lock().map(|s| s.members.values().any(|m| m.participant_type == 1)).unwrap_or(false)
    }

    pub fn participants(&self) -> Vec<MemberInfo> {
        self.st.lock().map(|s| s.members.values().filter(|m| !m.hidden).map(|m| m.info()).collect()).unwrap_or_default()
    }

    pub fn users(&self) -> Vec<String> {
        self.st.lock().map(|s| s.members.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn summary(&self) -> RoomSummary {
        RoomSummary {
            room_id: self.id.clone(),
            name: self.name.clone(),
            capacity: self.capacity,
            user_count: self.user_count(),
            created_at: self.created_at,
            rec: self.rec(),
        }
    }

    pub fn permission_for(&self, user: &str, token: Option<Permission>) -> Permission {
        self.st
            .lock()
            .ok()
            .and_then(|s| s.overrides.get(user).copied())
            .or(token)
            .unwrap_or_default()
    }

    pub fn full_for(&self, hidden: bool) -> bool {
        !hidden && self.user_count() >= self.capacity
    }

    pub fn join(&self, mut m: Member, rejoin: bool, token_perm: Option<Permission>) -> u64 {
        let Ok(mut s) = self.st.lock() else { return 0 };
        m.permission = s.overrides.get(&m.user).copied().or(token_perm).unwrap_or_default();
        s.ever_joined = true;
        s.empty_since = 0;
        s.empty_seen = false;
        let notify = !rejoin && !m.hidden;
        let info = m.info();
        let user = m.user.clone();
        s.members.insert(user.clone(), m);
        if notify {
            s.seq += 1;
            let body = ParticipantEventBody {
                type_: ParticipantEventType::Joined,
                room_id: self.id.clone(),
                user_id: user.clone(),
                role: info.role,
                select: Some(info.select),
                participant_type: Some(info.participant_type),
                metadata: info.metadata.clone(),
                permission: info.permission,
                version: Version { epoch: self.epoch.clone(), seq: s.seq },
            };
            crate::bus::room_notice(&self.id, notice(Op::ParticipantEvent, &body, vec![user], &self.id));
        }
        s.seq
    }

    pub fn bump_and_send<F>(&self, f: F) -> u64
    where
        F: FnOnce(Version) -> Option<Notice>,
    {
        let Ok(mut s) = self.st.lock() else { return 0 };
        s.seq += 1;
        let v = Version { epoch: self.epoch.clone(), seq: s.seq };
        if let Some(n) = f(v) {
            crate::bus::room_notice(&self.id, n);
        }
        s.seq
    }

    pub fn set_permission(&self, user: &str, p: Permission) -> Option<(Permission, Permission, bool)> {
        let Ok(mut s) = self.st.lock() else { return None };
        let hidden = s.members.get(user)?.hidden;
        let old = s.members.get(user).map(|m| m.permission)?;
        if old == p {
            return Some((old, p, hidden));
        }
        if let Some(m) = s.members.get_mut(user) {
            m.permission = p;
        }
        if p.is_default() {
            s.overrides.remove(user);
        } else {
            s.overrides.insert(user.to_string(), p);
        }
        Some((old, p, hidden))
    }

    pub fn ttl_due(&self, now: u64) -> bool {
        let Ok(mut s) = self.st.lock() else { return false };
        if !s.members.is_empty() {
            s.empty_since = 0;
            s.empty_seen = false;
            return false;
        }
        if s.ever_joined && s.empty_since == 0 {
            s.empty_since = now;
            s.empty_seen = true;
            return false;
        }
        let ttl = if s.ever_joined { self.departure_ttl } else { self.unused_ttl };
        if ttl == PERMANENT {
            return false;
        }
        now.saturating_sub(s.empty_since) >= ttl as u64 * 1000
    }
}

pub fn notice<T: serde::Serialize>(op: Op, body: &T, exclude: Vec<String>, room: &str) -> Notice {
    Notice {
        op: op.as_u16(),
        body: serde_json::to_value(body).unwrap_or_default(),
        exclude,
        room_id: Some(room.to_string()),
        sfu_id: crate::bus::get().map(|b| b.epoch.clone()),
    }
}

#[derive(Default)]
pub struct Rooms {
    pub map: Mutex<HashMap<String, Arc<Room>>>,
}

impl Rooms {
    pub fn get(&self, id: &str) -> Option<Arc<Room>> {
        self.map.lock().ok().and_then(|g| g.get(id).cloned())
    }

    pub fn all(&self) -> Vec<Arc<Room>> {
        self.map.lock().map(|g| g.values().cloned().collect()).unwrap_or_default()
    }

    pub fn insert(&self, r: Arc<Room>) -> Arc<Room> {
        let Ok(mut g) = self.map.lock() else { return r };
        g.entry(r.id.clone()).or_insert(r).clone()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<Room>> {
        self.map.lock().ok().and_then(|mut g| g.remove(id))
    }
}
