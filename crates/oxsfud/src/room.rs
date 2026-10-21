// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-ROOM-LIFE 정:IMPL-ROOM-DERIVED 정:IMPL-ROOM-VERSION-ISSUE 정:IMPL-ROOM-EMIT 정:IMPL-ROOM-RESYNC 정:IMPL-TRK-FROM-PUB 연:RULE-ROOM-SEQ-CONTENT 연:RULE-ROOM-ONE-COUNTER 연:RULE-ROOM-HIDDEN-ONLY-FOUR 연:RULE-TRK-FIELDS-BY-TYPE · model: claude-opus-5-5
//! 방 — 빈 그릇과 수명 · 명단 · 트랙 목록 · `seq` 와 통지 enqueue 를 한 임계 구역에서.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use arc_swap::{ArcSwap, ArcSwapOption};
use oxsig::Op;
use oxsig::bplane::UnitEvent;
use oxsig::code::Code;
use oxsig::wire::{
    CreateRoomReq, CreateRoomRes, GetRoomRes, Kind, MediaCodecs, MemberInfo, Permission, RoomSnapshot, RoomSummary,
    TrackEntry, TrackEntryType, Version,
};

use crate::media::{PubStream, Slot};
use crate::sfu::{Out, Sfu, lock, now_ms};
use crate::table::slot_track_id;

pub const ALL: Permission = Permission { publish_audio: true, publish_video: true, publish_screen: true, floor_request: true };

#[derive(Debug, Clone)]
pub struct Member {
    pub user_id: String,
    pub session_id: String,
    pub role: u8,
    pub select: bool,
    pub participant_type: u8,
    pub metadata: Option<serde_json::Value>,
    pub permission: Permission,
    pub hidden: bool,
}

impl Member {
    pub fn info(&self) -> MemberInfo {
        MemberInfo {
            user_id: self.user_id.clone(),
            role: Some(self.role),
            select: self.select,
            participant_type: self.participant_type,
            metadata: self.metadata.clone(),
            permission: (self.permission != ALL).then_some(self.permission),
        }
    }
}

#[derive(Default)]
pub struct Inner {
    pub members: BTreeMap<String, Member>,
    pub perm: HashMap<String, Permission>,
    pub tracks: Vec<Arc<PubStream>>,
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub capacity: u32,
    pub created_at: u64,
    pub codecs: MediaCodecs,
    pub unused_ttl: Option<u32>,
    pub departure_ttl: Option<u32>,
    pub inner: Mutex<Inner>,
    pub seq: AtomicU64,
    pub empty_since: AtomicU64,
    pub ever_joined: AtomicBool,
    pub floor: Mutex<crate::floor::FloorRoom>,
    pub holder: ArcSwapOption<String>,
    pub prev_speaker: ArcSwapOption<String>,
    pub slots: [Slot; 2],
    pub listed: ArcSwap<Vec<Arc<PubStream>>>,
    pub cool: Mutex<HashMap<String, u64>>,
    pub closed: AtomicBool,
}

