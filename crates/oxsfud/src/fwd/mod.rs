// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:RULE-FWD-SCOPE 정:RULE-FWD-STEPS 정:RULE-FWD-BRANCH-AUTHORITY 정:IMPL-FWD-ROOM 정:IMPL-FWD-SOURCE-SWITCH 정:RULE-PUB-TWO-LAYERS 정:RULE-CONC-SIX 정:RULE-CONC-PERF-AXES 정:CAND-FWD-REWRITER-LOCK 정:CAND-FWD-PRIME-EPISODE 정:CAND-PUB-INGRESS-STATS-OWNER · model: claude-opus-5-5
//! 전달 — 핫패스 자료(RCU 맵 · 원자값) · 업링크 스트림(물리 · 구독자 목록 `Weak`) · 다운링크 스트림(배정표 줄 하나 · 재기록기 Mutex 하나) · 방마다 슬롯 둘.

pub mod core;
pub mod rtp;
pub mod shell;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use oxsig::msg::Kind;

use crate::fwd::core::Rewriter;
use crate::link::transport::{Path, Role, Transport};
use crate::ops::peer::Peer;
use crate::r#pub::Stream;
use crate::rtcp::core::{NackUp, RecvStats, TwccRecv};
use crate::sub::table::{Class, SubExt};
use crate::sub::{Gate, Prefs};

pub const CACHE: usize = 1024;

pub struct Cache {
    seqs: Vec<Option<u16>>,
    pkts: Vec<Vec<u8>>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { seqs: vec![None; CACHE], pkts: vec![Vec::new(); CACHE] }
    }
}

impl Cache {
    pub fn put(&mut self, seq: u16, pkt: &[u8]) {
        let i = seq as usize % CACHE;
        self.seqs[i] = Some(seq);
        self.pkts[i].clear();
        self.pkts[i].extend_from_slice(pkt);
    }

    pub fn get(&self, seq: u16) -> Option<&[u8]> {
        let i = seq as usize % CACHE;
        (self.seqs[i] == Some(seq)).then(|| self.pkts[i].as_slice())
    }
}

pub struct InSt {
    pub stats: RecvStats,
    pub nack: NackUp,
    pub cache: Cache,
    pub sr: Option<core::Sr>,
}

pub struct Ingress {
    pub ssrc: u32,
    pub layer: u8,
    pub stream: Arc<Stream>,
    pub conn: Weak<Conn>,
    pub subs: ArcSwap<Vec<Weak<Egress>>>,
    pub st: std::sync::Mutex<InSt>,
    pub last_rx_ms: AtomicU64,
    pub last_pli_ms: AtomicU64,
    pub rx_pkts: AtomicU64,
    pub nack_tx: AtomicU64,
    pub pli_tx: AtomicU64,
}

impl Ingress {
    pub fn new(ssrc: u32, layer: u8, stream: Arc<Stream>, conn: Weak<Conn>) -> Ingress {
        let clock = crate::trk::codec::clock(stream.codec_name());
        Ingress {
            ssrc,
            layer,
            stream,
            conn,
            subs: ArcSwap::from_pointee(Vec::new()),
            st: std::sync::Mutex::new(InSt { stats: RecvStats::new(clock), nack: NackUp::default(), cache: Cache::default(), sr: None }),
            last_rx_ms: AtomicU64::new(0),
            last_pli_ms: AtomicU64::new(0),
            rx_pkts: AtomicU64::new(0),
            nack_tx: AtomicU64::new(0),
            pli_tx: AtomicU64::new(0),
        }
    }

    pub fn attach(&self, e: &Arc<Egress>) {
        let cur = self.subs.load();
        if cur.iter().any(|w| w.upgrade().is_some_and(|x| Arc::ptr_eq(&x, e))) {
            return;
        }
        let mut v: Vec<Weak<Egress>> = cur.iter().filter(|w| w.strong_count() > 0).cloned().collect();
        v.push(Arc::downgrade(e));
        self.subs.store(Arc::new(v));
    }

