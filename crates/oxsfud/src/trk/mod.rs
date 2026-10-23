// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-TRK-FROM-PUB · 정:IMPL-ROOM-EMIT · 연:RULE-TRK-PUB-TO-TRACK · 연:RULE-TRK-SLOT-LIFE · 연:TRK.op.TRACK_EVENT · 연:TRK.op.TRACK_STATE · 연:RULE-TRK-TRACK-EVENT-ONE-ROOM · model: claude-opus-5-5
//! TRK — 방마다 트랙 목록(슬롯 둘은 방과 수명이 같다 · 전이중 개인 트랙 · 쉬는 항목) · 바뀌면 그 방 `seq++` 와 `TRACK_EVENT` / `TRACK_STATE` 를 한 임계 구역에서.

use std::sync::{Arc, Mutex};

use common::bus::Notice;
use dashmap::DashMap;
use oxsig::op;
use oxsig::trk::{slot_track_id, MediaKind, TrackAction, TrackEntry, TrackEvent, TrackState, TrackType};

use crate::link::lock;
use crate::room::Room;

#[derive(Default)]
pub struct Tracks {
    map: DashMap<String, Arc<Mutex<Vec<TrackEntry>>>>,
}

fn notice(op_code: u16, body: impl serde::Serialize) -> Option<Notice> {
    let body = serde_json::to_value(body).ok()?;
    Some(Notice { op: op_code, body, exclude: Vec::new(), room_id: None, sfu_id: None, control: None })
}

impl Tracks {
    fn of(&self, room: &str) -> Arc<Mutex<Vec<TrackEntry>>> {
        self.map.entry(room.to_string()).or_default().clone()
    }

    pub fn create_slots(&self, room: &Room) {
        let l = self.of(&room.id);
        let mut g = lock(&l);
        if g.iter().any(|t| t.kind_of == TrackType::Slot) {
            return;
        }
        for kind in [MediaKind::Audio, MediaKind::Video] {
            let video = kind == MediaKind::Video;
            g.push(TrackEntry {
                kind_of: TrackType::Slot,
                room_id: room.id.clone(),
                track_id: slot_track_id(&room.id, kind),
                kind,
                codec: video.then(|| room.media.video().to_string()),
                fmtp: if video { crate::r#pub::core::room_fmtp(&room.media, kind).map(str::to_string) } else { None },
                user_id: None,
                source: None,
                active: None,
                muted: None,
                simulcast: None,
                scalability: None,
            });
        }
    }

    pub fn list(&self, room: &str) -> Vec<TrackEntry> {
        self.map.get(room).map(|l| lock(&l).clone()).unwrap_or_default()
    }

    pub fn snapshot(&self, room: &Room) -> (Vec<TrackEntry>, oxsig::room::Version) {
        let g = lock(&room.inner);
        (self.list(&room.id), room.ver(g.seq))
    }

    pub fn put(&self, room: &Room, entry: TrackEntry) -> Option<u64> {
        let l = self.of(&room.id);
        room.change(|_, v| {
            let mut g = lock(&l);
            match g.iter_mut().find(|t| t.track_id == entry.track_id) {
                Some(t) if *t == entry => return None,
                Some(t) => t.clone_from(&entry),
                None => g.push(entry.clone()),
            }
            notice(op::TRACK_EVENT, TrackEvent { action: TrackAction::Add, room_id: room.id.clone(), tracks: vec![entry.clone()], version: v })
        })
    }

    pub fn remove(&self, room: &Room, track_id: &str) -> Option<u64> {
        let l = self.of(&room.id);
        room.change(|_, v| {
            let mut g = lock(&l);
            let i = g.iter().position(|t| t.track_id == track_id && t.kind_of == TrackType::Individual)?;
            let t = g.remove(i);
            notice(op::TRACK_EVENT, TrackEvent { action: TrackAction::Remove, room_id: room.id.clone(), tracks: vec![t.removal()], version: v })
        })
    }

    pub fn has(&self, room: &str, track_id: &str) -> bool {
        self.map.get(room).map(|l| lock(&l).iter().any(|t| t.track_id == track_id)).unwrap_or(false)
    }

    pub fn entry(&self, room: &str, track_id: &str) -> Option<TrackEntry> {
        self.map.get(room).and_then(|l| lock(&l).iter().find(|t| t.track_id == track_id).cloned())
    }

    pub fn mute(&self, room: &Room, track_id: &str, user: &str, kind: MediaKind, muted: bool) -> Option<u64> {
        let l = self.of(&room.id);
        room.change(|_, v| {
            let mut g = lock(&l);
            let t = g.iter_mut().find(|t| t.track_id == track_id)?;
            if t.muted == Some(muted) {
                return None;
            }
            t.muted = Some(muted);
            notice(op::TRACK_STATE, TrackState { kind_of: "muted".to_string(), user_id: user.to_string(), track_id: track_id.to_string(), kind, room_id: room.id.clone(), version: v, muted })
        })
    }

    pub fn drop_room(&self, room: &str) {
        self.map.remove(room);
    }

    pub fn of_user(&self, room: &str, user: &str) -> Vec<TrackEntry> {
        self.list(room).into_iter().filter(|t| t.user_id.as_deref() == Some(user)).collect()
    }
}
