// author: kodeholic (powered by Claude)
// spec: v1.2 @250e836 · 정:RULE-PUB-TWO-LAYERS 정:RULE-FWD-STEPS 정:IMPL-FWD-ROOM 정:RULE-FWD-BRANCH-AUTHORITY 정:IMPL-FWD-SOURCE-SWITCH 정:IMPL-SIM-SWITCH 정:IMPL-PUB-ACCEPT 정:IMPL-FLR-RTP-OBSERVE 정:IMPL-FLR-MEDIA-BIND 정:RULE-RTCP-NACK-UP 정:RULE-CONC-SIX · model: claude-opus-5-5
//! 전달 핫패스 — 패킷 하나가 들어와 구독자마다 나가는 길. 매 패킷 읽는 자료는 RCU · 원자값이다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::{ArcSwap, ArcSwapOption};
use oxsig::wire::{Kind, Source};

use crate::peer::Conn;
use crate::room::Room;
use crate::rtp::{self, Rewrite, Rtp};
use crate::sfu::{Sfu, lock, mono_ms};
use crate::table::Row;

pub const LAYER_L: u8 = 0;
pub const LAYER_H: u8 = 1;
const CACHE: usize = 1024;

#[derive(Debug, Clone, Default)]
pub struct ExtIn {
    pub twcc: Option<u8>,
    pub rid: Option<u8>,
    pub rrid: Option<u8>,
    pub audio_level: Option<u8>,
}

pub struct PubStream {
    pub track_id: String,
    pub user_id: String,
    pub kind: Kind,
    pub source: Option<Source>,
    pub codec: String,
    pub fmtp: Option<String>,
    pub pt: u8,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub simulcast: bool,
    pub half: AtomicBool,
    pub muted: AtomicBool,
    pub room_id: Mutex<String>,
    pub listed: AtomicBool,
    pub ext: ExtIn,
    pub tracks: ArcSwap<Vec<Arc<PubTrack>>>,
    pub subs: ArcSwap<Vec<Weak<Row>>>,
    pub conn: Weak<Conn>,
    pub first_rtp_at: AtomicU64,
    pub pt_checked: AtomicBool,
    pub level_at: AtomicU64,
    pub notify_at: AtomicU64,
    pub gone: AtomicBool,
}

impl PubStream {
    pub fn is_half(&self) -> bool {
        self.half.load(Ordering::Relaxed)
    }

    pub fn is_video(&self) -> bool {
        self.kind == Kind::Video
    }

    pub fn room(&self) -> String {
        lock(&self.room_id).clone()
    }

    pub fn track_for_layer(&self, layer: u8) -> Option<Arc<PubTrack>> {
        let t = self.tracks.load();
        t.iter().find(|x| x.layer == layer).cloned().or_else(|| t.iter().max_by_key(|x| x.layer).cloned())
    }

    pub fn add_sub(&self, row: &Arc<Row>) {
        let mut v: Vec<Weak<Row>> = self.subs.load().iter().filter(|w| w.strong_count() > 0).cloned().collect();
        if !v.iter().any(|w| w.upgrade().is_some_and(|r| Arc::ptr_eq(&r, row))) {
            v.push(Arc::downgrade(row));
        }
        self.subs.store(Arc::new(v));
    }

    pub fn del_sub(&self, row: &Arc<Row>) {
        let v: Vec<Weak<Row>> =
            self.subs.load().iter().filter(|w| w.upgrade().is_some_and(|r| !Arc::ptr_eq(&r, row))).cloned().collect();
        self.subs.store(Arc::new(v));
    }
}

#[derive(Default)]
pub struct RecvStats {
    pub started: bool,
    pub base: u32,
    pub max: u32,
    pub received: u64,
    pub expected_prior: u64,
    pub received_prior: u64,
    pub jitter: f64,
    pub transit: i64,
    pub has_transit: bool,
    pub bytes: u64,
    pub bytes_prior: u64,
    pub rate_at: u64,
    pub rate_bps: u64,
}

