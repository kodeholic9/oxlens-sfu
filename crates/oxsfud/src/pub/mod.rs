// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-PUB-TWO-LAYERS · 정:RULE-PUB-STATE · 정:IMPL-PUB-ADD · 정:IMPL-PUB-ACCEPT · 정:IMPL-PUB-REMOVE · 정:IMPL-PUB-TRACK-SET · 정:RULE-RTCP-NACK-UP · 정:RULE-FWD-INGRESS-STATS · 정:RULE-PUB-SERVER-SCOPE · 정:CAND-PUB-RID-EXTRA · 정:CAND-PUB-INGRESS-STATS-OWNER · model: claude-opus-5-5
//! 발행 — 논리 `PublisherStream`(track_id · 메타 · 구독 줄 목록) · 물리 `PublisherTrack`(업링크 SSRC · 캐시 1024 · NACK 결손 · PLI 스로틀 · 수신 통계) · 시뮬캐스트는 첫 RTP 에 rid 로 붙인다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use bytes::Bytes;
use oxsig::msg::{Duplex, Kind, Source};

use crate::fwd::{DownRow, Rtp, ext_get};
use crate::link::{ExtIds, Transport};
use crate::rtcp::core::RecvStats;

pub const CACHE: usize = 1024;
pub const NACK_RESEND_MS: u64 = 200;
pub const NACK_GIVEUP_MS: u64 = 1_000;
pub const NACK_MAX: usize = 128;
pub const NACK_JUMP: i64 = 128;
pub const PER_REQUEST: usize = 8;
pub const PER_PERSON: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishState {
    Created,
    Intended,
    Active,
}

impl PublishState {
    pub fn as_str(self) -> &'static str {
        match self {
            PublishState::Created => "Created",
            PublishState::Intended => "Intended",
            PublishState::Active => "Active",
        }
    }
}

pub struct StreamInit {
    pub track_id: String,
    pub user: String,
    pub session: String,
    pub kind: Kind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub simulcast: bool,
    pub duplex: Duplex,
    pub room: String,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
}

pub struct PubStream {
    pub track_id: String,
    pub user: String,
    pub session: String,
    pub kind: Kind,
    pub mid: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub codec: Option<String>,
    pub fmtp: Option<String>,
    pub source: Option<Source>,
    pub simulcast: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub csrc: u32,
    duplex: AtomicU8,
    pub muted: AtomicBool,
    pub room: ArcSwap<String>,
    pub rows: ArcSwap<Vec<Arc<DownRow>>>,
    pub tracks: Mutex<Vec<Arc<PubTrack>>>,
    pub first_rtp_ms: AtomicU64,
    pub listed: AtomicBool,
    pub state: Mutex<PublishState>,
    pub unplaced: AtomicBool,
    pub unplaced_by: Mutex<BTreeSet<String>>,
}

static CSRC: AtomicU32 = AtomicU32::new(1);

pub fn next_csrc() -> u32 {
    loop {
        let v = CSRC.fetch_add(1, Ordering::Relaxed) & 0x7fff_ffff;
        if v != 0 {
            return v;
        }
    }
}

impl PubStream {
    pub fn new(i: StreamInit) -> Arc<PubStream> {
        Arc::new(PubStream {
            track_id: i.track_id,
            user: i.user,
            session: i.session,
            kind: i.kind,
            mid: i.mid,
            pt: i.pt,
            rtx_pt: i.rtx_pt,
            codec: i.codec,
            fmtp: i.fmtp,
            source: i.source,
            simulcast: i.simulcast,
            ssrc: i.ssrc,
            rtx_ssrc: i.rtx_ssrc,
            csrc: next_csrc(),
            duplex: AtomicU8::new(if i.duplex == Duplex::Half { 1 } else { 0 }),
            muted: AtomicBool::new(false),
            room: ArcSwap::from_pointee(i.room),
            rows: ArcSwap::from_pointee(Vec::new()),
            tracks: Mutex::new(Vec::new()),
            first_rtp_ms: AtomicU64::new(0),
            listed: AtomicBool::new(false),
            state: Mutex::new(PublishState::Created),
            unplaced: AtomicBool::new(false),
            unplaced_by: Mutex::new(BTreeSet::new()),
        })
    }

    pub fn is_half(&self) -> bool {
        self.duplex.load(Ordering::Acquire) == 1
    }

    pub fn duplex(&self) -> Duplex {
        if self.is_half() { Duplex::Half } else { Duplex::Full }
    }