impl Room {
    pub fn slot(&self, kind: Kind) -> &Slot {
        match kind {
            Kind::Audio => &self.slots[0],
            Kind::Video => &self.slots[1],
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    pub fn version(&self, sfu: &Sfu) -> Version {
        Version { epoch: sfu.epoch.clone(), seq: self.seq.load(Ordering::SeqCst) }
    }

    pub fn listed_tracks(&self) -> Vec<Arc<PubStream>> {
        self.listed.load().iter().cloned().collect()
    }

    pub fn publish_listed(&self, g: &Inner) {
        self.listed.store(Arc::new(g.tracks.clone()));
    }

    pub fn emit<B: serde::Serialize>(&self, sfu: &Sfu, _g: &Inner, op: Op, exclude: Option<&str>, body: impl FnOnce(Version) -> B) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let b = body(Version { epoch: sfu.epoch.clone(), seq });
        let ev = UnitEvent { op: op.as_u16(), body: serde_json::to_string(&b).unwrap_or_default(), exclude: exclude.map(str::to_string) };
        let _ = sfu.out_tx.send(Out::Room { room: self.id.clone(), ev });
    }

    pub fn bare<B: serde::Serialize>(&self, sfu: &Sfu, _g: &Inner, op: Op, exclude: Option<&str>, body: &B) {
        let ev = UnitEvent { op: op.as_u16(), body: serde_json::to_string(body).unwrap_or_default(), exclude: exclude.map(str::to_string) };
        let _ = sfu.out_tx.send(Out::Room { room: self.id.clone(), ev });
    }

    pub fn visible(&self, g: &Inner) -> Vec<MemberInfo> {
        g.members.values().filter(|m| !m.hidden).map(Member::info).collect()
    }

    pub fn user_count(&self, g: &Inner) -> u32 {
        g.members.values().filter(|m| !m.hidden).count() as u32
    }

    pub fn rec(&self, g: &Inner) -> bool {
        g.members.values().any(|m| m.participant_type == 1)
    }

    pub fn slot_entries(&self) -> Vec<TrackEntry> {
        let vcodec = self.codecs.video.clone().unwrap_or_else(|| "H264".into());
        vec![
            TrackEntry {
                type_: TrackEntryType::Slot,
                room_id: self.id.clone(),
                track_id: slot_track_id(&self.id, false),
                kind: Kind::Audio,
                codec: None,
                fmtp: None,
                user_id: None,
                source: None,
                active: None,
                muted: None,
                simulcast: None,
                scalability: None,
            },
            TrackEntry {
                type_: TrackEntryType::Slot,
                room_id: self.id.clone(),
                track_id: slot_track_id(&self.id, true),
                kind: Kind::Video,
                codec: Some(vcodec.clone()),
                fmtp: crate::rtp::sub_fmtp(&vcodec).map(str::to_string),
                user_id: None,
                source: None,
                active: None,
                muted: None,
                simulcast: None,
                scalability: None,
            },
        ]
    }

    pub fn tracks(&self, g: &Inner) -> Vec<TrackEntry> {
        let mut v = self.slot_entries();
        v.extend(g.tracks.iter().filter(|s| s.listed.load(Ordering::Relaxed)).map(|s| entry(self, s)));
        v
    }

    pub fn snapshot(&self, sfu: &Sfu, me: &str) -> Option<RoomSnapshot> {
        let g = self.lock();
        let me = g.members.get(me)?.info();
        Some(RoomSnapshot { participants: self.visible(&g), me, tracks: self.tracks(&g), version: self.version(sfu) })
    }

    pub fn summary(&self) -> RoomSummary {
        let g = self.lock();
        RoomSummary {
            room_id: self.id.clone(),
            name: self.name.clone(),
            capacity: self.capacity,
            user_count: self.user_count(&g),
            created_at: self.created_at,
            rec: self.rec(&g),
        }
    }
}

pub fn entry(room: &Room, s: &PubStream) -> TrackEntry {
    let video = s.is_video();
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: room.id.clone(),
        track_id: s.track_id.clone(),
        kind: s.kind,
        codec: video.then(|| room.codecs.video.clone().unwrap_or_else(|| "H264".into())),
        fmtp: s.fmtp.clone(),
        user_id: Some(s.user_id.clone()),
        source: if video { Some(s.source.unwrap_or(oxsig::wire::Source::Camera)) } else { None },
        active: Some(!s.is_half()),
        muted: Some(s.muted.load(Ordering::Relaxed)),
        simulcast: video.then_some(s.simulcast),
        scalability: (video && s.simulcast).then(|| "L2T1".to_string()),
    }
}