impl RecvStats {
    fn update(&mut self, seq: u16, ts: u32, arrival_ms: u64, clock: u32, len: usize) {
        if !self.started {
            self.started = true;
            self.base = seq as u32;
            self.max = seq as u32;
        } else {
            let cur = self.max & 0xffff;
            let cycles = self.max & !0xffff;
            let diff = seq.wrapping_sub(cur as u16) as i16;
            if diff > 0 {
                let ext = if (seq as u32) < cur { cycles + 0x1_0000 + seq as u32 } else { cycles + seq as u32 };
                self.max = ext;
            }
        }
        self.received += 1;
        self.bytes += len as u64;
        let arrival = (arrival_ms as i64) * (clock as i64) / 1000;
        let transit = arrival - ts as i64;
        if self.has_transit {
            let d = (transit - self.transit).abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = transit;
        self.has_transit = true;
    }

    pub fn report(&mut self) -> (u8, u32, u32, u32) {
        let expected = (self.max - self.base + 1) as u64;
        let lost_total = expected.saturating_sub(self.received);
        let e_int = expected.saturating_sub(self.expected_prior);
        let r_int = self.received.saturating_sub(self.received_prior);
        self.expected_prior = expected;
        self.received_prior = self.received;
        let lost_int = e_int.saturating_sub(r_int);
        let frac = if e_int == 0 { 0 } else { ((lost_int << 8) / e_int).min(255) as u8 };
        (frac, lost_total.min(0x7f_ffff) as u32, self.max, self.jitter as u32)
    }

    pub fn tick_rate(&mut self, now: u64) -> u64 {
        let dt = now.saturating_sub(self.rate_at);
        if dt >= 500 {
            let db = self.bytes.saturating_sub(self.bytes_prior);
            self.rate_bps = db * 8 * 1000 / dt.max(1);
            self.bytes_prior = self.bytes;
            self.rate_at = now;
        }
        self.rate_bps
    }
}

pub struct Cache {
    slots: Vec<Option<(u16, Vec<u8>)>>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache { slots: vec![None; CACHE] }
    }
}

impl Cache {
    pub fn put(&mut self, seq: u16, pkt: &[u8]) {
        let i = seq as usize % CACHE;
        self.slots[i] = Some((seq, pkt.to_vec()));
    }

