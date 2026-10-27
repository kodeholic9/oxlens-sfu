// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:RULE-FWD-INGRESS-STATS · 정:IMPL-FWD-ROOM · 정:IMPL-FLR-RTP-OBSERVE · 정:IMPL-FLR-MEDIA-BIND · 정:RULE-CONC-SIX · 정:RULE-CONC-PERF-AXES · 정:CAND-FWD-REWRITER-LOCK · 정:CAND-FWD-PRIME-EPISODE · model: claude-opus-5-5
//! 핫패스 자료 — 매 패킷 읽는 것은 RCU(`ArcSwap`) · 원자값이다. 다운링크 스트림(배정표 줄 하나) · 방의 슬롯 둘 · 수신 통계. 재기록기만 스트림마다 Mutex 하나(알고 두는 비용).

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use oxsig::types::Kind;
use tokio::sync::mpsc;

use super::core::Rewriter;
use crate::link::Transport;
use crate::rtcp::core::RecvStats;
use crate::sub::ext::ExtTable;

pub const SYNTH_KEY: u64 = 1 << 40;

#[derive(Debug)]
pub struct Carry {
    pub track_id: Arc<str>,
    pub room: Arc<str>,
    pub gate: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct RewriteState {
    pub rw: Rewriter,
    pub src: Option<Weak<crate::r#pub::Uplink>>,
}

pub struct Downlink {
    pub mid: Arc<str>,
    pub kind: Kind,
    pub slot: bool,
    pub ssrc: u32,
    pub rtx_ssrc: u32,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub clock: u32,
    pub user: Arc<str>,
    pub transport: Weak<Transport>,
    pub ext: ArcSwap<ExtTable>,
    pub carry: ArcSwapOption<Carry>,
    pub sendable: AtomicBool,
    pub rewrite: Mutex<RewriteState>,
    pub awaiting_key: AtomicBool,
    pub rtx_gate: AtomicBool,
    pub rtx_seq: AtomicU16,
    pub egress_pkts: AtomicU64,
    pub egress_bytes: AtomicU64,
    pub want_layer: AtomicU8,
    pub auto_layer: AtomicU8,
    pub cur_layer: AtomicU8,
    pub switch_at_ms: AtomicU64,
    pub paused: AtomicBool,
    pub priority: AtomicU8,
    pub last_pli_ms: AtomicU64,
    pub stall_from: AtomicU64,
    pub stall_pkts: AtomicU64,
    pub carry_at_ms: AtomicU64,
}

pub const LAYER_NONE: u8 = 255;

impl Downlink {
    #[allow(clippy::too_many_arguments)]
    pub fn new(mid: u8, kind: Kind, slot: bool, ssrc: u32, rtx_ssrc: u32, pt: u8, rtx_pt: u8, user: Arc<str>, transport: Weak<Transport>, ext: ExtTable) -> Downlink {
        Downlink {
            mid: Arc::from(mid.to_string().as_str()),
            kind,
            slot,
            ssrc,
            rtx_ssrc,
            pt: AtomicU8::new(pt),
            rtx_pt: AtomicU8::new(rtx_pt),
            clock: if kind == Kind::Audio { super::core::OPUS_CLOCK } else { super::core::VIDEO_CLOCK },
            user,
            transport,
            ext: ArcSwap::from_pointee(ext.for_kind(kind)),
            carry: ArcSwapOption::empty(),
            sendable: AtomicBool::new(false),
            rewrite: Mutex::new(RewriteState::default()),
            awaiting_key: AtomicBool::new(kind == Kind::Video),
            rtx_gate: AtomicBool::new(false),
            rtx_seq: AtomicU16::new(0),
            egress_pkts: AtomicU64::new(0),
            egress_bytes: AtomicU64::new(0),
            want_layer: AtomicU8::new(1),
            auto_layer: AtomicU8::new(1),
            cur_layer: AtomicU8::new(LAYER_NONE),
            switch_at_ms: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            priority: AtomicU8::new(0),
            last_pli_ms: AtomicU64::new(0),
            stall_from: AtomicU64::new(0),
            stall_pkts: AtomicU64::new(0),
            carry_at_ms: AtomicU64::new(0),
        }
    }

    pub fn carries(&self, track_id: &str) -> bool {
        self.carry.load().as_ref().is_some_and(|c| &*c.track_id == track_id)
    }
}

#[derive(Debug, Clone)]
pub struct Bound {
    pub user: Arc<str>,
    pub g: u64,
    pub at_ms: u64,
}

pub struct Slot {
    pub kind: Kind,
    pub track_id: Arc<str>,
    pub bound: ArcSwapOption<Bound>,
    pub downlinks: ArcSwap<Vec<Weak<Downlink>>>,
    pub bind: Mutex<crate::flr::bind::Slot>,
    pub episode: AtomicU64,
    pub priming: AtomicBool,
    pub speaker_first_ms: AtomicU64,
    pub speaker_last_ms: AtomicU64,
}

impl Slot {
    pub fn new(kind: Kind, room: &str) -> Slot {
        Slot {
            kind,
            track_id: Arc::from(crate::trk::slot_track_id(room, kind).as_str()),
            bound: ArcSwapOption::empty(),
            downlinks: ArcSwap::from_pointee(Vec::new()),
            bind: Mutex::new(crate::flr::bind::Slot::new()),
            episode: AtomicU64::new(SYNTH_KEY),
            priming: AtomicBool::new(false),
            speaker_first_ms: AtomicU64::new(0),
            speaker_last_ms: AtomicU64::new(0),
        }
    }

    pub fn attach(&self, d: &Arc<Downlink>) {
        let mut v: Vec<Weak<Downlink>> = self.downlinks.load().iter().filter(|w| w.strong_count() > 0).cloned().collect();
        if !v.iter().any(|w| w.as_ptr() == Arc::as_ptr(d)) {
            v.push(Arc::downgrade(d));
        }
        self.downlinks.store(Arc::new(v));
    }

    pub fn detach(&self, d: &Arc<Downlink>) {
        let v: Vec<Weak<Downlink>> = self.downlinks.load().iter().filter(|w| w.strong_count() > 0 && w.as_ptr() != Arc::as_ptr(d)).cloned().collect();
        self.downlinks.store(Arc::new(v));
    }

    pub fn live(&self) -> Vec<Arc<Downlink>> {
        self.downlinks.load().iter().filter_map(Weak::upgrade).collect()
    }

    pub fn new_episode(&self) -> u64 {
        self.episode.fetch_add(1, Ordering::AcqRel) + 1
    }
}

#[derive(Debug, Clone)]
pub enum FloorIn {
    Rtp(Arc<str>),
}

pub struct RoomHot {
    pub id: Arc<str>,
    pub audio: Slot,
    pub video: Slot,
    pub audio_codec: Arc<str>,
    pub video_codec: Arc<str>,
    pub floor_tx: mpsc::UnboundedSender<FloorIn>,
}

impl RoomHot {
    pub fn slot(&self, kind: Kind) -> &Slot {
        match kind {
            Kind::Audio => &self.audio,
            Kind::Video => &self.video,
        }
    }
}

#[derive(Debug)]
pub struct Ingress {
    pub stats: RecvStats,
    pub last_sr: Option<(u32, u64)>,
    pub packets: u64,
    pub bytes: u64,
    pub sr: Option<crate::fwd::core::SrIn>,
}

impl Ingress {
    pub fn new(clock: u32) -> Ingress {
        Ingress { stats: RecvStats::new(clock), last_sr: None, packets: 0, bytes: 0, sr: None }
    }
}
