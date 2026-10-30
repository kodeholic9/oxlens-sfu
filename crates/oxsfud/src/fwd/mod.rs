// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:IMPL-FWD-ROOM · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-CONC-HOT-PATH · 정:RULE-OPS-TRANSIT · 정:RULE-FWD-SCOPE · 정:CAND-FWD-REWRITER-LOCK · model: claude-opus-5-5
//! 전달 — `on_rtp`(복호한 평문 RTP 하나: 발언권 관문 · 줄 고르기 · 목적지 워커 묶음) · 다운링크 몫(재기록 · PT · 확장 · TWCC · SRTP · 송신 · 체류).

pub mod core;
pub mod rt;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::{ArcSwap, ArcSwapOption};
use bytes::Bytes;
use oxsig::msg::Kind;

use crate::link::Transport;
use crate::sub::ext::{ExtInput, ExtTable};
use core::Rewriter;
use rt::{Item, Worker, media, mono_us};

pub const LAYER_PAUSE: u8 = 255;
pub const PRIME_FRAMES: u32 = 250;
pub const SILENCE_FRAMES: u32 = 3;
pub const OPUS_SILENCE: [u8; 3] = [0xf8, 0xff, 0xfe];
pub const BLOCKED_NOTIFY_US: u64 = 500_000;

pub struct DownRow {
    pub conn: Arc<Transport>,
    pub user: String,
    pub mid: String,
    pub kind: Kind,
    pub slot: bool,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub clock: u32,
    pub opus: bool,
    pub since: AtomicU64,
    pub acked: Arc<AtomicU64>,
    pub ext: ArcSwap<ExtTable>,
    pub rw: Mutex<Rewriter>,
    pub rtx_seq: AtomicU16,
    pub src: ArcSwapOption<(String, String)>,
    pub room: ArcSwapOption<String>,
    pub gate: ArcSwapOption<AtomicBool>,
    pub sent: AtomicU64,
    pub bytes: AtomicU64,
    pub last_at: AtomicU64,
    pub loaded_at: AtomicU64,
    pub layer: AtomicU8,
    pub target: AtomicU8,
    pub cap: AtomicU8,
    pub auto_cap: AtomicU8,
    pub pending_since: AtomicU64,
    pub paused: AtomicBool,
    pub priority: AtomicU8,
    pub rtx_gate_until_new: AtomicBool,
    pub src_track: ArcSwapOption<crate::r#pub::PubTrack>,
    pub rr_loss: AtomicU8,
    pub rr_at: AtomicU64,
    pub nacks: AtomicU64,
    pub drops: AtomicU64,
}

impl DownRow {
    pub fn create(conn: Arc<Transport>, mid: String, kind: Kind, slot: bool, ssrc: u32, rtx_ssrc: Option<u32>, acked: Arc<AtomicU64>) -> DownRow {
        let clock = if kind == Kind::Audio { 48_000 } else { 90_000 };
        DownRow {
            user: conn.user.clone(),
            conn,
            mid,
            kind,
            slot,
            ssrc,
            rtx_ssrc,
            pt: AtomicU8::new(0),
            rtx_pt: AtomicU8::new(0),
            clock,
            opus: kind == Kind::Audio,
            since: AtomicU64::new(0),
            acked,
            ext: ArcSwap::from_pointee(ExtTable::default()),
            rw: Mutex::new(Rewriter::new(clock)),
            rtx_seq: AtomicU16::new(common::random_bytes(2).iter().fold(0u16, |a, b| (a << 8) | *b as u16)),
            src: ArcSwapOption::empty(),
            room: ArcSwapOption::empty(),
            gate: ArcSwapOption::empty(),
            sent: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            last_at: AtomicU64::new(0),
            loaded_at: AtomicU64::new(0),
            layer: AtomicU8::new(LAYER_PAUSE),
            target: AtomicU8::new(1),
            cap: AtomicU8::new(1),
            auto_cap: AtomicU8::new(1),
            pending_since: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            priority: AtomicU8::new(0),
            rtx_gate_until_new: AtomicBool::new(false),
            src_track: ArcSwapOption::empty(),
            rr_loss: AtomicU8::new(0),
            rr_at: AtomicU64::new(0),
            nacks: AtomicU64::new(0),
            drops: AtomicU64::new(0),
        }
    }

    pub fn open(&self) -> bool {
        self.since.load(Ordering::Acquire) <= self.acked.load(Ordering::Acquire)
    }
}

