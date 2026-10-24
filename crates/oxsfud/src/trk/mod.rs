// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-TRK-FROM-PUB · 정:IMPL-FLR-MEDIA-BIND · 정:CAND-FWD-PRIME-SOURCE · 연:RULE-TRK-PUB-TO-TRACK · 연:RULE-TRK-SLOT-LIFE · 연:TRK.type.TrackEntry · 연:RULE-TRK-TRACK-STATE-TO-ALL · 연:R-TRK-MUTED-IN-SNAPSHOT · 연:RULE-TRK-THREE-TIERS · 연:RULE-TRK-TRACK-EVENT-ONE-ROOM · 연:TRK.S.pub_full · 연:TRK.S.half_to_full · 연:TRK.S.full_to_half · 연:TRK.S.unpub_full · 연:TRK.S.unpub_resting · 연:TRK.S.rest_in · 연:TRK.S.mute · model: claude-opus-5-5
//! 영역 TRK — 트랙 목록 항목(individual · slot) · 슬롯 둘(방이 생길 때 · 방 것 · 화자 결합 · 예열 · 구독 줄 목록).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use oxsig::types::Kind;
use oxsig::{MediaCodecs, TrackEntry, TrackEntryType};

use crate::link::Down;
use crate::r#pub::PubTrack;

pub const PRIME_FRAMES: u32 = 250;
pub const SILENCE_FRAMES: u32 = 3;
pub const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const PRIME_SRC: u64 = 0xffff_0000_0000_0001;

pub fn slot_id(room: &str, kind: Kind) -> String {
    format!("ptt-{room}-{}", kind.as_str())
}

pub struct Slot {
    pub room: String,
    pub kind: Kind,
    pub track_id: String,
    pub codec: String,
    pub fmtp: Option<String>,
    pub speaker: ArcSwap<Option<(String, Arc<PubTrack>)>>,
    pub holder: ArcSwap<Option<String>>,
    pub last_holder: ArcSwap<Option<String>>,
    pub downs: ArcSwap<Vec<Arc<Down>>>,
    pub priming: AtomicU32,
    pub silence: AtomicU32,
    pub prime: Mutex<(u16, u32)>,
    pub bound_at: AtomicU64,
    pub first_rtp: AtomicBool,
    pub last_rtp_ms: AtomicU64,
    pub rtx_gate: AtomicBool,
    pub prime_epoch: AtomicU64,
}

