// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:RULE-FWD-STEPS · 정:RULE-FWD-BRANCH-AUTHORITY · 정:IMPL-FWD-ROOM · 정:RULE-FWD-MEDIA-WORKERS · 정:IMPL-FWD-SOURCE-SWITCH · 정:RULE-CONC-HOT-PATH · 정:RULE-OPS-TRANSIT · 정:RULE-FWD-SCOPE · 정:CAND-FWD-REWRITER-LOCK · 정:IMPL-SUB-TABLE · model: claude-opus-5-5
//! 전달 — `on_rtp`(복호한 평문 RTP 하나: 발언권 관문 · 줄 고르기 · 목적지 워커 묶음) · 다운링크 몫(재기록 · PT · 확장 · TWCC · SRTP · 송신 · 체류).

pub mod core;
pub mod rt;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arc_swap::{ArcSwap, ArcSwapOption};
use bytes::Bytes;
use oxsig::msg::Kind;

use crate::link::Transport;
use crate::sub::ext::{ExtInput, ExtTable};
use core::Rewriter;
use rt::{Item, Worker, media, mono_us};

pub const LAYER_PAUSE: u8 = 255;
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
    pub pending_pli_at: AtomicU64,
    pub paused: AtomicBool,
    pub priority: AtomicU8,
    pub last_ep: AtomicU64,
    pub applied: AtomicU64,
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
            pending_pli_at: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            priority: AtomicU8::new(0),
            last_ep: AtomicU64::new(0),
            applied: AtomicU64::new(0),
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

    pub fn rev(&self) -> u64 {
        self.since.load(Ordering::Acquire)
    }
}

pub fn row_same(r: &DownRow, rev: u64) -> bool {
    r.applied.load(Ordering::Acquire) == rev
}

pub struct RowCmd {
    pub row: Arc<DownRow>,
    pub rev: u64,
    pub reset: bool,
}

pub fn apply_row_cmd(row: &DownRow, rev: u64, reset: bool) -> bool {
    if rev <= row.applied.load(Ordering::Acquire) {
        return false;
    }
    if reset && let Ok(mut rw) = row.rw.lock() {
        rw.src = None;
    }
    row.src_track.store(None);
    row.applied.store(rev, Ordering::Release);
    true
}

#[derive(Default)]
pub struct RowCmds(crossbeam_queue::SegQueue<RowCmd>);

impl RowCmds {
    pub fn push(&self, c: RowCmd) {
        self.0.push(c);
    }