pub struct SlotFan {
    pub audio: bool,
    pub rows: ArcSwap<Vec<Arc<DownRow>>>,
    pub speaker: ArcSwapOption<String>,
    pub bind: Mutex<crate::flr::bind::Slot>,
    pub episode: AtomicU64,
    pub priming: AtomicU32,
    pub silence: AtomicU32,
    pub prime_seq: AtomicU16,
    pub prime_ts: AtomicU32,
    pub last_prime_us: AtomicU64,
    pub silence_skip: ArcSwapOption<String>,
    pub first_rtp_us: AtomicU64,
    pub last_rtp_us: AtomicU64,
    pub last_notify_us: AtomicU64,
}

impl SlotFan {
    pub fn new(audio: bool) -> SlotFan {
        SlotFan {
            audio,
            rows: ArcSwap::from_pointee(Vec::new()),
            speaker: ArcSwapOption::empty(),
            bind: Mutex::new(crate::flr::bind::Slot::default()),
            episode: AtomicU64::new(1 << 40),
            priming: AtomicU32::new(0),
            silence: AtomicU32::new(0),
            prime_seq: AtomicU16::new(0),
            prime_ts: AtomicU32::new(0),
            last_prime_us: AtomicU64::new(0),
            silence_skip: ArcSwapOption::empty(),
            first_rtp_us: AtomicU64::new(0),
            last_rtp_us: AtomicU64::new(0),
            last_notify_us: AtomicU64::new(0),
        }
    }

    pub fn speaker_is(&self, user: &str) -> bool {
        self.speaker.load().as_deref().map(|s| s.as_str() == user).unwrap_or(false)
    }
}

#[derive(Clone)]
pub struct Pkt {
    pub data: Bytes,
    pub hlen: usize,
    pub seq: u16,
    pub ts: u32,
    pub marker: bool,
    pub audio_level: Option<u8>,
    pub src_key: u64,
    pub covers: u32,
    pub at_us: u64,
    pub keyframe: bool,
    pub transit: bool,
    pub track: Option<Arc<crate::r#pub::PubTrack>>,
}

pub struct Egress {
    pub pkt: Pkt,
    pub targets: Vec<Arc<DownRow>>,
}

pub enum Ctl {
    Rtcp { tr: Arc<Transport>, data: Vec<u8> },
    Rtx { row: Arc<DownRow>, seqs: Vec<u16> },
    Frames { rows: Vec<Arc<DownRow>>, payload: Vec<u8>, src_key: u64, seq: u16, ts: u32 },
}

impl Ctl {
    pub fn packets(&self) -> usize {
        match self {
            Ctl::Rtcp { .. } => 1,
            Ctl::Rtx { seqs, .. } => seqs.len().max(1),
            Ctl::Frames { rows, .. } => rows.len().max(1),
        }
    }
}

pub struct Rtp<'a> {
    pub raw: &'a [u8],
    pub pt: u8,
    pub marker: bool,
    pub seq: u16,
    pub ts: u32,
    pub ssrc: u32,
    pub hlen: usize,
    pub ext: Option<(u16, &'a [u8])>,
}

pub fn parse_rtp(b: &[u8]) -> Option<Rtp<'_>> {
    if b.len() < 12 || b[0] >> 6 != 2 {
        return None;
    }
    let cc = (b[0] & 0x0f) as usize;
    let x = b[0] & 0x10 != 0;
    let mut h = 12 + cc * 4;
    if b.len() < h {
        return None;
    }
    let mut ext = None;
    if x {
        if b.len() < h + 4 {
            return None;
        }
        let profile = u16::from_be_bytes([b[h], b[h + 1]]);
        let words = u16::from_be_bytes([b[h + 2], b[h + 3]]) as usize;
        let end = h + 4 + words * 4;
        if b.len() < end {
            return None;
        }
        ext = Some((profile, &b[h + 4..end]));
        h = end;
    }
    Some(Rtp {
        raw: b,
        pt: b[1] & 0x7f,
        marker: b[1] & 0x80 != 0,
        seq: u16::from_be_bytes([b[2], b[3]]),
        ts: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        ssrc: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        hlen: h,
        ext,
    })
}