    pub fn detach(&self, e: &Arc<Egress>) {
        let cur = self.subs.load();
        let v: Vec<Weak<Egress>> = cur.iter().filter(|w| w.upgrade().is_some_and(|x| !Arc::ptr_eq(&x, e))).cloned().collect();
        self.subs.store(Arc::new(v));
    }
}

pub struct LoadRef {
    pub room: String,
    pub track_id: String,
    pub user: Option<String>,
    pub stream: Option<Arc<Stream>>,
}

pub struct Rw {
    pub rewriter: Rewriter,
    pub rtx_seq: u16,
    pub await_key: bool,
    pub src: u64,
    pub target_layer: u8,
    pub cur_layer: Option<u8>,
    pub pending_since: u64,
    pub ext_buf: Vec<u8>,
    pub out_buf: Vec<u8>,
}

pub struct Egress {
    pub conn: Weak<Conn>,
    pub user: String,
    pub mid: u8,
    pub mid_str: String,
    pub kind: Kind,
    pub class: Class,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub since: AtomicU64,
    pub load: ArcSwapOption<LoadRef>,
    pub gate: ArcSwapOption<Gate>,
    pub prefs: ArcSwapOption<Prefs>,
    pub rw: std::sync::Mutex<Rw>,
    pub sent: AtomicU64,
    pub last_sent_ms: AtomicU64,
    pub loaded_at: AtomicU64,
    pub rtx_block: AtomicBool,
    pub stall_from: AtomicU64,
    pub stall_count: AtomicU64,
    pub bytes: AtomicU64,
}