    pub fn drain(&self) -> usize {
        let mut n = 0;
        while let Some(c) = self.0.pop() {
            apply_row_cmd(&c.row, c.rev, c.reset);
            n += 1;
        }
        n
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub fn post_row(row: Arc<DownRow>, rev: u64, reset: bool) {
    let Some(w) = media().and_then(|m| m.workers.get(row.conn.worker)) else {
        apply_row_cmd(&row, rev, reset);
        return;
    };
    w.rows.push(RowCmd { row, rev, reset });
    std::sync::atomic::fence(Ordering::SeqCst);
    if w.sleeping.load(Ordering::SeqCst)
        && let Some(t) = w.thread.get()
    {
        t.unpark();
    }
}

pub struct Episode {
    pub id: u64,
    pub speaker: Option<Arc<String>>,
    pub grant: u64,
    pub first_rtp_us: AtomicU64,
    pub last_rtp_us: AtomicU64,
    pub last_notify_us: AtomicU64,
}

impl Episode {
    fn new(id: u64, speaker: Option<Arc<String>>, grant: u64) -> Episode {
        Episode { id, speaker, grant, first_rtp_us: AtomicU64::new(0), last_rtp_us: AtomicU64::new(0), last_notify_us: AtomicU64::new(0) }
    }

    pub fn speaker_is(&self, user: &str) -> bool {
        self.speaker.as_deref().is_some_and(|s| s.as_str() == user)
    }

    pub fn pass_gate(&self, at_us: u64) -> bool {
        self.first_rtp_us.compare_exchange(0, at_us, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }
}

pub struct SlotFan {
    pub audio: bool,
    pub rows: ArcSwap<Vec<Arc<DownRow>>>,
    pub ep: ArcSwap<Episode>,
    pub bind: Mutex<crate::flr::bind::Slot>,
}

impl SlotFan {
    pub fn new(audio: bool) -> SlotFan {
        SlotFan {
            audio,
            rows: ArcSwap::from_pointee(Vec::new()),
            ep: ArcSwap::from_pointee(Episode::new(1 << 40, None, 0)),
            bind: Mutex::new(crate::flr::bind::Slot::default()),
        }
    }

    pub fn speaker_is(&self, user: &str) -> bool {
        self.ep.load().speaker_is(user)
    }

    pub fn speaker(&self) -> Option<Arc<String>> {
        self.ep.load().speaker.clone()
    }

    pub fn episode(&self) -> u64 {
        self.ep.load().id
    }

    fn swap(&self, speaker: Option<Arc<String>>, grant: u64) -> u64 {
        let prev = self.ep.rcu(|cur| Episode::new(cur.id + 1, speaker.clone(), grant));
        prev.id + 1
    }

    pub fn bind(&self, speaker: Option<Arc<String>>, grant: u64) -> u64 {
        self.swap(speaker, grant)
    }

    pub fn release(&self) -> u64 {
        self.swap(None, 0)
    }

    pub fn admit(&self, user: &str) -> Option<Arc<Episode>> {
        let ep = self.ep.load_full();
        ep.speaker_is(user).then_some(ep)
    }

    pub fn live(&self, episode: u64) -> bool {
        self.ep.load().id == episode
    }
}

pub fn rtx_closed(last_ep: u64, cur_ep: u64) -> bool {
    last_ep != cur_ep
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
    pub ep: u64,
}

pub struct Egress {
    pub pkt: Pkt,
    pub targets: Vec<(Arc<DownRow>, u64)>,
    pub slot: Option<(Arc<SlotFan>, u64)>,
}

pub enum Ctl {
    Pli { tr: Arc<Transport>, track: Arc<crate::r#pub::PubTrack> },
    Sr { row: Arc<DownRow>, src_key: u64, sr: crate::rtcp::Sr, rtp: u32 },
}

impl Ctl {
    pub fn packets(&self) -> usize {
        match self {
            Ctl::Pli { .. } | Ctl::Sr { .. } => 1,
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
        let mut did = w.rows.drain();
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
        w.sleeping.store(true, Ordering::SeqCst);
        std::sync::atomic::fence(Ordering::SeqCst);
        let empty = w.rows.is_empty() && w.inbox.iter().all(|r| r.is_empty());
        if empty {
            std::thread::park_timeout(std::time::Duration::from_millis(5));
        }
        w.sleeping.store(false, Ordering::SeqCst);
        idle = 0;
    }
}

fn process(w: &Arc<Worker>, item: Item) {
    match item {
        Item::In { tr, data, at_us } => ingress(w, &tr, data, at_us),
        Item::Out(e) => {
            if let Some((fan, ep)) = &e.slot
                && !fan.live(*ep)
            {
                crate::ops::drop_count("slot_stale");
                return;
            }
            for (t, rev) in &e.targets {
                deliver(w, t, *rev, &e.pkt);
            }
        }
        Item::Ctl(c) => match c {
            Ctl::Pli { tr, track } => crate::rtcp::send_pli(w, &tr, &track),
            Ctl::Sr { row, src_key, sr, rtp } => crate::rtcp::send_sr(w, &row, src_key, &sr, rtp),
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
    let now_ms = common::now_ms();
    if let Some(prev) = track.on_ingress(&rtp, now_ms, at_us, &ids) {
        voice(w, &track, prev, now_ms);
    }
    forward(w, tr, &track, &rtp, plain.clone(), at_us);
}

fn voice_onset(prev: u64, run: u64, last: u64, now_ms: u64) -> (u64, bool) {
    let run = if prev == 0 || now_ms.saturating_sub(prev) > crate::sub::VOICE_GAP_MS { now_ms } else { run };
    let due = now_ms.saturating_sub(run) >= crate::sub::VOICE_DELAY_MS && (last == 0 || now_ms.saturating_sub(last) >= crate::sub::VOICE_RETRY_MS);
    (run, due)
}

fn voice(w: &Worker, track: &crate::r#pub::PubTrack, prev: u64, now_ms: u64) {
    let old = track.voice_run_ms.load(Ordering::Acquire);
    let (run, due) = voice_onset(prev, old, track.voice_event_ms.load(Ordering::Acquire), now_ms);
    if run != old {
        track.voice_run_ms.store(run, Ordering::Release);
    }
    if !due || !track.stream.unplaced.load(Ordering::Acquire) {
        return;
    }
    track.voice_event_ms.store(now_ms, Ordering::Release);
    rt::voice_event(w.id, &track.stream);
}

pub fn on_pub_room(stream: &crate::r#pub::PubStream, pub_room: &str) -> bool {
    stream.room.load().as_str() == pub_room
}

pub fn row_gate(stream: &crate::r#pub::PubStream, r: &DownRow) -> Option<&'static str> {
    if !r.open() {
        return Some("row_gate");
    }
    if stream.kind == Kind::Video && r.gate.load().as_ref().map(|g| !g.load(Ordering::Acquire)).unwrap_or(true) {
        return Some("ready_gate");
    }
    None
}

fn forward(w: &Arc<Worker>, tr: &Arc<Transport>, track: &Arc<crate::r#pub::PubTrack>, rtp: &Rtp<'_>, plain: Bytes, at_us: u64) {
    let now_ms = common::now_ms();
    let ids = tr.ext_ids.load();
    let stream = &track.stream;
    let audio_level = if stream.kind == Kind::Audio { ext_get(rtp.ext, ids.audio_level).and_then(|v| v.first().copied()) } else { None };
    let payload = &plain[rtp.hlen..];
    let keyframe = stream.kind == Kind::Video && crate::trk::is_keyframe(stream.codec_name(), payload);
    let mut pkt = Pkt {
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
        ep: 0,
    };
    if keyframe {
        track.last_keyframe_ms.store(now_ms, Ordering::Release);
    }
    let peer_room = tr.pub_room.load_full();
    let Some(room) = peer_room else {
        crate::ops::drop_count("no_pub_room");
        return;
    };
    let mut targets: Vec<(Arc<DownRow>, u64)> = Vec::new();
    let mut slot: Option<(Arc<SlotFan>, u64)> = None;
    if stream.is_half() {
        let Some(fan) = crate::ops::slot_of(&room, stream.kind) else {
            crate::ops::drop_count("prefan_gate");
            return;
        };
        let Some(ep) = fan.admit(&tr.user) else {
            crate::ops::drop_count("prefan_gate");
            if track.blocked_notify(at_us) {
                crate::ops::floor_blocked_rtp(&room, tr);
            }
            return;
        };
        let first = ep.pass_gate(at_us);
        ep.last_rtp_us.store(at_us, Ordering::Release);
        let prev = ep.last_notify_us.load(Ordering::Relaxed);
        if first || at_us.saturating_sub(prev) >= BLOCKED_NOTIFY_US {
            ep.last_notify_us.store(at_us, Ordering::Relaxed);
            crate::ops::floor_rtp(&room, tr, first, ep.grant);
        }
        pkt.ep = ep.id;
        for r in fan.rows.load().iter() {
            if r.user == tr.user {
                continue;
            }
            let rev = r.rev();
            if !r.open() {
                crate::ops::drop_count("row_gate");
                continue;
            }
            targets.push((r.clone(), rev));
        }
        slot = Some((fan.clone(), ep.id));
    } else {
        if !on_pub_room(stream, room.as_str()) {
            crate::ops::drop_count("no_pub_room");
            return;
        }
        for r in stream.rows.load().iter() {
            let rev = r.rev();
            if let Some(why) = row_gate(stream, r) {
                crate::ops::drop_count(why);
                continue;
            }
            if stream.kind == Kind::Video && stream.simulcast && !crate::sim::pass(r, track, keyframe, now_ms) {
                continue;
            }
            targets.push((r.clone(), rev));
        }
    }
    if targets.is_empty() {
        return;
    }
    let m = match media() {
        Some(m) => m,
        None => return,
    };
    let mut by_worker: Vec<Vec<(Arc<DownRow>, u64)>> = vec![Vec::new(); m.workers.len()];
    for t in targets {
        let i = t.0.conn.worker.min(m.workers.len() - 1);
        by_worker[i].push(t);
    }
    for (i, rows) in by_worker.into_iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        if i == w.id {
            for (r, rev) in &rows {
                deliver(w, r, *rev, &pkt);
            }
        } else if !m.workers[i].push(w.id + 1, Item::Out(Egress { pkt: pkt.clone(), targets: rows, slot: slot.clone() })) {
            crate::ops::drop_count("egress_queue_full");
        }
    }
}

pub fn deliver(w: &Worker, row: &DownRow, rev: u64, pkt: &Pkt) {
    if !row_same(row, rev) {
        crate::ops::drop_count("row_stale");
        return;
    }
    send_down(w, row, pkt);
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
    let Some(mapped) = mapped else {
        crate::ops::drop_count("pre_boundary");
        return;
    };
    if pkt.ep != 0 {
        row.last_ep.store(pkt.ep, Ordering::Release);
    }
    if (mapped.switched || row.src_track.load().is_none())
        && let Some(t) = &pkt.track
    {
        row.src_track.store(Some(t.clone()));
    }
    let twcc = conn.twcc_seq.fetch_add(1, Ordering::AcqRel);
    let table = row.ext.load();
    let mut out = Vec::with_capacity(pkt.data.len() + 32);
    let b0 = pkt.data[0] & 0x20;
    let csrc = if conn.source_tag { pkt.track.as_ref().map(|t| t.stream.csrc) } else { None };
    out.push(0x80 | b0 | u8::from(csrc.is_some()));
    out.push(core::pt_byte1(pkt.data[1], row.pt.load(Ordering::Relaxed)));
    out.extend_from_slice(&mapped.seq.to_be_bytes());
    out.extend_from_slice(&mapped.ts.to_be_bytes());
    out.extend_from_slice(&row.ssrc.to_be_bytes());
    if let Some(c) = csrc {
        out.extend_from_slice(&c.to_be_bytes());
    }
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

#[cfg(test)]
mod tests {
    use super::voice_onset;
    use crate::sub::{VOICE_DELAY_MS, VOICE_GAP_MS, VOICE_RETRY_MS};

    fn talk(from: u64, until: u64, step: u64, last: u64) -> Option<u64> {
        let (mut prev, mut run) = (0, 0);
        let mut t = from;
        while t <= until {
            let (r, due) = voice_onset(prev, run, last, t);
            if due {
                return Some(t);
            }
            (prev, run) = (t, r);
            t += step;
        }
        None
    }

    #[test]
    fn voice_fires_after_delay_of_steady_voice() {
        assert_eq!(voice_onset(0, 0, 0, 1_000), (1_000, false));
        assert_eq!(talk(1_000, 1_000 + VOICE_DELAY_MS - 20, 20, 0), None);
        assert_eq!(talk(1_000, 2_000, 20, 0), Some(1_000 + VOICE_DELAY_MS));
    }

    #[test]
    fn voice_gap_over_limit_restarts_run() {
        assert_eq!(voice_onset(1_000 + VOICE_DELAY_MS - VOICE_GAP_MS, 1_000, 0, 1_000 + VOICE_DELAY_MS), (1_000, true));
        let prev = 1_000 + VOICE_DELAY_MS - VOICE_GAP_MS - 1;
        assert_eq!(voice_onset(prev, 1_000, 0, 1_000 + VOICE_DELAY_MS), (1_000 + VOICE_DELAY_MS, false));
    }

    #[test]
    fn voice_retry_waits_r_after_last_event() {
        let last = 1_200;
        assert_eq!(voice_onset(last + VOICE_RETRY_MS - 40, 1_000, last, last + VOICE_RETRY_MS - 20), (1_000, false));
        assert_eq!(voice_onset(last + VOICE_RETRY_MS - 20, 1_000, last, last + VOICE_RETRY_MS), (1_000, true));
    }
}
