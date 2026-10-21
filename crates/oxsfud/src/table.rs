// author: kodeholic (powered by Claude)
// spec: v1.2 @124a233 · 정:IMPL-SUB-PIPELINE 정:IMPL-SUB-TABLE 정:IMPL-SUB-SUBSCRIBER-TABLE 정:IMPL-SUB-READY-GATE 정:IMPL-PUB-RID-SPATIAL 연:RULE-SUB-FILL 연:RULE-SUB-TABLE-SERVER 연:RULE-SUB-EGRESS-ACKED 연:RULE-SUB-TGATE 연:RULE-ROOM-BUDGET-SPLIT · model: claude-opus-5-5
//! 배정표 — 연결마다 하나. 줄 하나 = 구독 m-line 하나. 줄은 지우지 않고, 바뀔 때마다 `arev` 를 올려 표 전체를 보낸다.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use arc_swap::ArcSwapOption;
use oxsig::Op;
use oxsig::code::Code;
use oxsig::wire::{Kind, MlineEntry, MlineEventBody, MlineTable, PcMode, ReadyReq, SubscribeLayerReq};

use crate::media::{LAYER_H, PubStream, Rewriter};
use crate::peer::{Conn, Peer};
use crate::room::Room;
use crate::rtp;
use crate::sfu::{Sfu, lock, mono_ms};