impl Slot {
    pub fn new(room: &str, kind: Kind, media: &MediaCodecs) -> Arc<Slot> {
        let codec = match kind {
            Kind::Audio => media.audio.clone().unwrap_or_else(|| "opus".into()),
            Kind::Video => media.video.clone().unwrap_or_else(|| "H264".into()),
        };
        let fmtp = (kind == Kind::Video && codec.eq_ignore_ascii_case("H264")).then(|| "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string());
        Arc::new(Slot {
            room: room.to_string(),
            kind,
            track_id: slot_id(room, kind),
            codec,
            fmtp,
            speaker: ArcSwap::from_pointee(None),
            holder: ArcSwap::from_pointee(None),
            last_holder: ArcSwap::from_pointee(None),
            downs: ArcSwap::from_pointee(Vec::new()),
            priming: AtomicU32::new(0),
            silence: AtomicU32::new(0),
            prime: Mutex::new((0, 0)),
            bound_at: AtomicU64::new(0),
            first_rtp: AtomicBool::new(false),
            last_rtp_ms: AtomicU64::new(0),
            rtx_gate: AtomicBool::new(false),
            prime_epoch: AtomicU64::new(0),
        })
    }

    pub fn prime_src(&self) -> u64 {
        PRIME_SRC.wrapping_add(self.prime_epoch.load(Ordering::Acquire) << 8)
    }

    pub fn entry(&self) -> TrackEntry {
        TrackEntry {
            type_: TrackEntryType::Slot,
            room_id: self.room.clone(),
            track_id: self.track_id.clone(),
            kind: self.kind,
            codec: (self.kind == Kind::Video).then(|| self.codec.clone()),
            fmtp: if self.kind == Kind::Video { self.fmtp.clone() } else { None },
            user_id: None,
            source: None,
            active: None,
            muted: None,
            simulcast: None,
            scalability: None,
        }
    }

    pub fn speaker_user(&self) -> Option<String> {
        self.speaker.load().as_ref().as_ref().map(|(u, _)| u.clone())
    }

    pub fn bind(&self, user: &str, track: Option<Arc<PubTrack>>, prime: bool, now_ms: u64) {
        self.holder.store(Arc::new(Some(user.to_string())));
        match track {
            Some(t) => self.speaker.store(Arc::new(Some((user.to_string(), t)))),
            None => self.speaker.store(Arc::new(None)),
        }
        self.bound_at.store(now_ms, Ordering::Release);
        self.prime_epoch.fetch_add(1, Ordering::AcqRel);
        self.first_rtp.store(false, Ordering::Release);
        self.silence.store(0, Ordering::Release);
        self.priming.store(if prime && self.kind == Kind::Audio && self.codec.eq_ignore_ascii_case("opus") { PRIME_FRAMES } else { 0 }, Ordering::Release);
        self.rtx_gate.store(true, Ordering::Release);
    }

    pub fn release(&self) {
        let was_audio_opus = self.kind == Kind::Audio && self.codec.eq_ignore_ascii_case("opus");
        self.speaker.store(Arc::new(None));
        self.prime_epoch.fetch_add(1, Ordering::AcqRel);
        let prev = self.holder.swap(Arc::new(None));
        self.last_holder.store(prev);
        self.priming.store(0, Ordering::Release);
        self.rtx_gate.store(true, Ordering::Release);
        if was_audio_opus {
            self.silence.store(SILENCE_FRAMES, Ordering::Release);
        }
    }

    pub fn attach(&self, d: &Arc<Down>) {
        self.downs.rcu(|v| {
            let mut v = Vec::clone(v);
            if !v.iter().any(|x| Arc::ptr_eq(x, d)) {
                v.push(d.clone());
            }
            v
        });
    }

    pub fn detach(&self, d: &Arc<Down>) {
        self.downs.rcu(|v| {
            let mut v = Vec::clone(v);
            v.retain(|x| !Arc::ptr_eq(x, d));
            v
        });
    }

    pub fn detach_user(&self, user: &str) -> Vec<Arc<Down>> {
        let mut gone = Vec::new();
        self.downs.rcu(|v| {
            let mut v = Vec::clone(v);
            gone = v.iter().filter(|d| d.user == user).cloned().collect();
            v.retain(|d| d.user != user);
            v
        });
        gone
    }
}

pub fn entry_of(t: &PubTrack, room: &str, active: bool, room_video: Option<&str>) -> TrackEntry {
    let video = t.kind == Kind::Video;
    TrackEntry {
        type_: TrackEntryType::Individual,
        room_id: room.to_string(),
        track_id: t.track_id.clone(),
        kind: t.kind,
        codec: if video { room_video.map(str::to_string).or_else(|| t.codec.clone()) } else { None },
        fmtp: t.fmtp.clone(),
        user_id: Some(t.user.clone()),
        source: if video { t.source.or(Some(oxsig::Source::Camera)) } else { None },
        active: Some(active),
        muted: Some(t.muted.load(Ordering::Relaxed)),
        simulcast: if video { Some(t.simulcast) } else { None },
        scalability: if video && t.simulcast { Some("L2T1".to_string()) } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_entry_shape() {
        let s = Slot::new("r1", Kind::Video, &MediaCodecs { audio: Some("opus".into()), video: Some("H264".into()) });
        let e = s.entry();
        assert_eq!(e.track_id, "ptt-r1-video");
        assert!(e.user_id.is_none() && e.active.is_none() && e.muted.is_none());
        assert_eq!(e.codec.as_deref(), Some("H264"));
        let a = Slot::new("r1", Kind::Audio, &MediaCodecs { audio: None, video: None });
        assert!(a.entry().codec.is_none() && a.entry().fmtp.is_none());
    }
}
