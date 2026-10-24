// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-PUB-SERVER-SCOPE · 정:RULE-PUB-TWO-LAYERS · 정:RULE-PUB-STATE · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-TRACK-SET · 연:PUB.type.Publication · 연:PUB.op.PUBLISH_TRACKS · 연:PUB.op.TRACK_SET · 연:RULE-PUB-SET-PROC · 연:RULE-PUB-SET-FAIL · 연:RULE-PUB-SET-EXCLUSIVE · model: claude-opus-5-5
//! 영역 PUB — 논리 `PubTrack`(식별 · 신고 메타 그대로) · 물리 `Phys`(업링크 스트림 하나 — 수신 통계 · RTP 캐시 1024 · NACK · PLI) · 구독자 목록(`ArcSwap<Vec<Arc<Down>>>`).

pub mod core;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use oxsig::types::Kind;
use oxsig::{Duplex, Publication, Source};

use crate::link::Down;
use crate::rtcp::core::{NackUp, PliGate, RecvStats};

pub const CACHE_SLOTS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Created,
    Intended,
    Active,
}

pub struct RtpCache {
    slots: Vec<Option<(u16, Vec<u8>)>>,
}

impl Default for RtpCache {
    fn default() -> Self {
        RtpCache { slots: vec![None; CACHE_SLOTS] }
    }
}

impl RtpCache {
    pub fn put(&mut self, seq: u16, pkt: &[u8]) {
        let i = seq as usize % CACHE_SLOTS;
        self.slots[i] = Some((seq, pkt.to_vec()));
    }

    pub fn get(&self, seq: u16) -> Option<&[u8]> {
        match &self.slots[seq as usize % CACHE_SLOTS] {
            Some((s, p)) if *s == seq => Some(p),
            _ => None,
        }
    }
}

pub struct Phys {
    pub ssrc: u32,
    pub rtx_ssrc: AtomicU32,
    pub rid: Option<String>,
    pub spatial: Option<u8>,
    pub clock: u32,
    pub stats: Mutex<RecvStats>,
    pub nack: Mutex<NackUp>,
    pub cache: Mutex<RtpCache>,
    pub pli: Mutex<PliGate>,
    pub sr: Mutex<Option<(crate::fwd::core::Sr, u64)>>,
    pub rx_pkts: AtomicU64,
    pub rx_bytes: AtomicU64,
    pub first_rtp_ms: AtomicU64,
    pub last_rtp_ms: AtomicU64,
    pub last_kf_ms: AtomicU64,
    pub rate_bps: AtomicU64,
    rate_win: Mutex<(u64, u64)>,
}

impl Phys {
    pub fn new(ssrc: u32, rtx_ssrc: Option<u32>, rid: Option<String>, spatial: Option<u8>, clock: u32) -> Arc<Phys> {
        Arc::new(Phys {
            ssrc,
            rtx_ssrc: AtomicU32::new(rtx_ssrc.unwrap_or(0)),
            rid,
            spatial,
            clock,
            stats: Mutex::new(RecvStats::default()),
            nack: Mutex::new(NackUp::default()),
            cache: Mutex::new(RtpCache::default()),
            pli: Mutex::new(PliGate::default()),
            sr: Mutex::new(None),
            rx_pkts: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            first_rtp_ms: AtomicU64::new(0),
            last_rtp_ms: AtomicU64::new(0),
            last_kf_ms: AtomicU64::new(0),
            rate_bps: AtomicU64::new(0),
            rate_win: Mutex::new((0, 0)),
        })
    }

    pub fn on_bytes(&self, n: usize, now_ms: u64) {
        self.rx_pkts.fetch_add(1, Ordering::Relaxed);
        self.rx_bytes.fetch_add(n as u64, Ordering::Relaxed);
        let mut w = crate::link::lock(&self.rate_win);
        if w.0 == 0 {
            w.0 = now_ms;
        }
        w.1 += n as u64;
        let span = now_ms.saturating_sub(w.0);
        if span >= 1000 {
            self.rate_bps.store(w.1 * 8 * 1000 / span.max(1), Ordering::Relaxed);
            w.0 = now_ms;
            w.1 = 0;
        }
    }

    pub fn low(&self) -> bool {
        self.spatial == Some(0)
    }
}