pub const FULL_CAP: usize = 25;
pub const TGATE_MS: u64 = 5_000;
const MID_WINDOW_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowClass {
    Slot,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrackKey {
    Slot { room: String, kind_video: bool },
    Track(String),
}

pub struct Bind {
    pub key: TrackKey,
    pub room_id: String,
    pub stream: Option<Arc<PubStream>>,
    pub slot_room: Option<Weak<Room>>,
}

pub struct Row {
    pub mid: u8,
    pub kind: Kind,
    pub class: RowClass,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub pt: AtomicU8,
    pub rtx_pt: AtomicU8,
    pub tuple: Mutex<(String, String, Option<String>)>,
    pub user_id: String,
    pub conn: Weak<Conn>,
    pub since: AtomicU64,
    pub open: AtomicBool,
    pub gated_at: AtomicU64,
    pub tgate_latched: AtomicBool,
    pub bind: ArcSwapOption<Bind>,
    pub rw: Mutex<Rewriter>,
    pub egress: AtomicU64,
    pub mid_until: AtomicU64,
    pub want_key: AtomicBool,
    pub key_req_at: AtomicU64,
    pub sim: Mutex<crate::layer::SimSel>,
    pub bound_at: AtomicU64,
    pub rtx_seq: AtomicU32,
    pub stall: Mutex<(u64, u64)>,
}

impl Row {
    pub fn key(&self) -> Option<TrackKey> {
        self.bind.load().as_ref().map(|b| b.key.clone())
    }

    pub fn room_id(&self) -> Option<String> {
        self.bind.load().as_ref().map(|b| b.room_id.clone())
    }

    pub fn stream(&self) -> Option<Arc<PubStream>> {
        self.bind.load().as_ref().and_then(|b| b.stream.clone())
    }

    pub fn entry(&self) -> MlineEntry {
        let (_, codec, fmtp) = lock(&self.tuple).clone();
        let b = self.bind.load();
        let (track_id, room_id) = match b.as_ref() {
            Some(b) => (
                Some(match &b.key {
                    TrackKey::Slot { room, kind_video } => slot_track_id(room, *kind_video),
                    TrackKey::Track(t) => t.clone(),
                }),
                Some(b.room_id.clone()),
            ),
            None => (None, None),
        };
        let rtx_pt = self.rtx_pt.load(Ordering::Relaxed);
        MlineEntry {
            mid: self.mid.to_string(),
            kind: self.kind,
            pt: self.pt.load(Ordering::Relaxed),
            codec,
            fmtp,
            rtx_pt: (rtx_pt != 0).then_some(rtx_pt),
            track_id,
            room_id,
            ssrc: self.ssrc,
            rtx_ssrc: self.rtx_ssrc,
            stream_id: match self.class {
                RowClass::Slot => "ox-ptt".to_string(),
                RowClass::Full => format!("ox-m{}", self.mid),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Pref {
    pub paused: bool,
    pub priority: u8,
    pub spatial: u8,
    pub temporal: u8,
}

impl Default for Pref {
    fn default() -> Self {
        Pref { paused: false, priority: 128, spatial: LAYER_H, temporal: 0 }
    }
}

pub struct Table {
    pub rows: Vec<Arc<Row>>,
    pub next_mid: u16,
    pub pts: HashMap<String, (u8, Option<u8>)>,
    pub used: BTreeSet<u8>,
    pub tagged: HashMap<String, (u8, Option<u8>)>,
    pub ssrcs: HashSet<u32>,
    pub waiting: HashMap<TrackKey, u64>,
    pub prefs: HashMap<String, Pref>,
    pub h264_seeded: bool,
}

impl Table {
    pub fn new(mid_start: u8) -> Table {
        Table {
            rows: Vec::new(),
            next_mid: mid_start as u16,
            pts: HashMap::new(),
            used: BTreeSet::new(),
            tagged: HashMap::new(),
            ssrcs: HashSet::new(),
            waiting: HashMap::new(),
            prefs: HashMap::new(),
            h264_seeded: false,
        }
    }

    pub fn seed_tags(&mut self, tags: &crate::peer::JoinTags) {
        for c in &tags.codecs {
            self.used.insert(c.pt);
            if let Some(r) = c.rtx_pt {
                self.used.insert(r);
            }
            let kind = if c.kind == Kind::Video { "video" } else { "audio" };
            let key = rtp::tuple_key(kind, &c.name, c.fmtp.as_deref());
            self.tagged.entry(key).or_insert((c.pt, c.rtx_pt));
        }
    }

    fn free_pt(&self, from: u8, to: u8) -> Option<u8> {
        (from..=to).find(|p| !self.used.contains(p))
    }

    fn free_pair(&self) -> Option<(u8, u8)> {
        for (a, b) in [(96u8, 127u8), (35u8, 63u8)] {
            let mut p = a;
            while p < b {
                if !self.used.contains(&p) && !self.used.contains(&(p + 1)) {
                    return Some((p, p + 1));
                }
                p += 1;
            }
        }
        None
    }

    pub fn pt_for(&mut self, pc: PcMode, kind: Kind, codec: &str, fmtp: Option<&str>) -> Option<(u8, Option<u8>)> {
        let kname = if kind == Kind::Video { "video" } else { "audio" };
        let key = rtp::tuple_key(kname, codec, fmtp);
        if let Some(v) = self.pts.get(&key) {
            return Some(*v);
        }
        let video = kind == Kind::Video;
        let mut pick: Option<(u8, Option<u8>)> = None;
        if pc == PcMode::V1pc
            && let Some(t) = self.tagged.get(&key)
        {
            let rtx = if video { t.1.or_else(|| self.free_pt(96, 127).or_else(|| self.free_pt(35, 63))) } else { None };
            pick = Some((t.0, rtx));
        }
        if pick.is_none() && pc == PcMode::V2pc {
            let seed = match codec.to_ascii_uppercase().as_str() {
                "OPUS" => Some((111u8, None)),
                "VP8" => Some((96u8, Some(97u8))),
                "H264" if !self.h264_seeded => Some((102u8, Some(103u8))),
                _ => None,
            };
            if let Some((p, r)) = seed
                && !self.used.contains(&p)
                && r.is_none_or(|r| !self.used.contains(&r))
            {
                pick = Some((p, r));
            }
        }
        if pick.is_none() {
            pick = if video {
                self.free_pair().map(|(a, b)| (a, Some(b)))
            } else {
                self.free_pt(96, 127).or_else(|| self.free_pt(35, 63)).map(|p| (p, None))
            };
        }
        let v = pick?;
        if codec.eq_ignore_ascii_case("H264") {
            self.h264_seeded = true;
        }
        self.used.insert(v.0);
        if let Some(r) = v.1 {
            self.used.insert(r);
        }
        self.pts.insert(key, v);
        Some(v)
    }

    pub fn snapshot(&self, sfu: &Sfu, arev: u64) -> MlineTable {
        MlineTable { sfu_id: sfu.epoch.clone(), arev, mlines: self.rows.iter().map(|r| r.entry()).collect() }
    }

    pub fn count(&self, kind: Kind, class: RowClass) -> usize {
        self.rows.iter().filter(|r| r.kind == kind && r.class == class).count()
    }
}

pub fn slot_track_id(room: &str, video: bool) -> String {
    format!("ptt-{room}-{}", if video { "video" } else { "audio" })
}

fn new_ssrc(t: &Table, avoid: &HashSet<u32>) -> u32 {
    loop {
        let b = common::ident::random_bytes(4);
        let v = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        if v != 0 && !t.ssrcs.contains(&v) && !avoid.contains(&v) {
            return v;
        }
    }
}

pub fn current_arev(sfu: &Sfu, sid: &str) -> u64 {
    lock(&sfu.arevs).get(sid).copied().unwrap_or(0)
}

fn bump_arev(sfu: &Sfu, sid: &str) -> u64 {
    let mut m = lock(&sfu.arevs);
    let n = m.get(sid).copied().unwrap_or(0).saturating_add(1);
    m.insert(sid.to_string(), n);
    n
}

struct Rev<'a> {
    sfu: &'a Sfu,
    sid: &'a str,
    v: Option<u64>,
}

impl Rev<'_> {
    fn take(&mut self) -> u64 {
        match self.v {
            Some(v) => v,
            None => {
                let v = bump_arev(self.sfu, self.sid);
                self.v = Some(v);
                v
            }
        }
    }
}

pub fn uplink_ssrcs(peer: &Peer) -> HashSet<u32> {
    let mut s = HashSet::new();
    for st in lock(&peer.streams).iter() {
        s.insert(st.ssrc);
        if let Some(r) = st.rtx_ssrc {
            s.insert(r);
        }
        for t in st.tracks.load().iter() {
            s.insert(t.ssrc);
        }
    }
    s
}

#[derive(Clone)]
struct Want {
    key: TrackKey,
    room: Arc<Room>,
    stream: Option<Arc<PubStream>>,
    kind: Kind,
    rank: (u8, u64),
}

fn wants(peer: &Peer, rooms: &[Arc<Room>], t: &Table) -> Vec<Want> {
    let mut out = Vec::new();
    for r in rooms {
        for kv in [false, true] {
            out.push(Want {
                key: TrackKey::Slot { room: r.id.clone(), kind_video: kv },
                room: r.clone(),
                stream: None,
                kind: if kv { Kind::Video } else { Kind::Audio },
                rank: (255, u64::MAX),
            });
        }
        for s in r.listed_tracks() {
            if s.user_id == peer.user_id || s.is_half() || !s.listed.load(Ordering::Relaxed) {
                continue;
            }
            let pref = t.prefs.get(&s.track_id).copied().unwrap_or_default();
            if s.is_video() && pref.paused {
                continue;
            }
            let rank = if s.is_video() {
                (pref.priority, u64::MAX - t.waiting.get(&TrackKey::Track(s.track_id.clone())).copied().unwrap_or(0))
            } else {
                (128, s.level_at.load(Ordering::Relaxed))
            };
            out.push(Want { key: TrackKey::Track(s.track_id.clone()), room: r.clone(), stream: Some(s.clone()), kind: s.kind, rank });
        }
    }
    out
}

fn row_rank(row: &Row, t: &Table) -> (u8, u64) {
    match row.stream() {
        Some(s) if s.is_video() => {
            let pref = t.prefs.get(&s.track_id).copied().unwrap_or_default();
            (pref.priority, u64::MAX - row.bound_at.load(Ordering::Relaxed))
        }
        Some(s) => (128, s.level_at.load(Ordering::Relaxed)),
        None => (0, 0),
    }
}

fn unbind(row: &Arc<Row>) {
    let old = row.bind.swap(None);
    if let Some(b) = old {
        if let Some(s) = &b.stream {
            s.del_sub(row);
        }
        if let Some(r) = b.slot_room.as_ref().and_then(Weak::upgrade) {
            r.slot(row.kind).del_row(row);
        }
    }
}

fn bind(row: &Arc<Row>, w: &Want, arev: u64, now: u64, cap: u8) {
    unbind(row);
    let b = Bind {
        key: w.key.clone(),
        room_id: w.room.id.clone(),
        stream: w.stream.clone(),
        slot_room: if w.stream.is_none() { Some(Arc::downgrade(&w.room)) } else { None },
    };
    row.since.store(arev, Ordering::Relaxed);
    row.bound_at.store(now, Ordering::Relaxed);
    row.mid_until.store(now + MID_WINDOW_MS, Ordering::Relaxed);
    row.want_key.store(true, Ordering::Relaxed);
    row.tgate_latched.store(false, Ordering::Relaxed);
    let gate = row.kind == Kind::Video && row.class == RowClass::Full;
    row.open.store(!gate, Ordering::Relaxed);
    row.gated_at.store(now, Ordering::Relaxed);
    {
        let mut g = lock(&row.sim);
        let _ = std::mem::replace(&mut *g, crate::layer::SimSel { cap, ..crate::layer::SimSel::default() });
    }
    row.bind.store(Some(Arc::new(b)));
    match &w.stream {
        Some(s) => s.add_sub(row),
        None => w.room.slot(row.kind).add_row(row),
    }
}

struct RowSpec<'a> {
    pc: PcMode,
    kind: Kind,
    class: RowClass,
    codec: &'a str,
    user: &'a str,
}

fn make_row(conn: &Arc<Conn>, t: &mut Table, spec: &RowSpec<'_>, avoid: &HashSet<u32>) -> Option<Arc<Row>> {
    if t.next_mid > 255 {
        return None;
    }
    let kind = spec.kind;
    let fmtp = rtp::sub_fmtp(spec.codec).map(str::to_string);
    let (pt, rtx_pt) = t.pt_for(spec.pc, kind, spec.codec, fmtp.as_deref())?;
    let ssrc = new_ssrc(t, avoid);
    t.ssrcs.insert(ssrc);
    let rtx_ssrc = if kind == Kind::Video {
        let r = new_ssrc(t, avoid);
        t.ssrcs.insert(r);
        Some(r)
    } else {
        None
    };
    let kname = if kind == Kind::Video { "video" } else { "audio" };
    let row = Arc::new(Row {
        mid: t.next_mid as u8,
        kind,
        class: spec.class,
        ssrc,
        rtx_ssrc,
        pt: AtomicU8::new(pt),
        rtx_pt: AtomicU8::new(if kind == Kind::Video { rtx_pt.unwrap_or(0) } else { 0 }),
        tuple: Mutex::new((rtp::tuple_key(kname, spec.codec, fmtp.as_deref()), rtp::canon_codec(spec.codec), fmtp)),
        user_id: spec.user.to_string(),
        conn: Arc::downgrade(conn),
        since: AtomicU64::new(0),
        open: AtomicBool::new(true),
        gated_at: AtomicU64::new(0),
        tgate_latched: AtomicBool::new(false),
        bind: ArcSwapOption::empty(),
        rw: Mutex::new(Rewriter { clock: rtp::clock(kind == Kind::Video), ..Rewriter::default() }),
        egress: AtomicU64::new(0),
        mid_until: AtomicU64::new(0),
        want_key: AtomicBool::new(false),
        key_req_at: AtomicU64::new(0),
        sim: Mutex::new(crate::layer::SimSel::default()),
        bound_at: AtomicU64::new(0),
        rtx_seq: AtomicU32::new(1),
        stall: Mutex::new((0, 0)),
    });
    t.next_mid += 1;
    t.rows.push(row.clone());
    Some(row)
}

pub fn room_codec(room: &Room, kind: Kind) -> String {
    match kind {
        Kind::Audio => room.codecs.audio.clone().unwrap_or_else(|| "opus".into()),
        Kind::Video => room.codecs.video.clone().unwrap_or_else(|| "H264".into()),
    }
}

pub struct Filled {
    pub table: MlineTable,
}

pub fn slot_budget(sfu: &Sfu, peer: &Peer) -> (usize, usize) {
    let t = lock(&peer.sub_conn.table);
    let limit = sfu.policy.media.slot_mid_reserve as usize;
    let used = t.rows.iter().filter(|r| r.class == RowClass::Slot && r.bind.load().is_some()).count();
    (limit, used)
}

pub fn fill(sfu: &Sfu, peer: &Arc<Peer>, notify: bool) -> Filled {
    let conn = peer.sub_conn.clone();
    let rooms: Vec<Arc<Room>> = peer.room_ids().iter().filter_map(|r| sfu.room(r)).collect();
    let now = mono_ms();
    let avoid = if peer.pc_mode == PcMode::V1pc { uplink_ssrcs(peer) } else { HashSet::new() };
    let mut t = lock(&conn.table);
    let want = wants(peer, &rooms, &t);
    let want_keys: HashSet<TrackKey> = want.iter().map(|w| w.key.clone()).collect();
    let mut changed = false;
    let mut rev = Rev { sfu, sid: &peer.session_id, v: None };
    for row in t.rows.clone() {
        let Some(b) = row.bind.load_full() else { continue };
        if !want_keys.contains(&b.key) {
            unbind(&row);
            changed = true;
            rev.take();
            continue;
        }
        if let Some(w) = want.iter().find(|w| w.key == b.key)
            && w.room.id != b.room_id
        {
            let nb = Bind { key: b.key.clone(), room_id: w.room.id.clone(), stream: b.stream.clone(), slot_room: b.slot_room.clone() };
            row.bind.store(Some(Arc::new(nb)));
            changed = true;
            rev.take();
        }
    }
    let bound: HashSet<TrackKey> = t.rows.iter().filter_map(|r| r.key()).collect();
    let mut pending: Vec<Want> = want.into_iter().filter(|w| !bound.contains(&w.key)).collect();
    pending.sort_by(|a, b| {
        let sa = matches!(a.key, TrackKey::Slot { .. });
        let sb = matches!(b.key, TrackKey::Slot { .. });
        sb.cmp(&sa).then(b.rank.cmp(&a.rank))
    });
    let mut still_waiting: HashMap<TrackKey, u64> = HashMap::new();
    for w in pending {
        let class = if w.stream.is_none() { RowClass::Slot } else { RowClass::Full };
        let codec = match &w.stream {
            Some(s) if s.kind == Kind::Video => s.codec.clone(),
            _ => room_codec(&w.room, w.kind),
        };
        let kname = if w.kind == Kind::Video { "video" } else { "audio" };
        let key = rtp::tuple_key(kname, &codec, rtp::sub_fmtp(&codec));
        let empty = t
            .rows
            .iter()
            .filter(|r| r.kind == w.kind && r.class == class && r.bind.load().is_none() && lock(&r.tuple).0 == key)
            .min_by_key(|r| r.mid)
            .cloned();
        let row = match empty {
            Some(r) => Some(r),
            None => {
                let cap_ok = match class {
                    RowClass::Slot => t.count(w.kind, RowClass::Slot) < sfu.policy.media.slot_mid_reserve as usize,
                    RowClass::Full => t.count(w.kind, RowClass::Full) < FULL_CAP,
                };
                if cap_ok {
                    let spec = RowSpec { pc: peer.pc_mode, kind: w.kind, class, codec: &codec, user: &peer.user_id };
                    make_row(&conn, &mut t, &spec, &avoid)
                } else if class == RowClass::Full {
                    let victim = t
                        .rows
                        .iter()
                        .filter(|r| r.kind == w.kind && r.class == RowClass::Full && r.bind.load().is_some())
                        .filter(|r| lock(&r.tuple).0 == key)
                        .map(|r| (row_rank(r, &t), r.clone()))
                        .filter(|(rk, _)| *rk < w.rank)
                        .min_by_key(|(rk, _)| *rk)
                        .map(|(_, r)| r);
                    if let Some(v) = &victim
                        && let Some(k) = v.key()
                    {
                        still_waiting.insert(k, now);
                    }
                    victim
                } else {
                    None
                }
            }
        };
        match row {
            Some(r) => {
                changed = true;
                let a = rev.take();
                let cap = match &w.stream {
                    Some(s) => t.prefs.get(&s.track_id).copied().unwrap_or_default().spatial,
                    None => LAYER_H,
                };
                bind(&r, &w, a, now, cap);
            }
            None => {
                let since = t.waiting.get(&w.key).copied().unwrap_or(now);
                still_waiting.insert(w.key.clone(), since);
            }
        }
    }
    t.waiting = still_waiting;
    let a = rev.v.unwrap_or_else(|| current_arev(sfu, &peer.session_id));
    let table = t.snapshot(sfu, a);
    drop(t);
    if changed && notify {
        sfu.emit_user(&peer.user_id, Op::MlineEvent, &MlineEventBody { table: table.clone() });
    }
    Filled { table }
}

pub fn table_now(sfu: &Sfu, peer: &Peer) -> MlineTable {
    lock(&peer.sub_conn.table).snapshot(sfu, current_arev(sfu, &peer.session_id))
}

pub fn drop_stream_rows(stream: &Arc<PubStream>) -> Vec<Arc<Peer>> {
    let mut touched: Vec<Arc<Peer>> = Vec::new();
    for w in stream.subs.load().iter() {
        let Some(row) = w.upgrade() else { continue };
        unbind(&row);
        if let Some(p) = row.conn.upgrade().and_then(|c| c.peer())
            && !touched.iter().any(|x| Arc::ptr_eq(x, &p))
        {
            touched.push(p);
        }
    }
    stream.subs.store(Arc::new(Vec::new()));
    touched
}

pub fn clear(conn: &Conn) {
    let t = lock(&conn.table);
    for row in &t.rows {
        unbind(row);
    }
}

pub fn ready(sfu: &Arc<Sfu>, peer: &Arc<Peer>, body: &str) -> Result<String, (Code, String)> {
    let req: ReadyReq = serde_json::from_str(body).map_err(|e| {
        if e.to_string().contains("missing field") { (Code::MISSING_FIELD, e.to_string()) } else { (Code::INVALID_PAYLOAD, e.to_string()) }
    })?;
    let room = sfu.room(&req.room_id).ok_or((Code::ROOM_NOT_FOUND, "room not found".to_string()))?;
    if !peer.in_room(&req.room_id) {
        return Err((Code::NOT_IN_ROOM, "not in room".into()));
    }
    let now = mono_ms();
    {
        let mut m = lock(&peer.ready);
        let e = m.entry(req.room_id.clone()).or_default();
        if e.first_at == 0 {
            e.first_at = now;
        }
        e.open = true;
    }
    let conn = peer.sub_conn.clone();
    conn.ack.fetch_max(req.arev, Ordering::Relaxed);
    let mut keys: Vec<Arc<PubStream>> = Vec::new();
    {
        let t = lock(&conn.table);
        for row in &t.rows {
            if row.room_id().as_deref() != Some(req.room_id.as_str()) {
                continue;
            }
            row.open.store(true, Ordering::Relaxed);
            if row.kind == Kind::Video {
                if let Some(s) = row.stream() {
                    if !keys.iter().any(|k| Arc::ptr_eq(k, &s)) {
                        keys.push(s);
                    }
                } else if let Some(s) = room.slot(Kind::Video).bound.load_full()
                    && !keys.iter().any(|k| Arc::ptr_eq(k, &s))
                {
                    keys.push(s);
                }
            }
        }
    }
    for s in keys {
        crate::media::request_keyframe(sfu, &s, None, true);
    }
    crate::floor::first_notice(sfu, &room, &peer.user_id);
    Ok("{}".into())
}

pub fn tgate_sweep(sfu: &Sfu, now: u64) {
    let peers: Vec<Arc<Peer>> = lock(&sfu.peers).values().cloned().collect();
    for p in peers {
        let conn = p.sub_conn.clone();
        let t = lock(&conn.table);
        for row in &t.rows {
            if row.bind.load().is_none() {
                continue;
            }
            let gated = !row.open.load(Ordering::Relaxed) || row.since.load(Ordering::Relaxed) > conn.ack.load(Ordering::Relaxed);
            if gated && now.saturating_sub(row.gated_at.load(Ordering::Relaxed)) >= TGATE_MS {
                row.open.store(true, Ordering::Relaxed);
                conn.ack.fetch_max(row.since.load(Ordering::Relaxed), Ordering::Relaxed);
                if !row.tgate_latched.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        kind = "diag",
                        spec = "RULE-SUB-TGATE",
                        user = p.user_id.as_str(),
                        room = row.room_id().unwrap_or_default().as_str(),
                        mid = row.mid,
                        "T-gate expired — READY 미수신 의심"
                    );
                }
            }
        }
    }
}