pub fn ext_get(ext: Option<(u16, &[u8])>, id: u8) -> Option<&[u8]> {
    let (profile, d) = ext?;
    if id == 0 {
        return None;
    }
    let mut i = 0;
    if profile == 0xbede {
        while i < d.len() {
            let b = d[i];
            if b == 0 {
                i += 1;
                continue;
            }
            let eid = b >> 4;
            let len = (b & 0x0f) as usize + 1;
            if eid == 15 {
                return None;
            }
            if i + 1 + len > d.len() {
                return None;
            }
            if eid == id {
                return Some(&d[i + 1..i + 1 + len]);
            }
            i += 1 + len;
        }
    } else if profile & 0xfff0 == 0x1000 {
        while i + 1 < d.len() {
            let eid = d[i];
            if eid == 0 {
                i += 1;
                continue;
            }
            let len = d[i + 1] as usize;
            if i + 2 + len > d.len() {
                return None;
            }
            if eid == id {
                return Some(&d[i + 2..i + 2 + len]);
            }
            i += 2 + len;
        }
    }
    None
}

pub fn worker_loop(w: &Arc<Worker>) {
    let _ = w.thread.set(std::thread::current());
    let mut next_tick = mono_us();
    let mut idle = 0u32;
    loop {
        let mut did = 0usize;
        for ring in &w.inbox {
            while let Some(item) = ring.pop() {
                let n = match &item {
                    Item::In { .. } => 1,
                    Item::Out(e) => e.targets.len().max(1),
                    Item::Ctl(c) => c.packets(),
                };
                process(w, item);
                w.queued.fetch_sub(n, Ordering::AcqRel);
                did += 1;
            }
        }
        let now = mono_us();
        if now >= next_tick {
            crate::rtcp::tick(w, now);
            next_tick = now + 10_000;
        }
        if did > 0 {
            idle = 0;
            continue;
        }
        idle += 1;
        if idle < 64 {
            std::hint::spin_loop();
            continue;
        }
        w.sleeping.store(true, Ordering::Release);
        let empty = w.inbox.iter().all(|r| r.is_empty());
        if empty {
            std::thread::park_timeout(std::time::Duration::from_millis(5));
        }
        w.sleeping.store(false, Ordering::Release);
        idle = 0;
    }
}

fn process(w: &Arc<Worker>, item: Item) {
    match item {
        Item::In { tr, data, at_us } => ingress(w, &tr, data, at_us),
        Item::Out(e) => {
            for t in &e.targets {
                send_down(w, t, &e.pkt);
            }
        }
        Item::Ctl(c) => match c {
            Ctl::Rtcp { tr, data } => crate::rtcp::send_rtcp(w, &tr, &data),
            Ctl::Rtx { row, seqs } => crate::rtcp::send_rtx(w, &row, &seqs),
            Ctl::Frames { rows, payload, src_key, seq, ts } => {
                let mut b = Vec::with_capacity(12 + payload.len());
                b.extend_from_slice(&[0x80, 111]);
                b.extend_from_slice(&seq.to_be_bytes());
                b.extend_from_slice(&ts.to_be_bytes());
                b.extend_from_slice(&0u32.to_be_bytes());
                b.extend_from_slice(&payload);
                let pkt = Pkt {
                    data: Bytes::from(b),
                    hlen: 12,
                    seq,
                    ts,
                    marker: false,
                    audio_level: None,
                    src_key,
                    covers: core::opus_toc_samples(&payload),
                    at_us: mono_us(),
                    keyframe: false,
                    transit: false,
                    track: None,
                };
                for r in &rows {
                    send_down(w, r, &pkt);
                }
            }
        },
    }
}

fn ingress(w: &Arc<Worker>, tr: &Arc<Transport>, data: Vec<u8>, at_us: u64) {
    if !tr.ready.load(Ordering::Acquire) {
        crate::ops::drop_count("srtp_not_ready");
        return;
    }
    let is_rtcp = data.len() > 1 && (192..=223).contains(&data[1]);
    let plain = {
        let Ok(mut g) = tr.srtp_rx.lock() else { return };
        let Some(ctx) = g.as_mut() else {
            crate::ops::drop_count("srtp_not_ready");
            return;
        };
        if is_rtcp { ctx.decrypt_rtcp(&data) } else { ctx.decrypt_rtp(&data) }
    };
    let plain = match plain {
        Ok(p) => p,
        Err(_) => {
            crate::ops::drop_count(if is_rtcp { "srtcp_fail" } else { "srtp_fail" });
            return;
        }
    };
    if is_rtcp {
        crate::rtcp::on_rtcp(w, tr, &plain);
    } else {
        on_rtp(w, tr, plain, at_us);
    }
}