impl Egress {
    pub fn new(conn: Weak<Conn>, user: &str, row: &crate::sub::table::Row) -> Egress {
        Egress {
            conn,
            user: user.to_string(),
            mid: row.mid,
            mid_str: row.mid.to_string(),
            kind: row.kind,
            class: row.class,
            ssrc: row.ssrc,
            rtx_ssrc: row.rtx_ssrc,
            pt: AtomicU8::new(row.pt),
            rtx_pt: AtomicU8::new(row.rtx_pt.unwrap_or(0)),
            since: AtomicU64::new(row.since),
            load: ArcSwapOption::empty(),
            gate: ArcSwapOption::empty(),
            prefs: ArcSwapOption::empty(),
            rw: std::sync::Mutex::new(Rw {
                rewriter: Rewriter::default(),
                rtx_seq: 0,
                await_key: true,
                src: 0,
                target_layer: crate::sub::MAX_SPATIAL,
                cur_layer: None,
                pending_since: 0,
                ext_buf: Vec::with_capacity(32),
                out_buf: Vec::with_capacity(1500),
            }),
            sent: AtomicU64::new(0),
            last_sent_ms: AtomicU64::new(0),
            loaded_at: AtomicU64::new(0),
            rtx_block: AtomicBool::new(false),
            stall_from: AtomicU64::new(0),
            stall_count: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    pub fn acked(&self) -> bool {
        let Some(c) = self.conn.upgrade() else { return false };
        self.since.load(Ordering::Acquire) <= c.acked.load(Ordering::Acquire)
    }
}

pub struct Bound {
    pub user: String,
    pub episode: u64,
    pub at_ms: u64,
    pub primed: AtomicBool,
}

pub struct Slot {
    pub kind: Kind,
    pub subs: ArcSwap<Vec<Weak<Egress>>>,
    pub bound: ArcSwapOption<Bound>,
    pub episode: AtomicU64,
    pub switch_at: AtomicU64,
}

impl Slot {
    fn new(kind: Kind) -> Slot {
        Slot { kind, subs: ArcSwap::from_pointee(Vec::new()), bound: ArcSwapOption::empty(), episode: AtomicU64::new(1), switch_at: AtomicU64::new(0) }
    }

    pub fn attach(&self, e: &Arc<Egress>) {
        let cur = self.subs.load();
        if cur.iter().any(|w| w.upgrade().is_some_and(|x| Arc::ptr_eq(&x, e))) {
            return;
        }
        let mut v: Vec<Weak<Egress>> = cur.iter().filter(|w| w.strong_count() > 0).cloned().collect();
        v.push(Arc::downgrade(e));
        self.subs.store(Arc::new(v));
    }

    pub fn detach(&self, e: &Arc<Egress>) {
        let cur = self.subs.load();
        let v: Vec<Weak<Egress>> = cur.iter().filter(|w| w.upgrade().is_some_and(|x| !Arc::ptr_eq(&x, e))).cloned().collect();
        self.subs.store(Arc::new(v));
    }
}

pub struct RoomMedia {
    pub id: String,
    pub audio_codec: String,
    pub video_codec: String,
    pub audio: Slot,
    pub video: Slot,
}

impl RoomMedia {
    pub fn new(id: &str, audio: &str, video: &str) -> RoomMedia {
        RoomMedia {
            id: id.to_string(),
            audio_codec: audio.to_string(),
            video_codec: video.to_string(),
            audio: Slot::new(Kind::Audio),
            video: Slot::new(Kind::Video),
        }
    }

    pub fn slot(&self, kind: Kind) -> &Slot {
        match kind {
            Kind::Audio => &self.audio,
            Kind::Video => &self.video,
        }
    }
}

pub struct Conn {
    pub id: u64,
    pub link: Arc<Transport>,
    pub user: String,
    pub peer: Weak<Peer>,
    pub role: Role,
    pub ingress: ArcSwap<HashMap<u32, Arc<Ingress>>>,
    pub rtx_map: ArcSwap<HashMap<u32, u32>>,
    pub egress: ArcSwap<Vec<Arc<Egress>>>,
    pub acked: AtomicU64,
    pub twcc_out: AtomicU64,
    pub ext: ArcSwap<SubExt>,
    pub twcc_in: std::sync::Mutex<TwccRecv>,
    pub twcc_id: AtomicU8,
    pub media_ssrc: AtomicU32,
    pub unknown_drop: AtomicU64,
    pub rtx_tx: AtomicU64,
    pub nack_rx: AtomicU64,
    pub pli_rx: AtomicU64,
    pub bwe: std::sync::Mutex<crate::sim::Bwe>,
}

impl Conn {
    pub fn ingress_of(&self, ssrc: u32) -> Option<Arc<Ingress>> {
        self.ingress.load().get(&ssrc).cloned()
    }

    pub fn egress_by_ssrc(&self, ssrc: u32) -> Option<Arc<Egress>> {
        self.egress.load().iter().find(|e| e.ssrc == ssrc || e.rtx_ssrc == Some(ssrc)).cloned()
    }

    pub fn add_ingress(&self, i: Arc<Ingress>) {
        let mut m = (**self.ingress.load()).clone();
        m.insert(i.ssrc, i);
        self.ingress.store(Arc::new(m));
    }

    pub fn remove_ingress(&self, stream_track: &str) -> Vec<Arc<Ingress>> {
        let cur = self.ingress.load();
        let gone: Vec<Arc<Ingress>> = cur.values().filter(|i| i.stream.track_id == stream_track).cloned().collect();
        if gone.is_empty() {
            return gone;
        }
        let m: HashMap<u32, Arc<Ingress>> = cur.iter().filter(|(_, i)| i.stream.track_id != stream_track).map(|(k, v)| (*k, v.clone())).collect();
        self.ingress.store(Arc::new(m));
        let rx = self.rtx_map.load();
        let keep: HashMap<u32, u32> = rx.iter().filter(|(_, p)| !gone.iter().any(|g| g.ssrc == **p)).map(|(a, b)| (*a, *b)).collect();
        self.rtx_map.store(Arc::new(keep));
        gone
    }

    pub fn add_rtx(&self, rtx: u32, primary: u32) {
        let mut m = (**self.rtx_map.load()).clone();
        m.insert(rtx, primary);
        self.rtx_map.store(Arc::new(m));
    }

    pub fn set_egress(&self, v: Vec<Arc<Egress>>) {
        self.egress.store(Arc::new(v));
    }
}

#[derive(Default)]
pub struct Plane {
    pub by_path: ArcSwap<HashMap<Path, Arc<Conn>>>,
    pub by_ufrag: ArcSwap<HashMap<String, Arc<Conn>>>,
    pub rooms: ArcSwap<HashMap<String, Arc<RoomMedia>>>,
    w: std::sync::Mutex<()>,
}

impl Plane {
    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.w.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn conn_by_path(&self, p: &Path) -> Option<Arc<Conn>> {
        self.by_path.load().get(p).cloned()
    }

    pub fn conn_by_ufrag(&self, u: &str) -> Option<Arc<Conn>> {
        self.by_ufrag.load().get(u).cloned()
    }

    pub fn room(&self, id: &str) -> Option<Arc<RoomMedia>> {
        self.rooms.load().get(id).cloned()
    }

    pub fn bind_path(&self, p: Path, c: &Arc<Conn>) {
        let _g = self.guard();
        let cur = self.by_path.load();
        if cur.get(&p).is_some_and(|x| Arc::ptr_eq(x, c)) {
            return;
        }
        let mut m = (**cur).clone();
        m.insert(p, c.clone());
        self.by_path.store(Arc::new(m));
    }

    pub fn bind_ufrag(&self, u: &str, c: &Arc<Conn>) {
        let _g = self.guard();
        let mut m = (**self.by_ufrag.load()).clone();
        m.insert(u.to_string(), c.clone());
        self.by_ufrag.store(Arc::new(m));
    }

    pub fn unbind_ufrag(&self, u: &str) {
        let _g = self.guard();
        let mut m = (**self.by_ufrag.load()).clone();
        m.remove(u);
        self.by_ufrag.store(Arc::new(m));
    }

    pub fn unbind_conn(&self, c: &Arc<Conn>) {
        let _g = self.guard();
        let ufrags: HashMap<String, Arc<Conn>> = self.by_ufrag.load().iter().filter(|(_, x)| !Arc::ptr_eq(x, c)).map(|(k, v)| (k.clone(), v.clone())).collect();
        self.by_ufrag.store(Arc::new(ufrags));
        let paths: HashMap<Path, Arc<Conn>> = self.by_path.load().iter().filter(|(_, x)| !Arc::ptr_eq(x, c)).map(|(k, v)| (*k, v.clone())).collect();
        self.by_path.store(Arc::new(paths));
    }

    pub fn unbind_path(&self, p: &Path) {
        let _g = self.guard();
        let mut m = (**self.by_path.load()).clone();
        m.remove(p);
        self.by_path.store(Arc::new(m));
    }

    pub fn add_room(&self, rm: Arc<RoomMedia>) {
        let _g = self.guard();
        let mut m = (**self.rooms.load()).clone();
        m.insert(rm.id.clone(), rm);
        self.rooms.store(Arc::new(m));
    }

    pub fn remove_room(&self, id: &str) {
        let _g = self.guard();
        let mut m = (**self.rooms.load()).clone();
        m.remove(id);
        self.rooms.store(Arc::new(m));
    }

    pub fn conns(&self) -> Vec<Arc<Conn>> {
        let mut seen: Vec<Arc<Conn>> = Vec::new();
        for c in self.by_ufrag.load().values() {
            if !seen.iter().any(|x| Arc::ptr_eq(x, c)) {
                seen.push(c.clone());
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ring() {
        let mut c = Cache::default();
        c.put(5, &[1, 2]);
        assert_eq!(c.get(5), Some(&[1u8, 2][..]));
        c.put(5 + CACHE as u16, &[3]);
        assert_eq!(c.get(5), None, "링버퍼 1024");
    }
}