pub fn remove_entry(room: &Room, s: &PubStream) -> TrackEntry {
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: room.id.clone(),
        track_id: s.track_id.clone(),
        kind: s.kind,
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

fn filled_codecs(c: Option<&MediaCodecs>) -> MediaCodecs {
    let pick = |v: Option<&String>, d: &str| match v {
        Some(s) if !s.is_empty() => crate::rtp::canon_codec(s),
        _ => d.to_string(),
    };
    MediaCodecs { audio: Some(pick(c.and_then(|c| c.audio.as_ref()), "opus")), video: Some(pick(c.and_then(|c| c.video.as_ref()), "H264")) }
}

pub fn create(sfu: &Arc<Sfu>, req: &CreateRoomReq, capacity: u32, unused: u32, departure: u32) -> Result<CreateRoomRes, (Code, String)> {
    let id = req.room_id.clone().filter(|s| !s.is_empty()).ok_or((Code::MISSING_FIELD, "room_id".to_string()))?;
    if let Some(r) = sfu.room(&id) {
        return Ok(CreateRoomRes { room_id: r.id.clone(), name: r.name.clone(), capacity: r.capacity, created_at: r.created_at, media_codecs: r.codecs.clone() });
    }
    let codecs = filled_codecs(req.media_codecs.as_ref());
    if let Some(v) = codecs.video.as_deref()
        && !crate::rtp::has_keyframe_judge(v)
    {
        return Err((Code::CODEC_REQUIRED, format!("unsupported video codec {v}")));
    }
    let now = now_ms();
    let room = Arc::new(Room {
        id: id.clone(),
        name: req.name.clone(),
        capacity,
        created_at: now,
        codecs: codecs.clone(),
        unused_ttl: Some(req.unused_ttl_secs.unwrap_or(unused)),
        departure_ttl: Some(req.departure_ttl_secs.unwrap_or(departure)),
        inner: Mutex::new(Inner::default()),
        seq: AtomicU64::new(1),
        empty_since: AtomicU64::new(now),
        ever_joined: AtomicBool::new(false),
        floor: Mutex::new(crate::floor::FloorRoom::new(sfu.policy.floor.queue_max as usize)),
        holder: ArcSwapOption::empty(),
        prev_speaker: ArcSwapOption::empty(),
        slots: [Slot::new(), Slot::new()],
        listed: ArcSwap::from_pointee(Vec::new()),
        cool: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
    });
    let won = {
        let mut m = lock(&sfu.rooms);
        match m.get(&id) {
            Some(r) => r.clone(),
            None => {
                m.insert(id.clone(), room.clone());
                room.clone()
            }
        }
    };
    if Arc::ptr_eq(&won, &room) {
        sfu.room_token(&id, true);
        tracing::info!(kind = "ev", spec = "IMPL-ROOM-LIFE", room = id.as_str(), "room created");
    }
    Ok(CreateRoomRes { room_id: won.id.clone(), name: won.name.clone(), capacity: won.capacity, created_at: won.created_at, media_codecs: won.codecs.clone() })
}

pub fn get(sfu: &Arc<Sfu>, room_id: &str, viewer: Option<&str>, tracks: bool) -> Result<GetRoomRes, (Code, String)> {
    let r = sfu.room(room_id).ok_or((Code::ROOM_NOT_FOUND, "room not found".to_string()))?;
    let g = r.lock();
    let me = viewer.and_then(|v| g.members.get(v)).map(Member::info);
    Ok(GetRoomRes {
        room_id: r.id.clone(),
        name: r.name.clone(),
        capacity: r.capacity,
        user_count: r.user_count(&g),
        participants: r.visible(&g),
        me,
        created_at: r.created_at,
        media_codecs: r.codecs.clone(),
        version: r.version(sfu),
        tracks: tracks.then(|| r.tracks(&g)),
    })
}

pub fn list(sfu: &Arc<Sfu>) -> Vec<RoomSummary> {
    let rooms: Vec<Arc<Room>> = lock(&sfu.rooms).values().cloned().collect();
    rooms.iter().map(|r| r.summary()).collect()
}

pub fn remove(sfu: &Arc<Sfu>, room: &Arc<Room>) {
    room.closed.store(true, Ordering::SeqCst);
    let gone = {
        let mut m = lock(&sfu.rooms);
        match m.get(&room.id) {
            Some(r) if Arc::ptr_eq(r, room) => m.remove(&room.id).is_some(),
            _ => false,
        }
    };
    for s in &room.slots {
        s.rows.store(Arc::new(Vec::new()));
        s.bound.store(None);
    }
    lock(&room.cool).clear();
    if gone {
        sfu.room_token(&room.id, false);
        tracing::info!(kind = "ev", spec = "IMPL-ROOM-LIFE", room = room.id.as_str(), "room removed");
    }
}

pub fn sweep(sfu: &Arc<Sfu>) {
    let now = now_ms();
    let rooms: Vec<Arc<Room>> = lock(&sfu.rooms).values().cloned().collect();
    for r in rooms {
        let empty = r.lock().members.is_empty();
        if !empty {
            continue;
        }
        let since = r.empty_since.load(Ordering::SeqCst);
        if since == 0 {
            let _ = r.empty_since.compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
            continue;
        }
        let ttl = if r.ever_joined.load(Ordering::SeqCst) { r.departure_ttl } else { r.unused_ttl };
        let Some(ttl) = ttl else { continue };
        if now.saturating_sub(since) >= ttl as u64 * 1000 {
            let still_empty = r.lock().members.is_empty();
            if still_empty {
                remove(sfu, &r);
            }
        }
    }
}