pub fn subscribe_layer(sfu: &Arc<Sfu>, peer: &Arc<Peer>, body: &str) -> Result<String, (Code, String)> {
    let req: SubscribeLayerReq = serde_json::from_str(body).map_err(|e| (Code::INVALID_PAYLOAD, e.to_string()))?;
    if !peer.in_room(&req.room_id) {
        return Err((Code::NOT_IN_ROOM, "not in room".into()));
    }
    let mut unpaused: Vec<Arc<PubStream>> = Vec::new();
    let mut refill = false;
    {
        let rooms: Vec<Arc<Room>> = peer.room_ids().iter().filter_map(|r| sfu.room(r)).collect();
        let mut t = lock(&peer.sub_conn.table);
        for tg in &req.targets {
            let Some(stream) = rooms.iter().flat_map(|r| r.listed_tracks()).find(|s| s.track_id == tg.track_id) else {
                continue;
            };
            let p = t.prefs.entry(tg.track_id.clone()).or_default();
            let max_spatial = if stream.simulcast { 1 } else { 0 };
            if let Some(s) = tg.spatial {
                p.spatial = s.min(max_spatial);
                let cap = p.spatial;
                for row in stream.subs.load().iter().filter_map(|w| w.upgrade()) {
                    if row.user_id == peer.user_id {
                        lock(&row.sim).cap = cap;
                    }
                }
            }
            if tg.temporal.is_some() {
                p.temporal = 0;
            }
            if let Some(pr) = tg.priority {
                refill |= pr.max(1) != p.priority;
                p.priority = pr.max(1);
            }
            if let Some(pa) = tg.paused {
                if p.paused && !pa {
                    unpaused.push(stream.clone());
                }
                refill |= p.paused != pa;
                p.paused = pa;
            }
        }
    }
    if refill {
        fill(sfu, peer, true);
    }
    for s in unpaused {
        crate::media::request_keyframe(sfu, &s, None, true);
    }
    Ok("{}".into())
}

pub fn pref(conn: &Conn, track_id: &str) -> Pref {
    lock(&conn.table).prefs.get(track_id).copied().unwrap_or_default()
}

pub fn waiting_audio(conn: &Conn) -> bool {
    let t = lock(&conn.table);
    !t.waiting.is_empty() && t.count(Kind::Audio, RowClass::Full) >= FULL_CAP
}