    pub fn set_duplex(&self, d: Duplex) {
        self.duplex.store(if d == Duplex::Half { 1 } else { 0 }, Ordering::Release);
    }

    pub fn codec_name(&self) -> &str {
        match self.kind {
            Kind::Audio => "opus",
            Kind::Video => self.codec.as_deref().unwrap_or(""),
        }
    }

    pub fn current(&self) -> PublishState {
        self.state.lock().map(|g| *g).unwrap_or(PublishState::Created)
    }

    pub fn set_state(&self, s: PublishState) -> Option<PublishState> {
        let mut g = self.state.lock().ok()?;
        if *g == s {
            return None;
        }
        Some(std::mem::replace(&mut g, s))
    }

    pub fn physical(&self) -> Vec<Arc<PubTrack>> {
        self.tracks.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[derive(Default)]
pub struct Cache {
    slots: Vec<Option<(u16, Bytes)>>,
}

impl Cache {
    pub fn put(&mut self, seq: u16, b: Bytes) {
        if self.slots.is_empty() {
            self.slots = vec![None; CACHE];
        }
        let i = seq as usize % CACHE;
        self.slots[i] = Some((seq, b));
    }

    pub fn get(&self, seq: u16) -> Option<Bytes> {
        let i = seq as usize % CACHE;
        match self.slots.get(i) {
            Some(Some((s, b))) if *s == seq => Some(b.clone()),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct NackUp {
    pub last: Option<u16>,
    pub missing: BTreeMap<i64, (u64, Option<u64>)>,
    base: i64,
    pub sent: u64,
    pub pkts: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrival {
    First,
    Newer,
    Filled,
    Seen,
}

impl Arrival {
    pub fn name(self) -> &'static str {
        match self {
            Arrival::First => "first",
            Arrival::Newer => "newer",
            Arrival::Filled => "filled",
            Arrival::Seen => "seen",
        }
    }

    pub fn repair_passes(self) -> bool {
        self != Arrival::Seen
    }
}

pub enum Rtx {
    Original(u16, Vec<u8>),
    Padding,
    Malformed,
}

pub fn rtx_original(plain: &[u8], hlen: usize, pt: u8, ssrc: u32) -> Rtx {
    if hlen < 12 || plain.len() < hlen {
        return Rtx::Malformed;
    }
    let pad = if plain[0] & 0x20 != 0 { plain[plain.len() - 1] as usize } else { 0 };
    if plain[0] & 0x20 != 0 && (pad == 0 || hlen + pad > plain.len()) {
        return Rtx::Malformed;
    }
    if plain.len() - hlen - pad < 2 {
        return Rtx::Padding;
    }
    let osn = u16::from_be_bytes([plain[hlen], plain[hlen + 1]]);
    let mut out = Vec::with_capacity(plain.len());
    out.extend_from_slice(&plain[..hlen]);
    out[1] = (out[1] & 0x80) | (pt & 0x7f);
    out[2..4].copy_from_slice(&osn.to_be_bytes());
    out[8..12].copy_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&plain[hlen + 2..]);
    Rtx::Original(osn, out)
}

impl NackUp {
    pub fn on_packet(&mut self, seq: u16, now: u64) -> Arrival {
        let Some(l) = self.last else {
            self.last = Some(seq);
            self.base = seq as i64;
            return Arrival::First;
        };
        let u = self.base + (seq.wrapping_sub(l) as i16) as i64;
        let diff = u - self.base;
        if diff <= 0 {
            return if self.missing.remove(&u).is_some() { Arrival::Filled } else { Arrival::Seen };
        }
        if diff > 1 && diff < NACK_JUMP {
            for s in (self.base + 1)..u {
                if self.missing.len() >= NACK_MAX {
                    break;
                }
                self.missing.entry(s).or_insert((now, None));
            }
        }
        self.last = Some(seq);
        self.base = u;
        Arrival::Newer
    }

    pub fn holes(&self) -> Vec<u16> {
        self.missing.keys().map(|s| s.rem_euclid(65_536) as u16).collect()
    }

    pub fn due(&mut self, now: u64) -> (Vec<u16>, bool) {
        let mut out = Vec::new();
        let mut gave_up = false;
        self.missing.retain(|s, v| {
            if now.saturating_sub(v.0) > NACK_GIVEUP_MS {
                gave_up = true;
                return false;
            }
            if v.1.map(|t| now.saturating_sub(t) >= NACK_RESEND_MS).unwrap_or(true) {
                out.push(s.rem_euclid(65_536) as u16);
                v.1 = Some(now);
            }
            true
        });
        self.sent += out.len() as u64;
        self.pkts += u64::from(!out.is_empty());
        (out, gave_up)
    }
}

#[derive(Default)]
pub struct PliState {
    pub last_ms: u64,
    pub pending: bool,
    pub sent: u64,
}

pub struct PubTrack {
    pub ssrc: u32,
    pub rtx_ssrc: Mutex<Option<u32>>,
    pub rid: Option<u8>,
    pub stream: Arc<PubStream>,
    pub src_key: u64,
    pub recv: Mutex<RecvStats>,
    pub nack: Mutex<NackUp>,
    pub cache: Mutex<Cache>,
    pub pli: Mutex<PliState>,
    pub pli_want: AtomicU8,
    pub last_rtp_ms: AtomicU64,
    pub last_voice_ms: AtomicU64,
    pub voice_run_ms: AtomicU64,
    pub voice_event_ms: AtomicU64,
    pub last_keyframe_ms: AtomicU64,
    pub blocked_last_us: AtomicU64,
    pub pt_checked: AtomicBool,
    pub transport: std::sync::Weak<Transport>,
    pub rate: Mutex<crate::sim::LayerRate>,
}

static SRC: AtomicU64 = AtomicU64::new(1);

impl PubTrack {
    pub fn new(stream: Arc<PubStream>, ssrc: u32, rtx_ssrc: Option<u32>, rid: Option<u8>, tr: &Arc<Transport>) -> Arc<PubTrack> {
        let clock = if stream.kind == Kind::Audio { 48_000 } else { 90_000 };
        Arc::new(PubTrack {
            ssrc,
            rtx_ssrc: Mutex::new(rtx_ssrc),
            rid,
            stream,
            src_key: SRC.fetch_add(1, Ordering::Relaxed),
            recv: Mutex::new(RecvStats::new(clock)),
            nack: Mutex::new(NackUp::default()),
            cache: Mutex::new(Cache::default()),
            pli: Mutex::new(PliState::default()),
            pli_want: AtomicU8::new(0),
            last_rtp_ms: AtomicU64::new(0),
            last_voice_ms: AtomicU64::new(0),
            voice_run_ms: AtomicU64::new(0),
            voice_event_ms: AtomicU64::new(0),
            last_keyframe_ms: AtomicU64::new(0),
            blocked_last_us: AtomicU64::new(0),
            pt_checked: AtomicBool::new(false),
            transport: Arc::downgrade(tr),
            rate: Mutex::new(crate::sim::LayerRate::default()),
        })
    }

    pub fn is_rtx(&self, ssrc: u32) -> bool {
        ssrc != self.ssrc
    }

    pub fn on_ingress(&self, rtp: &Rtp<'_>, now_ms: u64, _at_us: u64, ids: &ExtIds) -> Option<u64> {
        if let Ok(mut r) = self.recv.lock() {
            r.update(rtp.seq, rtp.ts, now_ms, rtp.raw.len());
        }
        if self.stream.kind == Kind::Video
            && let Ok(mut n) = self.nack.lock()
        {
            n.on_packet(rtp.seq, now_ms);
        }
        if let Ok(mut c) = self.cache.lock() {
            c.put(rtp.seq, Bytes::copy_from_slice(rtp.raw));
        }
        if let Ok(mut r) = self.rate.lock() {
            r.add(now_ms, rtp.raw.len() as u64);
        }
        self.last_rtp_ms.store(now_ms, Ordering::Release);
        if self.stream.first_rtp_ms.compare_exchange(0, now_ms, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            crate::ops::first_rtp(&self.stream);
        }
        if !self.pt_checked.swap(true, Ordering::AcqRel) && rtp.pt != self.stream.pt {
            tracing::warn!(
                kind = "diag",
                spec = "SV-IMPL-PUB-ACCEPT",
                session = %self.stream.session,
                user = %self.stream.user,
                track = %self.stream.track_id,
                "in.declared" = self.stream.pt,
                "in.first" = rtp.pt,
                "[PT:MISMATCH] declared {} first rtp {}",
                self.stream.pt,
                rtp.pt
            );
        }
        if self.stream.kind == Kind::Audio
            && let Some(v) = ext_get(rtp.ext, ids.audio_level).and_then(|v| v.first())
            && (v & 0x7f) < 50
        {
            return Some(self.last_voice_ms.swap(now_ms, Ordering::AcqRel));
        }
        None
    }

    pub fn on_repair(&self, rtp: &Rtp<'_>) -> Arrival {
        let arrival = match self.nack.lock() {
            Ok(mut n) => n.on_packet(rtp.seq, common::now_ms()),
            Err(_) => return Arrival::Seen,
        };
        if arrival.repair_passes()
            && let Ok(mut c) = self.cache.lock()
        {
            c.put(rtp.seq, Bytes::copy_from_slice(rtp.raw));
        }
        arrival
    }

    pub fn blocked_notify(&self, at_us: u64) -> bool {
        let prev = self.blocked_last_us.load(Ordering::Relaxed);
        if prev == 0 || at_us.saturating_sub(prev) >= crate::fwd::BLOCKED_NOTIFY_US {
            self.blocked_last_us.store(at_us, Ordering::Relaxed);
            return true;
        }
        false
    }

    pub fn unwrap_rtx(&self, plain: &[u8], hlen: usize) -> Rtx {
        rtx_original(plain, hlen, self.stream.pt, self.ssrc)
    }
}

pub struct PubCtx {
    pub user: String,
    pub streams: Mutex<BTreeMap<String, Arc<PubStream>>>,
}

impl PubCtx {
    fn fresh(user: &str) -> Arc<PubCtx> {
        Arc::new(PubCtx { user: user.to_string(), streams: Mutex::new(BTreeMap::new()) })
    }

    pub fn streams(&self) -> Vec<Arc<PubStream>> {
        self.streams.lock().map(|g| g.values().cloned().collect()).unwrap_or_default()
    }

    pub fn active(&self) -> usize {
        self.streams.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn find(&self, track_id: &str) -> Option<Arc<PubStream>> {
        self.streams.lock().ok().and_then(|g| g.get(track_id).cloned())
    }

    pub fn by_ssrc(&self, ssrc: u32) -> Option<Arc<PubStream>> {
        self.streams.lock().ok().and_then(|g| g.values().find(|s| s.ssrc == ssrc).cloned())
    }

    pub fn has_half(&self) -> bool {
        self.streams.lock().map(|g| g.values().any(|s| s.is_half())).unwrap_or(false)
    }
}

#[derive(Default)]
pub struct Pubs {
    pub map: Mutex<HashMap<String, Arc<PubCtx>>>,
}

impl Pubs {
    pub fn ctx(&self, user: &str) -> Arc<PubCtx> {
        let Ok(mut g) = self.map.lock() else { return PubCtx::fresh(user) };
        g.entry(user.to_string()).or_insert_with(|| PubCtx::fresh(user)).clone()
    }

    pub fn get(&self, user: &str) -> Option<Arc<PubCtx>> {
        self.map.lock().ok().and_then(|g| g.get(user).cloned())
    }

    pub fn remove(&self, user: &str) -> Option<Arc<PubCtx>> {
        self.map.lock().ok().and_then(|mut g| g.remove(user))
    }
}

pub fn index_add(tr: &Arc<Transport>, t: &Arc<PubTrack>) {
    let rtx = t.rtx_ssrc.lock().ok().and_then(|g| g.as_ref().copied());
    tr.ingress.rcu(|m| {
        let mut n = (**m).clone();
        n.insert(t.ssrc, t.clone());
        if let Some(r) = rtx {
            n.insert(r, t.clone());
        }
        n
    });
}

pub fn index_remove(tr: &Arc<Transport>, stream: &Arc<PubStream>) {
    tr.ingress.rcu(|m| {
        let mut n = (**m).clone();
        n.retain(|_, t| !Arc::ptr_eq(&t.stream, stream));
        n
    });
}

fn spatial(r: &str) -> Option<u8> {
    match r {
        "l" => Some(0),
        "h" => Some(1),
        _ => None,
    }
}

pub fn learn(tr: &Arc<Transport>, rtp: &Rtp<'_>) -> Option<Arc<PubTrack>> {
    let ids = tr.ext_ids.load();
    let text = |id: u8| ext_get(rtp.ext, id).map(|v| String::from_utf8_lossy(v).into_owned());
    let rid = text(ids.rid);
    let rrid = text(ids.repair_rid);
    let mid = text(ids.mid);
    let ctx = crate::ops::pubs().get(&tr.user)?;
    let stream = ctx.streams().into_iter().find(|s| s.simulcast && mid.as_deref().map(|m| m == s.mid).unwrap_or(true))?;
    if let Some(r) = rrid {
        let sp = spatial(&r)?;
        let base = stream.physical().into_iter().find(|t| t.rid == Some(sp))?;
        if let Ok(mut g) = base.rtx_ssrc.lock() {
            g.replace(rtp.ssrc);
        }
        index_add(tr, &base);
        return Some(base);
    }
    let r = rid?;
    let Some(sp) = spatial(&r) else {
        crate::ops::drop_count("rid_unknown");
        return None;
    };
    if let Some(t) = stream.physical().into_iter().find(|t| t.rid == Some(sp)) {
        return Some(t);
    }
    let t = PubTrack::new(stream.clone(), rtp.ssrc, None, Some(sp), tr);
    if let Ok(mut g) = stream.tracks.lock() {
        g.push(t.clone());
    }
    index_add(tr, &t);
    crate::ops::simulcast_first_rtp(&stream);
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_gap_and_giveup() {
        let mut n = NackUp::default();
        n.on_packet(10, 0);
        n.on_packet(13, 0);
        let (s, g) = n.due(0);
        assert_eq!(s, vec![11, 12]);
        assert!(!g);
        assert!(n.due(100).0.is_empty());
        assert_eq!(n.due(200).0, vec![11, 12]);
        n.on_packet(11, 250);
        let (s, g) = n.due(1_001);
        assert!(s.is_empty());
        assert!(g);
    }

    #[test]
    fn nack_packets_count_once_per_batch() {
        let mut n = NackUp::default();
        n.on_packet(10, 0);
        n.on_packet(14, 0);
        n.due(0);
        n.due(50);
        n.due(200);
        assert_eq!((n.sent, n.pkts), (6, 2));
    }

    #[test]
    fn rtx_restores_the_original_only_when_it_carries_one() {
        let head = [0x80u8, 97, 0, 7, 0, 0, 0, 9, 0, 0, 0, 2];
        let mut media = head.to_vec();
        media.extend_from_slice(&[0x12, 0x34, 0xaa, 0xbb]);
        match rtx_original(&media, 12, 96, 1) {
            Rtx::Original(osn, b) => {
                assert_eq!(osn, 0x1234, "정:RULE-RTCP-NACK-UP the first two payload bytes are the original seq");
                assert_eq!(b, vec![0x80, 96, 0x12, 0x34, 0, 0, 0, 9, 0, 0, 0, 1, 0xaa, 0xbb]);
            }
            _ => panic!("정:RULE-RTCP-NACK-UP an RTX packet with a payload restores its original"),
        }
        let mut pad = head.to_vec();
        pad[0] = 0xa0;
        pad.extend_from_slice(&[0u8; 31]);
        pad.push(32);
        assert!(matches!(rtx_original(&pad, 12, 96, 1), Rtx::Padding), "S-sym-RULE-RTCP-NACK-UP-3 padding only — its padding bytes are not an original seq");
        let mut both = head.to_vec();
        both[0] = 0xa0;
        both.extend_from_slice(&[0x00, 0x05, 0xcc, 0, 0, 3]);
        assert!(matches!(rtx_original(&both, 12, 96, 1), Rtx::Original(5, _)), "정:RULE-RTCP-NACK-UP payload plus padding still carries an original");
        assert!(matches!(rtx_original(&head, 12, 96, 1), Rtx::Padding), "S-sym-RULE-RTCP-NACK-UP-3 an empty RTX packet carries nothing");
        let mut bad = head.to_vec();
        bad[0] = 0xa0;
        bad.extend_from_slice(&[1, 2, 0]);
        assert!(matches!(rtx_original(&bad, 12, 96, 1), Rtx::Malformed), "정:RULE-RTCP-NACK-UP a zero padding count is malformed");
        let mut over = head.to_vec();
        over[0] = 0xa0;
        over.extend_from_slice(&[1, 2, 200]);
        assert!(matches!(rtx_original(&over, 12, 96, 1), Rtx::Malformed), "정:RULE-RTCP-NACK-UP a padding count past the packet is malformed");
        assert!(matches!(rtx_original(&head[..8], 12, 96, 1), Rtx::Malformed));
    }

    #[test]
    fn nack_wrap() {
        let mut n = NackUp::default();
        n.on_packet(65_534, 0);
        n.on_packet(1, 0);
        assert_eq!(n.due(0).0, vec![65_535, 0]);
    }
}