pub fn on_rtp(w: &Arc<Worker>, tr: &Arc<Transport>, plain: Bytes, at_us: u64) {
    let Some(rtp) = parse_rtp(&plain) else {
        crate::ops::drop_count("rtp_malformed");
        return;
    };
    let ing = tr.ingress.load();
    let track = match ing.get(&rtp.ssrc) {
        Some(t) => t.clone(),
        None => match crate::r#pub::learn(tr, &rtp) {
            Some(t) => t,
            None => {
                crate::ops::drop_count("rtp_unknown_ssrc");
                return;
            }
        },
    };
    let ids = tr.ext_ids.load();
    if let Some(seq) = ext_get(rtp.ext, ids.twcc).filter(|v| v.len() == 2).map(|v| u16::from_be_bytes([v[0], v[1]])) {
        crate::rtcp::twcc_in(tr, seq, at_us as i64);
    }
    if track.is_rtx(rtp.ssrc) {
        let Some((_, fixed)) = track.unwrap_rtx(&plain, rtp.hlen) else {
            crate::ops::drop_count("rtp_malformed");
            return;
        };
        let fixed = Bytes::from(fixed);
        let Some(orig) = parse_rtp(&fixed) else {
            crate::ops::drop_count("rtp_malformed");
            return;
        };
        track.on_repair(&orig);
        forward(w, tr, &track, &orig, fixed.clone(), at_us);
        return;
    }
    track.on_ingress(&rtp, common::now_ms(), at_us, &ids);
    forward(w, tr, &track, &rtp, plain.clone(), at_us);
}

fn forward(w: &Arc<Worker>, tr: &Arc<Transport>, track: &Arc<crate::r#pub::PubTrack>, rtp: &Rtp<'_>, plain: Bytes, at_us: u64) {
    let now_ms = common::now_ms();
    let ids = tr.ext_ids.load();
    let stream = &track.stream;
    let audio_level = if stream.kind == Kind::Audio { ext_get(rtp.ext, ids.audio_level).and_then(|v| v.first().copied()) } else { None };
    let payload = &plain[rtp.hlen..];
    let keyframe = stream.kind == Kind::Video && crate::trk::is_keyframe(&stream.codec_name(), payload);
    let pkt = Pkt {
        data: plain.clone(),
        hlen: rtp.hlen,
        seq: rtp.seq,
        ts: rtp.ts,
        marker: rtp.marker,
        audio_level,
        src_key: track.src_key,
        covers: if stream.kind == Kind::Audio { core::opus_toc_samples(payload) } else { 1 },
        at_us,
        keyframe,
        transit: true,
        track: Some(track.clone()),
    };
    if keyframe {
        track.last_keyframe_ms.store(now_ms, Ordering::Release);
    }
    let peer_room = tr.pub_room.load_full();
    let Some(room) = peer_room else {
        crate::ops::drop_count("no_pub_room");
        return;
    };
    let mut targets: Vec<Arc<DownRow>> = Vec::new();
    if stream.is_half() {
        let Some(fan) = crate::ops::slot_of(&room, stream.kind) else {
            crate::ops::drop_count("prefan_gate");
            return;
        };
        if !fan.speaker_is(&tr.user) {
            crate::ops::drop_count("prefan_gate");
            if track.blocked_notify(at_us) {
                crate::ops::floor_blocked_rtp(&room, tr);
            }
            return;
        }
        fan.priming.store(0, Ordering::Release);
        let first = fan.first_rtp_us.compare_exchange(0, at_us, Ordering::AcqRel, Ordering::Acquire).is_ok();
        fan.last_rtp_us.store(at_us, Ordering::Release);
        let prev = fan.last_notify_us.load(Ordering::Relaxed);
        if first || at_us.saturating_sub(prev) >= BLOCKED_NOTIFY_US {
            fan.last_notify_us.store(at_us, Ordering::Relaxed);
            crate::ops::floor_rtp(&room, tr, first);
        }
        for r in fan.rows.load().iter() {
            if r.user != tr.user && r.open() {
                targets.push(r.clone());
            }
        }
    } else {
        if stream.room.load().as_str() != room.as_str() {
            crate::ops::drop_count("no_pub_room");
            return;
        }
        for r in stream.rows.load().iter() {
            if !r.open() {
                crate::ops::drop_count("row_gate");
                continue;
            }
            if stream.kind == Kind::Video {
                let gated = r.gate.load().as_ref().map(|g| !g.load(Ordering::Acquire)).unwrap_or(true);
                if gated {
                    crate::ops::drop_count("ready_gate");
                    continue;
                }
                if stream.simulcast && !crate::sim::pass(r, track, keyframe, now_ms) {
                    continue;
                }
            }
            targets.push(r.clone());
        }
    }
    if targets.is_empty() {
        return;
    }
    let m = match media() {
        Some(m) => m,
        None => return,
    };
    let mut by_worker: Vec<Vec<Arc<DownRow>>> = vec![Vec::new(); m.workers.len()];
    for t in targets {
        let i = t.conn.worker.min(m.workers.len() - 1);
        by_worker[i].push(t);
    }
    for (i, rows) in by_worker.into_iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        if i == w.id {
            for r in &rows {
                send_down(w, r, &pkt);
            }
        } else if !m.workers[i].push(w.id + 1, Item::Out(Egress { pkt: pkt.clone(), targets: rows })) {
            crate::ops::drop_count("egress_queue_full");
        }
    }
}