    pub fn get(&self, seq: u16) -> Option<&[u8]> {
        match &self.slots[seq as usize % CACHE] {
            Some((s, p)) if *s == seq => Some(p.as_slice()),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct NackTrack {
    pub started: bool,
    pub last: u16,
    pub missing: BTreeMap<u16, (u64, u64)>,
}

impl NackTrack {
    pub fn on_seq(&mut self, seq: u16, now: u64) -> bool {
        if !self.started {
            self.started = true;
            self.last = seq;
            return false;
        }
        let diff = seq.wrapping_sub(self.last) as i16;
        if diff <= 0 {
            self.missing.remove(&seq);
            return false;
        }
        if diff >= 128 {
            self.missing.clear();
            self.last = seq;
            return true;
        }
        let mut s = self.last.wrapping_add(1);
        while s != seq {
            if self.missing.len() >= 128 {
                break;
            }
            self.missing.insert(s, (now, 0));
            s = s.wrapping_add(1);
        }
        self.last = seq;
        false
    }

    pub fn due(&mut self, now: u64) -> Vec<u16> {
        self.missing.retain(|_, (first, _)| now.saturating_sub(*first) < 1_000);
        let mut out = Vec::new();
        for (s, v) in self.missing.iter_mut() {
            if v.1 == 0 || now.saturating_sub(v.1) >= 200 {
                out.push(*s);
                v.1 = now;
            }
        }
        out
    }
}

pub struct PubTrack {
    pub ssrc: u32,
    pub rtx_ssrc: AtomicU32,
    pub rid: Option<String>,
    pub layer: u8,
    pub stream: Weak<PubStream>,
    pub recv: Mutex<RecvStats>,
    pub cache: Mutex<Cache>,
    pub nack: Mutex<NackTrack>,
    pub sr: Mutex<Option<(u64, u32)>>,
    pub sr_at: AtomicU64,
    pub pli_at: AtomicU64,
    pub last_rx: AtomicU64,
    pub key_at: AtomicU64,
}

impl PubTrack {
    pub fn new(ssrc: u32, rid: Option<String>, layer: u8, stream: &Arc<PubStream>) -> Arc<PubTrack> {
        Arc::new(PubTrack {
            ssrc,
            rtx_ssrc: AtomicU32::new(0),
            rid,
            layer,
            stream: Arc::downgrade(stream),
            recv: Mutex::new(RecvStats::default()),
            cache: Mutex::new(Cache::default()),
            nack: Mutex::new(NackTrack::default()),
            sr: Mutex::new(None),
            sr_at: AtomicU64::new(0),
            pli_at: AtomicU64::new(0),
            last_rx: AtomicU64::new(0),
            key_at: AtomicU64::new(0),
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rewriter {
    pub src: u32,
    pub started: bool,
    pub seq_off: u16,
    pub ts_off: u32,
    pub last_seq: u16,
    pub last_ts: u32,
    pub last_at: u64,
    pub boundary: u16,
    pub clock: u32,
    pub sent: u64,
    pub octets: u64,
}

impl Rewriter {
    pub fn map(&mut self, src: u32, seq: u16, ts: u32, now: u64) -> (u16, u32) {
        if !self.started || src != self.src {
            if self.started {
                let elapsed = now.saturating_sub(self.last_at).max(1);
                let ticks = ((elapsed as u128 * self.clock.max(1) as u128) / 1000).max(1) as u32;
                self.seq_off = self.last_seq.wrapping_add(1).wrapping_sub(seq);
                self.ts_off = self.last_ts.wrapping_add(ticks).wrapping_sub(ts);
            } else {
                self.seq_off = 0;
                self.ts_off = 0;
                self.last_seq = seq.wrapping_sub(1);
                self.last_ts = ts;
            }
            self.src = src;
            self.started = true;
            self.boundary = seq.wrapping_add(self.seq_off);
        }
        let out_seq = seq.wrapping_add(self.seq_off);
        let out_ts = ts.wrapping_add(self.ts_off);
        if (out_seq.wrapping_sub(self.last_seq) as i16) > 0 {
            self.last_seq = out_seq;
            self.last_ts = out_ts;
            self.last_at = now;
        }
        (out_seq, out_ts)
    }

    pub fn reverse(&self, out_seq: u16) -> Option<u16> {
        if !self.started || (out_seq.wrapping_sub(self.boundary) as i16) < 0 {
            return None;
        }
        Some(out_seq.wrapping_sub(self.seq_off))
    }
}

pub fn learn_sim(sfu: &Sfu, conn: &Arc<Conn>, pkt: &Rtp<'_>) -> Option<Arc<PubTrack>> {
    let sims = conn.sims.load();
    for s in sims.iter() {
        if s.gone.load(Ordering::Relaxed) {
            continue;
        }
        if let Some(id) = s.ext.rid
            && let Some(v) = pkt.ext_value(id)
        {
            let rid = String::from_utf8_lossy(v).to_string();
            if let Some(t) = s.tracks.load().iter().find(|t| t.rid.as_deref() == Some(rid.as_str())) {
                if t.ssrc == pkt.ssrc {
                    return Some(t.clone());
                }
                continue;
            }
            let layer = match rid.as_str() {
                "l" | "q" | "low" | "0" => LAYER_L,
                _ => LAYER_H,
            };
            let t = PubTrack::new(pkt.ssrc, Some(rid.clone()), layer, s);
            let mut v: Vec<Arc<PubTrack>> = s.tracks.load().iter().cloned().collect();
            v.push(t.clone());
            s.tracks.store(Arc::new(v));
            let _g = lock(&sfu.index_lock);
            let mut m = (**conn.pubs.load()).clone();
            m.insert(pkt.ssrc, t.clone());
            conn.pubs.store(Arc::new(m));
            tracing::info!(kind = "ev", spec = "IMPL-PUB-ACCEPT", track = s.track_id.as_str(), ssrc = pkt.ssrc, rid = rid.as_str(), "rid attach");
            return Some(t);
        }
        if let Some(id) = s.ext.rrid
            && let Some(v) = pkt.ext_value(id)
        {
            let rid = String::from_utf8_lossy(v).to_string();
            if let Some(t) = s.tracks.load().iter().find(|t| t.rid.as_deref() == Some(rid.as_str())) {
                t.rtx_ssrc.store(pkt.ssrc, Ordering::Relaxed);
                let _g = lock(&sfu.index_lock);
                let mut m = (**conn.rtx_of.load()).clone();
                m.insert(pkt.ssrc, t.clone());
                conn.rtx_of.store(Arc::new(m));
                return None;
            }
        }
    }
    None
}

pub fn on_rtp(sfu: &Arc<Sfu>, conn: &Arc<Conn>, plain: &[u8]) {
    let Some(pkt) = Rtp::parse(plain) else {
        sfu.counters.drop("rtp_parse");
        return;
    };
    let now = mono_ms();
    let track = match conn.pubs.load().get(&pkt.ssrc) {
        Some(t) => t.clone(),
        None => {
            if let Some(t) = conn.rtx_of.load().get(&pkt.ssrc).cloned() {
                on_rtx(sfu, conn, &t, &pkt, now);
                return;
            }
            match learn_sim(sfu, conn, &pkt) {
                Some(t) => t,
                None => {
                    sfu.counters.drop("unknown_ssrc");
                    return;
                }
            }
        }
    };
    ingest(sfu, conn, &track, &pkt, plain, now, false);
}

fn on_rtx(sfu: &Arc<Sfu>, conn: &Arc<Conn>, track: &Arc<PubTrack>, pkt: &Rtp<'_>, now: u64) {
    let p = pkt.payload();
    if p.len() < 2 {
        sfu.counters.drop("rtx_short");
        return;
    }
    let osn = u16::from_be_bytes([p[0], p[1]]);
    let Some(stream) = track.stream.upgrade() else { return };
    let w = Rewrite { pt: stream.pt, seq: osn, ts: pkt.ts, ssrc: track.ssrc, marker: pkt.marker, exts: &[] };
    let orig = rtp::build_raw(&pkt.buf[12..pkt.csrc_end], pkt.buf[0] & 0x0f, &w, &p[2..]);
    let Some(op) = Rtp::parse(&orig) else { return };
    ingest(sfu, conn, track, &op, &orig, now, true);
}

#[allow(clippy::too_many_arguments)]
fn ingest(sfu: &Arc<Sfu>, conn: &Arc<Conn>, track: &Arc<PubTrack>, pkt: &Rtp<'_>, plain: &[u8], now: u64, repaired: bool) {
    let Some(stream) = track.stream.upgrade() else {
        sfu.counters.drop("orphan_track");
        return;
    };
    if stream.gone.load(Ordering::Relaxed) {
        sfu.counters.drop("stream_gone");
        return;
    }
    track.last_rx.store(now, Ordering::Relaxed);
    let video = stream.is_video();
    if !repaired {
        lock(&track.recv).update(pkt.seq, pkt.ts, now, rtp::clock(video), plain.len());
        if video {
            let jump = lock(&track.nack).on_seq(pkt.seq, now);
            if jump {
                request_keyframe(sfu, &stream, Some(track.layer), true);
            }
        }
        if let Some(id) = stream.ext.twcc
            && let Some(v) = pkt.ext_value(id)
            && v.len() >= 2
        {
            lock(&conn.twcc_in).record(u16::from_be_bytes([v[0], v[1]]), crate::sfu::now_ms() * 1000);
        }
    } else {
        lock(&track.nack).missing.remove(&pkt.seq);
    }
    lock(&track.cache).put(pkt.seq, plain);
    if !repaired && pkt.pt != stream.pt && !stream.pt_checked.swap(true, Ordering::Relaxed) {
        tracing::warn!(kind = "diag", track = stream.track_id.as_str(), declared = stream.pt, got = pkt.pt, "[PT:MISMATCH]");
    }
    if stream.first_rtp_at.compare_exchange(0, now, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        tracing::info!(kind = "ev", spec = "DEV-PUB-STALL-FROM-RTP", track = stream.track_id.as_str(), ssrc = pkt.ssrc, "first rtp");
        if stream.simulcast && !stream.listed.load(Ordering::Relaxed) {
            crate::publish::list_sim(sfu, &stream);
        }
    }
    if let Some(id) = stream.ext.audio_level
        && let Some(v) = pkt.ext_value(id)
        && !v.is_empty()
        && (v[0] & 0x7f) < 50
    {
        stream.level_at.store(now, Ordering::Relaxed);
    }
    let video_key = video && rtp::is_keyframe(&stream.codec, pkt.payload());
    if video_key {
        track.key_at.store(now, Ordering::Relaxed);
    }
    let Some(peer) = conn.peer() else { return };
    let Some(room) = peer.pub_room.load_full() else {
        sfu.counters.drop("no_pub_room");
        return;
    };
    if stream.is_half() {
        forward_half(sfu, &room, &stream, track, pkt, now, video_key);
    } else {
        if stream.room() != room.id {
            sfu.counters.drop("not_pub_room");
            return;
        }
        forward_full(sfu, &stream, track, pkt, now, video_key);
    }
}

#[allow(clippy::too_many_arguments)]
fn forward_half(sfu: &Arc<Sfu>, room: &Arc<Room>, stream: &Arc<PubStream>, track: &Arc<PubTrack>, pkt: &Rtp<'_>, now: u64, key: bool) {
    let holder = room.holder.load();
    if holder.as_deref().map(String::as_str) != Some(stream.user_id.as_str()) {
        sfu.counters.drop("floor_gate");
        return;
    }
    let slot = room.slot(stream.kind);
    let bound = slot.bound.load();
    if !bound.as_ref().is_some_and(|b| Arc::ptr_eq(b, stream)) {
        sfu.counters.drop("slot_unbound");
        return;
    }
    crate::floor::observe_rtp(sfu, room, stream, now);
    slot.priming_stop.store(true, Ordering::Relaxed);
    for w in slot.rows.load().iter() {
        let Some(row) = w.upgrade() else { continue };
        if row.user_id == stream.user_id {
            continue;
        }
        send_row(sfu, &row, track.ssrc, pkt, now, key, stream);
    }
}

fn forward_full(sfu: &Arc<Sfu>, stream: &Arc<PubStream>, track: &Arc<PubTrack>, pkt: &Rtp<'_>, now: u64, key: bool) {
    for w in stream.subs.load().iter() {
        let Some(row) = w.upgrade() else { continue };
        if stream.is_video() && !row.open.load(Ordering::Relaxed) {
            sfu.counters.drop("ready_gate");
            continue;
        }
        if stream.simulcast && !crate::layer::sim_pass(sfu, &row, stream, track, key, now) {
            continue;
        }
        send_row(sfu, &row, track.ssrc, pkt, now, key, stream);
    }
}

pub fn send_row(sfu: &Sfu, row: &Arc<Row>, src: u32, pkt: &Rtp<'_>, now: u64, key: bool, stream: &PubStream) {
    let Some(conn) = row.conn.upgrade() else { return };
    if row.since.load(Ordering::Relaxed) > conn.ack.load(Ordering::Relaxed) {
        sfu.counters.drop("row_unacked");
        return;
    }
    let (seq, ts) = {
        let mut rw = lock(&row.rw);
        if row.kind == Kind::Video && (!rw.started || rw.src != src) && !key {
            drop(rw);
            if row.want_key.swap(false, Ordering::Relaxed) || now.saturating_sub(row.key_req_at.load(Ordering::Relaxed)) > 1_000 {
                row.key_req_at.store(now, Ordering::Relaxed);
                request_keyframe_ssrc(sfu, stream, src, false);
            }
            sfu.counters.drop("wait_keyframe");
            return;
        }
        let r = rw.map(src, pkt.seq, pkt.ts, now);
        rw.sent += 1;
        rw.octets += pkt.payload().len() as u64;
        r
    };
    let exts = egress_exts(&conn, row, pkt, stream, now);
    let w = Rewrite { pt: row.pt.load(Ordering::Relaxed), seq, ts, ssrc: row.ssrc, marker: pkt.marker, exts: &exts };
    let out = rtp::build(pkt, &w);
    if conn.send_rtp(&out) {
        row.egress.fetch_add(1, Ordering::Relaxed);
        if let Some(seq) = twcc_of(&conn, &exts) {
            lock(&conn.bwe).sent(seq, out.len(), crate::sfu::mono_us());
        }
    } else {
        sfu.counters.drop("egress_send");
    }
}

fn twcc_of(conn: &Conn, exts: &[(u8, Vec<u8>)]) -> Option<u16> {
    let id = conn.sub_ext.load().twcc?;
    exts.iter().find(|(i, _)| *i == id).map(|(_, v)| u16::from_be_bytes([v[0], v[1]]))
}

pub fn egress_exts(conn: &Conn, row: &Row, pkt: &Rtp<'_>, stream: &PubStream, now: u64) -> Vec<(u8, Vec<u8>)> {
    let ids = conn.sub_ext.load();
    let mut exts = Vec::with_capacity(4);
    if let Some(id) = ids.mid
        && now < row.mid_until.load(Ordering::Relaxed)
    {
        exts.push((id, row.mid.to_string().into_bytes()));
    }
    if row.kind == Kind::Audio
        && let (Some(id), Some(src_id)) = (ids.audio_level, stream.ext.audio_level)
        && let Some(v) = pkt.ext_value(src_id)
        && !v.is_empty()
    {
        exts.push((id, vec![v[0]]));
    }
    if let Some(id) = ids.abs_send_time {
        exts.push((id, rtp::abs_send_time(crate::sfu::now_ms() * 1000).to_vec()));
    }
    if let Some(id) = ids.twcc {
        let s = conn.twcc_out.fetch_add(1, Ordering::Relaxed) as u16;
        exts.push((id, s.to_be_bytes().to_vec()));
    }
    exts
}

pub fn send_synthetic(sfu: &Sfu, row: &Arc<Row>, payload: &[u8], ts_step: u32, now: u64) {
    let Some(conn) = row.conn.upgrade() else { return };
    if row.since.load(Ordering::Relaxed) > conn.ack.load(Ordering::Relaxed) {
        return;
    }
    let (seq, ts) = {
        let mut rw = lock(&row.rw);
        let base_seq = if rw.started && rw.src == 0 { rw.last_seq.wrapping_sub(rw.seq_off).wrapping_add(1) } else { 1 };
        let base_ts = if rw.started && rw.src == 0 { rw.last_ts.wrapping_sub(rw.ts_off).wrapping_add(ts_step) } else { 0 };
        let r = rw.map(0, base_seq, base_ts, now);
        rw.sent += 1;
        rw.octets += payload.len() as u64;
        r
    };
    let ids = conn.sub_ext.load();
    let mut exts = Vec::new();
    if let Some(id) = ids.abs_send_time {
        exts.push((id, rtp::abs_send_time(crate::sfu::now_ms() * 1000).to_vec()));
    }
    if let Some(id) = ids.twcc {
        let s = conn.twcc_out.fetch_add(1, Ordering::Relaxed) as u16;
        exts.push((id, s.to_be_bytes().to_vec()));
    }
    let w = Rewrite { pt: row.pt.load(Ordering::Relaxed), seq, ts, ssrc: row.ssrc, marker: false, exts: &exts };
    let out = rtp::build_raw(&[], 0, &w, payload);
    if conn.send_rtp(&out) {
        row.egress.fetch_add(1, Ordering::Relaxed);
    } else {
        sfu.counters.drop("synthetic_send");
    }
}

pub fn request_keyframe(sfu: &Sfu, stream: &PubStream, layer: Option<u8>, infra: bool) {
    let t = match layer {
        Some(l) => stream.track_for_layer(l),
        None => stream.track_for_layer(LAYER_H),
    };
    match t {
        Some(t) => request_keyframe_track(sfu, stream, &t, infra),
        None => {
            if !stream.simulcast && stream.ssrc != 0 {
                send_pli(sfu, stream, stream.ssrc);
            }
        }
    }
}

pub fn request_keyframe_ssrc(sfu: &Sfu, stream: &PubStream, ssrc: u32, infra: bool) {
    let t = stream.tracks.load().iter().find(|t| t.ssrc == ssrc).cloned();
    match t {
        Some(t) => request_keyframe_track(sfu, stream, &t, infra),
        None => send_pli(sfu, stream, ssrc),
    }
}

pub fn request_keyframe_track(sfu: &Sfu, stream: &PubStream, t: &PubTrack, infra: bool) {
    let now = mono_ms();
    let gap = if t.rid.is_some() && t.layer == LAYER_L { 100 } else { 300 };
    if !infra && now.saturating_sub(t.pli_at.load(Ordering::Relaxed)) < gap {
        sfu.counters.drop("pli_throttle");
        return;
    }
    t.pli_at.store(now, Ordering::Relaxed);
    send_pli(sfu, stream, t.ssrc);
}

fn send_pli(sfu: &Sfu, stream: &PubStream, ssrc: u32) {
    let Some(conn) = stream.conn.upgrade() else { return };
    let pkt = crate::rtcp_io::pli(ssrc);
    if !conn.send_rtcp(&pkt) {
        sfu.counters.drop("pli_send");
    }
}

pub struct Slot {
    pub rows: ArcSwap<Vec<Weak<Row>>>,
    pub bound: ArcSwapOption<PubStream>,
    pub priming_left: AtomicU32,
    pub priming_stop: AtomicBool,
    pub pli_burst: Mutex<Vec<u64>>,
    pub silence_left: AtomicU8,
}

impl Slot {
    pub fn new() -> Slot {
        Slot {
            rows: ArcSwap::from_pointee(Vec::new()),
            bound: ArcSwapOption::empty(),
            priming_left: AtomicU32::new(0),
            priming_stop: AtomicBool::new(true),
            pli_burst: Mutex::new(Vec::new()),
            silence_left: AtomicU8::new(0),
        }
    }

    pub fn add_row(&self, row: &Arc<Row>) {
        let mut v: Vec<Weak<Row>> = self.rows.load().iter().filter(|w| w.strong_count() > 0).cloned().collect();
        if !v.iter().any(|w| w.upgrade().is_some_and(|r| Arc::ptr_eq(&r, row))) {
            v.push(Arc::downgrade(row));
        }
        self.rows.store(Arc::new(v));
    }

    pub fn del_row(&self, row: &Arc<Row>) {
        let v: Vec<Weak<Row>> =
            self.rows.load().iter().filter(|w| w.upgrade().is_some_and(|r| !Arc::ptr_eq(&r, row))).cloned().collect();
        self.rows.store(Arc::new(v));
    }
}

pub const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];

pub fn slot_tick(sfu: &Sfu, room: &Room, now: u64) {
    let audio = room.slot(Kind::Audio);
    let opus = room.codecs.audio.as_deref().unwrap_or("opus").eq_ignore_ascii_case("opus");
    if opus && !audio.priming_stop.load(Ordering::Relaxed) {
        let left = audio.priming_left.load(Ordering::Relaxed);
        if left == 0 {
            audio.priming_stop.store(true, Ordering::Relaxed);
        } else {
            audio.priming_left.store(left - 1, Ordering::Relaxed);
            let speaker = room.holder.load_full();
            for w in audio.rows.load().iter() {
                let Some(row) = w.upgrade() else { continue };
                if speaker.as_deref().map(String::as_str) == Some(row.user_id.as_str()) {
                    continue;
                }
                send_synthetic(sfu, &row, &OPUS_SILENCE, 960, now);
            }
        }
    }
    let silence = audio.silence_left.load(Ordering::Relaxed);
    if opus && silence > 0 {
        audio.silence_left.store(silence - 1, Ordering::Relaxed);
        let prev = room.prev_speaker.load_full();
        for w in audio.rows.load().iter() {
            let Some(row) = w.upgrade() else { continue };
            if prev.as_deref().map(String::as_str) == Some(row.user_id.as_str()) {
                continue;
            }
            send_synthetic(sfu, &row, &OPUS_SILENCE, 960, now);
        }
    }
    let video = room.slot(Kind::Video);
    let due: Vec<u64> = {
        let mut b = lock(&video.pli_burst);
        let (d, rest): (Vec<u64>, Vec<u64>) = b.iter().partition(|t| **t <= now);
        b.clear();
        b.extend(rest);
        d
    };
    if !due.is_empty()
        && let Some(s) = video.bound.load_full()
    {
        request_keyframe(sfu, &s, None, true);
    }
}