pub struct PubTrack {
    pub track_id: String,
    pub user: String,
    pub kind: Kind,
    pub source: Option<Source>,
    pub mid: String,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub simulcast: bool,
    pub decl_ssrc: u32,
    pub decl_rtx_ssrc: Option<u32>,
    pub half: AtomicBool,
    pub muted: AtomicBool,
    pub room: ArcSwap<String>,
    pub phys: ArcSwap<Vec<Arc<Phys>>>,
    pub downs: ArcSwap<Vec<Arc<Down>>>,
    pub state: AtomicU8,
    pub first_rtp_ms: AtomicU64,
    pub last_level_ms: AtomicU64,
    pub pt_mismatch: AtomicBool,
    pub listed: AtomicBool,
    pub ever_full: AtomicBool,
    pub kf_burst_until: AtomicU64,
}

pub fn new_track_id(user: &str) -> String {
    format!("t-{}-{}", user, common::identity::fresh_hex(4))
}

impl PubTrack {
    #[allow(clippy::too_many_arguments)]
    pub fn new(track_id: String, user: &str, kind: Kind, source: Option<Source>, mid: &str, codec: Option<String>, fmtp: Option<String>, pt: u8, rtx_pt: Option<u8>, simulcast: bool, ssrc: u32, rtx_ssrc: Option<u32>, duplex: Duplex, room: &str) -> Arc<PubTrack> {
        Arc::new(PubTrack {
            track_id,
            user: user.to_string(),
            kind,
            source,
            mid: mid.to_string(),
            codec,
            fmtp,
            pt,
            rtx_pt,
            simulcast,
            decl_ssrc: ssrc,
            decl_rtx_ssrc: rtx_ssrc,
            half: AtomicBool::new(duplex == Duplex::Half),
            muted: AtomicBool::new(false),
            room: ArcSwap::from_pointee(room.to_string()),
            phys: ArcSwap::from_pointee(Vec::new()),
            downs: ArcSwap::from_pointee(Vec::new()),
            state: AtomicU8::new(1),
            first_rtp_ms: AtomicU64::new(0),
            last_level_ms: AtomicU64::new(0),
            pt_mismatch: AtomicBool::new(false),
            listed: AtomicBool::new(false),
            ever_full: AtomicBool::new(duplex == Duplex::Full),
            kf_burst_until: AtomicU64::new(0),
        })
    }

    pub fn duplex(&self) -> Duplex {
        if self.half.load(Ordering::Acquire) { Duplex::Half } else { Duplex::Full }
    }

    pub fn room(&self) -> String {
        String::clone(&self.room.load())
    }

    pub fn codec_name(&self) -> &str {
        match self.kind {
            Kind::Audio => self.codec.as_deref().unwrap_or("opus"),
            Kind::Video => self.codec.as_deref().unwrap_or(""),
        }
    }

    pub fn clock(&self) -> u32 {
        crate::link::table::clock_of(self.codec_name())
    }

    pub fn state(&self) -> State {
        match self.state.load(Ordering::Relaxed) {
            0 => State::Created,
            1 => State::Intended,
            _ => State::Active,
        }
    }

    pub fn add_phys(&self, p: Arc<Phys>) {
        self.phys.rcu(|v| {
            let mut v = Vec::clone(v);
            v.retain(|x| x.ssrc != p.ssrc);
            v.push(p.clone());
            v
        });
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

    pub fn detach_all(&self) -> Vec<Arc<Down>> {
        let old = self.downs.swap(Arc::new(Vec::new()));
        Vec::clone(&old)
    }

    pub fn publication(&self) -> Publication {
        Publication {
            track_id: self.track_id.clone(),
            room_id: self.room(),
            kind: self.kind,
            duplex: self.duplex(),
            codec: self.codec.clone(),
            fmtp: self.fmtp.clone(),
            simulcast: self.simulcast,
        }
    }

    pub fn phys_by_spatial(&self, spatial: u8) -> Option<Arc<Phys>> {
        self.phys.load().iter().find(|p| p.spatial == Some(spatial)).cloned()
    }

    pub fn main_phys(&self) -> Option<Arc<Phys>> {
        let v = self.phys.load();
        v.iter().find(|p| p.spatial.unwrap_or(1) == 1).cloned().or_else(|| v.first().cloned())
    }
}