pub fn send_down(w: &Worker, row: &DownRow, pkt: &Pkt) {
    let Some(m) = media() else { return };
    let conn = &row.conn;
    if conn.closed.load(Ordering::Acquire) {
        crate::ops::drop_count("transport_gone");
        return;
    }
    if !conn.ready.load(Ordering::Acquire) {
        return;
    }
    let now_ms = common::now_ms();
    let Some(path) = conn.send_ok(now_ms, |id| m.tcp_open(id)) else { return };
    let mapped = match row.rw.lock() {
        Ok(mut rw) => {
            if row.kind == Kind::Video && rw.src != Some(pkt.src_key) && !pkt.keyframe {
                drop(rw);
                crate::ops::drop_count("await_keyframe");
                if let Some(t) = &pkt.track {
                    crate::rtcp::request_pli(t, true);
                }
                return;
            }
            rw.map(pkt.src_key, pkt.seq, pkt.ts, now_ms, pkt.covers)
        }
        Err(_) => return,
    };
    if mapped.switched || row.src_track.load().is_none() {
        if row.kind == Kind::Video {
            row.rtx_gate_until_new.store(false, Ordering::Release);
        }
        if let Some(t) = &pkt.track {
            row.src_track.store(Some(t.clone()));
        }
    }
    let twcc = conn.twcc_seq.fetch_add(1, Ordering::AcqRel);
    let table = row.ext.load();
    let mut out = Vec::with_capacity(pkt.data.len() + 32);
    let b0 = pkt.data[0] & 0x20;
    out.push(0x80 | b0);
    out.push(core::pt_byte1(pkt.data[1], row.pt.load(Ordering::Relaxed)));
    out.extend_from_slice(&mapped.seq.to_be_bytes());
    out.extend_from_slice(&mapped.ts.to_be_bytes());
    out.extend_from_slice(&row.ssrc.to_be_bytes());
    let inp = ExtInput { mid: row.mid.as_bytes(), audio_level: pkt.audio_level, t_us: mono_us(), twcc_seq: Some(twcc) };
    let n = crate::sub::ext::build(&table, &inp, &mut out);
    if n > 0 {
        out[0] |= 0x10;
    }
    out.extend_from_slice(&pkt.data[pkt.hlen..]);
    let enc = match conn.srtp_tx.lock() {
        Ok(mut g) => match g.as_mut() {
            Some(ctx) => ctx.encrypt_rtp(&out),
            None => return,
        },
        Err(_) => return,
    };
    let Ok(enc) = enc else { return };
    if !m.send_on(&w.sock, path, &enc) {
        crate::ops::drop_count("send_fail");
        return;
    }
    crate::rtcp::sent_twcc(conn, twcc, enc.len(), mono_us(), false);
    row.sent.fetch_add(1, Ordering::Relaxed);
    row.bytes.fetch_add(enc.len() as u64, Ordering::Relaxed);
    row.last_at.store(now_ms, Ordering::Relaxed);
    conn.tx.packets.fetch_add(1, Ordering::Relaxed);
    conn.tx.bytes.fetch_add(enc.len() as u64, Ordering::Relaxed);
    if pkt.transit {
        w.transit.record(mono_us().saturating_sub(pkt.at_us));
    }
}
