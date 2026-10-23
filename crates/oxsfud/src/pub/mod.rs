// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:RULE-PUB-TWO-LAYERS · 정:RULE-PUB-STATE · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-RID-SPATIAL · 정:RULE-CONC-SIX · model: claude-opus-5-5
//! PUB — 발행 두 계층: 논리 `PublisherStream`(식별 · 신고 메타) · 물리 `PublisherTrack`(업링크 스트림 하나 — 수신 통계 · RTP 캐시 1024 · NACK · 구독자 목록).

pub mod core;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::ArcSwap;
use bytes::Bytes;
use oxsig::r#pub::{Duplex, Publication};
use oxsig::trk::{MediaKind, Source, TrackEntry, TrackType};

use crate::link::Down;
use crate::rtcp::core::{NackUp, RecvStats};

pub const CACHE: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InExt {
    pub twcc: Option<u8>,
    pub rid: Option<u8>,
    pub rrid: Option<u8>,
    pub mid: Option<u8>,
    pub audio_level: Option<u8>,
}

pub struct PubStream {
    pub track_id: String,
    pub user_id: String,
    pub kind: MediaKind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub simulcast: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub ext: InExt,
    pub half: AtomicBool,
    pub muted: AtomicBool,
    pub listed: AtomicBool,
    pub room: Mutex<String>,
    pub phys: ArcSwap<Vec<Arc<PubTrack>>>,
    pub subs: ArcSwap<Vec<Weak<Down>>>,
    pub first_rtp_at: AtomicU64,
    pub pt_warned: AtomicBool,
    pub last_talk_ms: AtomicU64,
    pub gone: AtomicBool,
}

impl PubStream {
    pub fn duplex(&self) -> Duplex {
        if self.half.load(Ordering::Acquire) { Duplex::Half } else { Duplex::Full }
    }

    pub fn room_id(&self) -> String {
        self.room.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn clock(&self) -> u32 {
        oxsig::link::clock_of(self.codec.as_deref().unwrap_or(if self.kind == MediaKind::Audio { "opus" } else { "H264" }))
    }

    pub fn entry(&self, room_codec: &str) -> TrackEntry {
        let video = self.kind == MediaKind::Video;
        TrackEntry {
            kind_of: TrackType::Individual,
            room_id: self.room_id(),
            track_id: self.track_id.clone(),
            kind: self.kind,
            codec: video.then(|| room_codec.to_string()),
            fmtp: self.fmtp.clone(),
            user_id: Some(self.user_id.clone()),
            source: if video { Some(self.source.unwrap_or(Source::Camera)) } else { None },
            active: Some(!self.half.load(Ordering::Acquire)),
            muted: Some(self.muted.load(Ordering::Acquire)),
            simulcast: video.then_some(self.simulcast),
            scalability: (video && self.simulcast).then(|| "L2T1".to_string()),
        }
    }

    pub fn publication(&self) -> Publication {
        Publication {
            track_id: self.track_id.clone(),
            room_id: self.room_id(),
            kind: self.kind,
            duplex: self.duplex(),
            codec: self.codec.clone(),
            fmtp: self.fmtp.clone(),
            simulcast: self.simulcast,
        }
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

    pub fn track_of_rid(&self, rid: &str) -> Option<Arc<PubTrack>> {
        self.phys.load().iter().find(|p| p.rid.as_deref() == Some(rid)).cloned()
    }

    pub fn add_phys(&self, p: Arc<PubTrack>) {
        self.phys.rcu(|cur| {
            let mut v: Vec<Arc<PubTrack>> = cur.iter().cloned().collect();
            v.push(p.clone());
            v
        });
    }
}

pub struct Cache {
    slots: Vec<Option<(u16, Bytes)>>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { slots: vec![None; CACHE] }
    }
}

impl Cache {
    pub fn put(&mut self, seq: u16, pkt: Bytes) {
        let i = seq as usize % CACHE;
        self.slots[i] = Some((seq, pkt));
    }

    pub fn get(&self, seq: u16) -> Option<Bytes> {
        match &self.slots[seq as usize % CACHE] {
            Some((s, b)) if *s == seq => Some(b.clone()),
            _ => None,
        }
    }
}

pub struct PubTrack {
    pub ssrc: u32,
    pub rid: Option<String>,
    pub stream: Weak<PubStream>,
    pub clock: u32,
    pub stats: Mutex<RecvStats>,
    pub nack: Mutex<NackUp>,
    pub cache: Mutex<Cache>,
    pub sr: Mutex<Option<(crate::fwd::core::Sr, u64)>>,
    pub last_pli_ms: AtomicU64,
    pub packets: AtomicU64,
    pub bytes: AtomicU64,
    pub last_rtp_ms: AtomicU64,
    pub last_key_ms: AtomicU64,
    pub rtx_ssrc: AtomicU64,
    pub layer: AtomicU8,
}

impl PubTrack {
    pub fn new(ssrc: u32, rid: Option<String>, stream: &Arc<PubStream>) -> PubTrack {
        let layer = match rid.as_deref() {
            Some("l") | Some("q") | Some("0") => 0,
            _ => 1,
        };
        PubTrack {
            ssrc,
            rid,
            stream: Arc::downgrade(stream),
            clock: stream.clock(),
            stats: Mutex::new(RecvStats::default()),
            nack: Mutex::new(NackUp::default()),
            cache: Mutex::new(Cache::default()),
            sr: Mutex::new(None),
            last_pli_ms: AtomicU64::new(0),
            packets: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last_rtp_ms: AtomicU64::new(0),
            last_key_ms: AtomicU64::new(0),
            rtx_ssrc: AtomicU64::new(0),
            layer: AtomicU8::new(layer),
        }
    }
}
