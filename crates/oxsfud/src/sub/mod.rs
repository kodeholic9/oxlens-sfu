// author: kodeholic (powered by Claude)
// spec: v1.6 · 정:IMPL-SUB-PIPELINE · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-SUB-READY-GATE · 정:RULE-SUB-STATE · 연:SUB.type.MlineTable · 연:RULE-SUB-FILL · 연:RULE-ROOM-BUDGET-SPLIT · 정:RULE-SUB-SERVER-SCOPE · 정:CAND-SUB-ROW-CLASS · 정:CAND-SUB-PAUSED-FREES-ROW · 정:CAND-SUB-REASSIGN-TICK · 연:RULE-SUB-TABLE-ONE-SOURCE · model: claude-opus-5-5
//! 구독 — 연결마다 배정표 하나(줄 = 구독 m-line · 다운링크 SSRC 는 줄 수명 동안 고정 · `since` ≤ ack 만 송신) · 구독자 표(튜플 → PT 짝 · 확장 번호) · 빈 줄은 같은 부류 · kind · 튜플의 가장 작은 mid 부터 · 상한이면 순위로 재배정 · READY 게이트.

pub mod ext;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oxsig::msg::{ExtMap, JoinCodec, Kind, MlineEntry, MlineTable, PcMode};

use crate::fwd::DownRow;
use crate::link::Transport;
use ext::ExtTable;

pub const START_1PC: u16 = 32;
pub const START_2PC: u16 = 0;
pub const SUB_1PC: usize = 96;
pub const SUB_2PC: usize = 128;
pub const FULL_AUDIO: usize = 25;
pub const FULL_VIDEO: usize = 25;
pub const T_GATE_MS: u64 = 5_000;
pub const SLOT_STREAM: &str = "ox-ptt";

pub const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const URI_REPAIR_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";

pub fn server_extmap() -> Vec<ExtMap> {
    [(1, URI_MID), (4, URI_AUDIO_LEVEL), (5, URI_ABS_SEND_TIME), (6, URI_TWCC), (10, URI_RID), (11, URI_REPAIR_RID)]
        .into_iter()
        .map(|(id, uri)| ExtMap { id, uri: uri.to_string() })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Full,
    Slot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubState {
    Created,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tuple(pub String);

pub fn tuple_of(kind: Kind, codec: &str) -> Tuple {
    let name = codec.to_ascii_lowercase();
    let t = match kind {
        Kind::Audio => format!("{name}/48000/2"),
        Kind::Video if name == "h264" => "h264/90000/pm1/42e0".to_string(),
        Kind::Video => format!("{name}/90000"),
    };
    Tuple(t)
}

fn join_tuple(c: &JoinCodec) -> Option<Tuple> {
    let name = c.name.to_ascii_lowercase();
    match c.kind {
        Kind::Audio if name == "opus" => {
            let st = crate::trk::fmtp_param(c.fmtp.as_deref(), "sprop-stereo").unwrap_or_else(|| "0".into());
            if st == "1" { Some(Tuple("opus/48000/2/stereo".into())) } else { Some(tuple_of(Kind::Audio, "opus")) }
        }
        Kind::Video if name == "h264" => {
            let pm = crate::trk::fmtp_param(c.fmtp.as_deref(), "packetization-mode").unwrap_or_else(|| "0".into());
            let pr = crate::trk::h264_profile(c.fmtp.as_deref()).unwrap_or_default();
            Some(Tuple(format!("h264/90000/pm{pm}/{pr}")))
        }
        Kind::Video => Some(tuple_of(Kind::Video, &name)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Cand {
    pub track_id: String,
    pub room: String,
    pub kind: Kind,
    pub class: Class,
    pub codec: String,
    pub rank: i64,
    pub paused: bool,
}

pub struct Row {
    pub mid: u16,
    pub kind: Kind,
    pub class: Class,
    pub tuple: Tuple,
    pub codec: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub track: Option<(String, String)>,
    pub since: u64,
    pub loaded_at: u64,
    pub state: SubState,
    pub down: Arc<DownRow>,
}

impl Row {
    pub fn entry(&self) -> MlineEntry {
        MlineEntry {
            mid: self.mid.to_string(),
            kind: self.kind,
            pt: self.pt,
            codec: self.codec.clone(),
            fmtp: crate::trk::row_fmtp(&self.codec),
            rtx_pt: self.rtx_pt,
            track_id: self.track.as_ref().map(|t| t.0.clone()),
            room_id: self.track.as_ref().map(|t| t.1.clone()),
            ssrc: self.ssrc,
            rtx_ssrc: self.rtx_ssrc,
            stream_id: match self.class {
                Class::Full => format!("ox-m{}", self.mid),
                Class::Slot => SLOT_STREAM.to_string(),
            },
        }
    }
}

#[derive(Clone)]
pub struct Change {
    pub down: Arc<DownRow>,
    pub class: Class,
    pub old: Option<(String, String)>,
    pub new: Option<(String, String)>,
}

#[derive(Default)]
pub struct SyncOut {
    pub changes: Vec<Change>,
    pub changed: bool,
    pub mid_limit: bool,
}

#[derive(Default)]
pub struct State {
    pub rows: Vec<Row>,
    pub arev: u64,
    pts: BTreeMap<Tuple, (u8, Option<u8>)>,
    reserved: BTreeSet<u8>,
    ssrcs: BTreeSet<u32>,
    pub uplink: BTreeSet<u32>,
    pub waiting: HashMap<String, u64>,
    pub gates: HashMap<String, (Arc<AtomicBool>, u64, bool)>,
    pub ready_at: HashMap<String, u64>,
    pub seeded: bool,
    pub prefs: HashMap<String, (u8, bool)>,
}

pub struct Table {
    pub conn: Arc<Transport>,
    pub pc_mode: PcMode,
    pub ext: arc_swap::ArcSwap<ExtTable>,
    pub acked: Arc<AtomicU64>,
    pub st: Mutex<State>,
    pub slot_budget: usize,
}

fn rand_u32() -> u32 {
    common::random_bytes(4).iter().fold(0u32, |a, b| (a << 8) | *b as u32)
}

impl Table {
    pub fn new(conn: Arc<Transport>, pc_mode: PcMode, slot_budget: usize) -> Arc<Table> {
        Arc::new(Table {
            conn,
            pc_mode,
            ext: arc_swap::ArcSwap::from_pointee(ext_table(&server_extmap())),
            acked: Arc::new(AtomicU64::new(0)),
            st: Mutex::new(State::default()),
            slot_budget,
        })
    }

    fn start(&self) -> u16 {
        if self.pc_mode == PcMode::One { START_1PC } else { START_2PC }
    }

    fn limit(&self) -> usize {
        if self.pc_mode == PcMode::One { SUB_1PC } else { SUB_2PC }
    }

    pub fn seed(&self, codecs: &[JoinCodec], extmap: &[ExtMap]) -> bool {
        let Ok(mut s) = self.st.lock() else { return false };
        if s.seeded {
            return false;
        }
        s.seeded = true;
        for c in codecs {
            s.reserved.insert(c.pt);
            if let Some(r) = c.rtx_pt {
                s.reserved.insert(r);
            }
            if let Some(t) = join_tuple(c) {
                s.pts.entry(t).or_insert((c.pt, c.rtx_pt));
            }
        }
        if !extmap.is_empty() {
            let mut merged = server_extmap();
            for m in &mut merged {
                if let Some(x) = extmap.iter().find(|e| e.uri == m.uri) {
                    m.id = x.id;
                }
            }
            self.ext.store(Arc::new(ext_table(&merged)));
        }
        true
    }

    pub fn mark_seeded(&self) {
        if let Ok(mut s) = self.st.lock() {
            s.seeded = true;
        }
    }

    pub fn snapshot(&self, sfu_id: &str) -> MlineTable {
        let s = self.st.lock();
        match s {
            Ok(s) => MlineTable { sfu_id: sfu_id.to_string(), arev: s.arev, mlines: s.rows.iter().map(|r| r.entry()).collect() },
            Err(_) => MlineTable { sfu_id: sfu_id.to_string(), arev: 0, mlines: Vec::new() },
        }
    }

    pub fn arev(&self) -> u64 {
        self.st.lock().map(|s| s.arev).unwrap_or(0)
    }

    pub fn down_ssrcs(&self) -> BTreeSet<u32> {
        self.st.lock().map(|s| s.ssrcs.clone()).unwrap_or_default()
    }

    pub fn add_uplink(&self, ssrcs: &[u32]) {
        if let Ok(mut s) = self.st.lock() {
            s.uplink.extend(ssrcs.iter().copied());
        }
    }

    pub fn remove_uplink(&self, ssrcs: &[u32]) {
        if let Ok(mut s) = self.st.lock() {
            for x in ssrcs {
                s.uplink.remove(x);
            }
        }
    }

    pub fn rows(&self) -> Vec<Arc<DownRow>> {
        self.st.lock().map(|s| s.rows.iter().map(|r| r.down.clone()).collect()).unwrap_or_default()
    }

    pub fn row_by_ssrc(&self, ssrc: u32) -> Option<(Arc<DownRow>, bool)> {
        let s = self.st.lock().ok()?;
        s.rows.iter().find_map(|r| {
            if r.ssrc == ssrc {
                Some((r.down.clone(), false))
            } else if r.rtx_ssrc == Some(ssrc) {
                Some((r.down.clone(), true))
            } else {
                None
            }
        })
    }

    pub fn loaded(&self) -> Vec<(String, String, Arc<DownRow>)> {
        self.st
            .lock()
            .map(|s| s.rows.iter().filter_map(|r| r.track.as_ref().map(|t| (t.0.clone(), t.1.clone(), r.down.clone()))).collect())
            .unwrap_or_default()
    }

    fn fresh_ssrc(s: &mut State) -> u32 {
        loop {
            let v = rand_u32();
            if v > 1 && !s.ssrcs.contains(&v) && !s.uplink.contains(&v) {
                s.ssrcs.insert(v);
                return v;
            }
        }
    }

    fn pt_for(&self, s: &mut State, kind: Kind, t: &Tuple, codec: &str) -> (u8, Option<u8>) {
        if let Some(p) = s.pts.get(t) {
            return *p;
        }
        let used: BTreeSet<u8> = s.pts.values().flat_map(|(a, b)| [Some(*a), *b]).flatten().chain(s.reserved.iter().copied()).collect();
        let seed: Option<(u8, Option<u8>)> = if self.pc_mode == PcMode::Two {
            match (kind, codec.to_ascii_lowercase().as_str()) {
                (Kind::Audio, "opus") => Some((111, None)),
                (Kind::Video, "vp8") => Some((96, Some(97))),
                (Kind::Video, "h264") => Some((102, Some(103))),
                _ => None,
            }
        } else {
            None
        };
        let p = match seed.filter(|(a, b)| !used.contains(a) && b.map(|b| !used.contains(&b)).unwrap_or(true)) {
            Some(p) => p,
            None => {
                let free: Vec<u8> = (96u8..=127).chain(35u8..=63).filter(|x| !used.contains(x)).collect();
                if kind == Kind::Audio {
                    (free.first().copied().unwrap_or(127), None)
                } else {
                    let a = free.first().copied().unwrap_or(126);
                    let b = free.get(1).copied().unwrap_or(127);
                    (a, Some(b))
                }
            }
        };
        s.pts.insert(t.clone(), p);
        p
    }

    fn new_row(&self, s: &mut State, kind: Kind, class: Class, codec: &str) -> usize {
        let t = tuple_of(kind, codec);
        let (pt, rtx_pt) = self.pt_for(s, kind, &t, codec);
        let ssrc = Self::fresh_ssrc(s);
        let rtx_ssrc = if kind == Kind::Video { Some(Self::fresh_ssrc(s)) } else { None };
        let mid = self.start() + s.rows.len() as u16;
        let down = Arc::new(DownRow::create(self.conn.clone(), mid.to_string(), kind, class == Class::Slot, ssrc, rtx_ssrc, self.acked.clone()));
        down.pt.store(pt, Ordering::Release);
        down.rtx_pt.store(rtx_pt.unwrap_or(0), Ordering::Release);
        down.ext.store(self.ext.load_full());
        s.rows.push(Row {
            mid,
            kind,
            class,
            tuple: t,
            codec: codec.to_string(),
            pt,
            rtx_pt,
            ssrc,
            rtx_ssrc,
            track: None,
            since: 0,
            loaded_at: 0,
            state: SubState::Created,
            down,
        });
        s.rows.len() - 1
    }

    fn retune(&self, s: &mut State, i: usize, codec: &str) {
        let kind = s.rows[i].kind;
        let t = tuple_of(kind, codec);
        let (pt, rtx_pt) = self.pt_for(s, kind, &t, codec);
        let r = &mut s.rows[i];
        r.tuple = t;
        r.codec = codec.to_string();
        r.pt = pt;
        r.rtx_pt = rtx_pt;
        r.down.pt.store(pt, Ordering::Release);
        r.down.rtx_pt.store(rtx_pt.unwrap_or(0), Ordering::Release);
    }

    fn gate(s: &mut State, room: &str, now: u64) -> Arc<AtomicBool> {
        s.gates.entry(room.to_string()).or_insert_with(|| (Arc::new(AtomicBool::new(false)), now, false)).0.clone()
    }

    fn load(s: &mut State, i: usize, c: &Cand, rev: u64, now: u64, out: &mut Vec<Change>) {
        let gate = if c.class == Class::Full && c.kind == Kind::Video { Some(Self::gate(s, &c.room, now)) } else { None };
        let r = &mut s.rows[i];
        let old = r.track.replace((c.track_id.clone(), c.room.clone()));
        r.since = rev;
        r.loaded_at = now;
        r.state = SubState::Created;
        r.down.since.store(rev, Ordering::Release);
        r.down.room.store(Some(Arc::new(c.room.clone())));
        r.down.src.store(Some(Arc::new((c.track_id.clone(), c.room.clone()))));
        r.down.gate.store(gate);
        r.down.loaded_at.store(now, Ordering::Release);
        out.push(Change { down: r.down.clone(), class: r.class, old, new: Some((c.track_id.clone(), c.room.clone())) });
    }

    fn unload(s: &mut State, i: usize, rev: u64, out: &mut Vec<Change>) {
        let r = &mut s.rows[i];
        let Some(old) = r.track.take() else { return };
        r.since = rev;
        r.state = SubState::Created;
        r.down.since.store(rev, Ordering::Release);
        r.down.src.store(None);
        r.down.room.store(None);
        r.down.gate.store(None);
        out.push(Change { down: r.down.clone(), class: r.class, old: Some(old), new: None });
    }

    pub fn sync(&self, cands: &[Cand], now: u64) -> SyncOut {
        let mut out = SyncOut::default();
        let Ok(mut s) = self.st.lock() else { return out };
        let rev = s.arev + 1;
        let mut ch = Vec::new();
        let rows_before = s.rows.len();
        let mut tuned = false;
        let want_slot: BTreeSet<(String, Kind)> = cands.iter().filter(|c| c.class == Class::Slot).map(|c| (c.room.clone(), c.kind)).collect();
        for i in 0..s.rows.len() {
            let r = &s.rows[i];
            if r.class == Class::Slot && r.track.as_ref().map(|t| !want_slot.contains(&(t.1.clone(), r.kind))).unwrap_or(false) {
                Self::unload(&mut s, i, rev, &mut ch);
            }
        }
        let slots_needed: Vec<&Cand> = cands
            .iter()
            .filter(|c| c.class == Class::Slot)
            .filter(|c| !s.rows.iter().any(|r| r.class == Class::Slot && r.kind == c.kind && r.track.as_ref().map(|t| t.1 == c.room).unwrap_or(false)))
            .collect();
        for c in slots_needed {
            let t = tuple_of(c.kind, &c.codec);
            let free = (0..s.rows.len()).find(|&i| {
                let r = &s.rows[i];
                r.class == Class::Slot && r.kind == c.kind && r.track.is_none() && r.tuple == t
            });
            let i = match free {
                Some(i) => i,
                None => {
                    let slot_rows = s.rows.iter().filter(|r| r.class == Class::Slot).count();
                    if slot_rows >= self.slot_budget || s.rows.len() >= self.limit() {
                        out.mid_limit = true;
                        break;
                    }
                    self.new_row(&mut s, c.kind, Class::Slot, &c.codec)
                }
            };
            Self::load(&mut s, i, c, rev, now, &mut ch);
        }
        if out.mid_limit {
            drop(s);
            return out;
        }
        for kind in [Kind::Audio, Kind::Video] {
            let cap = if kind == Kind::Audio { FULL_AUDIO } else { FULL_VIDEO };
            let mut desired: Vec<&Cand> = cands.iter().filter(|c| c.class == Class::Full && c.kind == kind && !c.paused).collect();
            let loaded_at: HashMap<(String, String), u64> = s
                .rows
                .iter()
                .filter(|r| r.class == Class::Full && r.kind == kind)
                .filter_map(|r| r.track.clone().map(|t| (t, r.loaded_at)))
                .collect();
            for c in &desired {
                if !loaded_at.contains_key(&(c.track_id.clone(), c.room.clone())) {
                    s.waiting.entry(c.track_id.clone()).or_insert(now);
                }
            }
            desired.sort_by(|a, b| {
                let ka = (a.track_id.clone(), a.room.clone());
                let kb = (b.track_id.clone(), b.room.clone());
                let ta = loaded_at.get(&ka).map(|t| (0u8, u64::MAX - t)).unwrap_or((1, s.waiting.get(&a.track_id).copied().unwrap_or(now)));
                let tb = loaded_at.get(&kb).map(|t| (0u8, u64::MAX - t)).unwrap_or((1, s.waiting.get(&b.track_id).copied().unwrap_or(now)));
                b.rank.cmp(&a.rank).then(ta.cmp(&tb))
            });
            let target: Vec<&Cand> = desired.into_iter().take(cap).collect();
            let tset: BTreeSet<(String, String)> = target.iter().map(|c| (c.track_id.clone(), c.room.clone())).collect();
            for i in 0..s.rows.len() {
                let r = &s.rows[i];
                if r.class == Class::Full && r.kind == kind && r.track.as_ref().map(|t| !tset.contains(t)).unwrap_or(false) {
                    Self::unload(&mut s, i, rev, &mut ch);
                }
            }
            for c in target {
                let key = (c.track_id.clone(), c.room.clone());
                if s.rows.iter().any(|r| r.track.as_ref() == Some(&key) && r.class == Class::Full) {
                    s.waiting.remove(&c.track_id);
                    continue;
                }
                let t = tuple_of(kind, &c.codec);
                let free = (0..s.rows.len())
                    .filter(|&i| {
                        let r = &s.rows[i];
                        r.class == Class::Full && r.kind == kind && r.track.is_none() && r.tuple == t
                    })
                    .min_by_key(|&i| s.rows[i].mid);
                let full_rows = s.rows.iter().filter(|r| r.class == Class::Full && r.kind == kind).count();
                let i = match free {
                    Some(i) => Some(i),
                    None if full_rows < cap && s.rows.len() < self.limit() => Some(self.new_row(&mut s, kind, Class::Full, &c.codec)),
                    None => {
                        let other = (0..s.rows.len())
                            .filter(|&i| {
                                let r = &s.rows[i];
                                r.class == Class::Full && r.kind == kind && r.track.is_none()
                            })
                            .min_by_key(|&i| s.rows[i].loaded_at);
                        if let Some(i) = other {
                            self.retune(&mut s, i, &c.codec);
                            tuned = true;
                        }
                        other
                    }
                };
                if let Some(i) = i {
                    s.waiting.remove(&c.track_id);
                    Self::load(&mut s, i, c, rev, now, &mut ch);
                }
            }
        }
        let live: BTreeSet<String> = cands.iter().map(|c| c.track_id.clone()).collect();
        s.waiting.retain(|k, _| live.contains(k));
        if !ch.is_empty() || s.rows.len() != rows_before || tuned {
            s.arev = rev;
            out.changed = true;
        }
        out.changes = ch;
        out
    }

    pub fn ready(&self, room: &str, arev: u64, now: u64) -> (bool, Vec<(String, String)>) {
        let Ok(mut s) = self.st.lock() else { return (false, Vec::new()) };
        let first = !s.ready_at.contains_key(room);
        if first {
            s.ready_at.insert(room.to_string(), now);
        }
        if let Some(g) = s.gates.get(room) {
            g.0.store(true, Ordering::Release);
        }
        let ack = s.arev.min(arev);
        if ack > self.acked.load(Ordering::Acquire) {
            self.acked.store(ack, Ordering::Release);
        }
        let mut became = Vec::new();
        let acked = self.acked.load(Ordering::Acquire);
        for r in s.rows.iter_mut() {
            if r.state == SubState::Created && r.track.is_some() && r.since <= acked {
                r.state = SubState::Active;
                if let Some(t) = &r.track {
                    became.push(t.clone());
                }
            }
        }
        (first, became)
    }

    pub fn ready_at(&self, room: &str) -> Option<u64> {
        self.st.lock().ok().and_then(|s| s.ready_at.get(room).copied())
    }

    pub fn gate_timeouts(&self, now: u64) -> Vec<String> {
        let Ok(mut s) = self.st.lock() else { return Vec::new() };
        let mut out = Vec::new();
        for (room, (g, since, latched)) in s.gates.iter_mut() {
            if !g.load(Ordering::Acquire) && now.saturating_sub(*since) >= T_GATE_MS {
                g.store(true, Ordering::Release);
                if !*latched {
                    let _ = std::mem::replace(latched, true);
                    out.push(room.clone());
                }
            }
        }
        out
    }

    pub fn pref(&self, track: &str) -> (u8, bool) {
        self.st.lock().ok().and_then(|s| s.prefs.get(track).copied()).unwrap_or((0, false))
    }

    pub fn set_pref(&self, track: &str, priority: Option<u8>, paused: Option<bool>) -> bool {
        let Ok(mut s) = self.st.lock() else { return false };
        let e = s.prefs.entry(track.to_string()).or_insert((0, false));
        let before = *e;
        if let Some(p) = priority {
            e.0 = p;
        }
        if let Some(p) = paused {
            e.1 = p;
        }
        before != *e
    }

    pub fn row_of_track(&self, track: &str) -> Option<Arc<DownRow>> {
        self.st.lock().ok().and_then(|s| s.rows.iter().find(|r| r.track.as_ref().map(|t| t.0 == track).unwrap_or(false)).map(|r| r.down.clone()))
    }

    pub fn forget_room(&self, room: &str) {
        if let Ok(mut s) = self.st.lock() {
            s.gates.remove(room);
            s.ready_at.remove(room);
        }
    }
}

pub fn ext_table(m: &[ExtMap]) -> ExtTable {
    let id = |u: &str| m.iter().find(|e| e.uri == u).map(|e| e.id);
    ExtTable { mid: id(URI_MID), audio_level: id(URI_AUDIO_LEVEL), abs_send_time: id(URI_ABS_SEND_TIME), twcc: id(URI_TWCC) }
}

#[derive(Default)]
pub struct Tables {
    pub map: Mutex<HashMap<u64, Arc<Table>>>,
}

impl Tables {
    pub fn get(&self, conn: u64) -> Option<Arc<Table>> {
        self.map.lock().ok().and_then(|g| g.get(&conn).cloned())
    }

    pub fn insert(&self, t: Arc<Table>) {
        if let Ok(mut g) = self.map.lock() {
            g.insert(t.conn.id, t);
        }
    }

    pub fn remove(&self, conn: u64) -> Option<Arc<Table>> {
        self.map.lock().ok().and_then(|mut g| g.remove(&conn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Role;

    fn cand(id: &str, room: &str, kind: Kind, class: Class, rank: i64) -> Cand {
        Cand { track_id: id.into(), room: room.into(), kind, class, codec: if kind == Kind::Audio { "opus".into() } else { "H264".into() }, rank, paused: false }
    }

    #[test]
    fn slot_rows_and_reuse() {
        let tr = Transport::new(1, "u", Role::Both, 0);
        let t = Table::new(tr, PcMode::Two, 40);
        let c = vec![cand("ptt-a-audio", "a", Kind::Audio, Class::Slot, 0), cand("ptt-a-video", "a", Kind::Video, Class::Slot, 0)];
        let o = t.sync(&c, 1);
        assert!(o.changed && !o.mid_limit);
        assert_eq!(t.snapshot("e").mlines.len(), 2);
        assert_eq!(t.snapshot("e").mlines[0].pt, 111);
        let o = t.sync(&[], 2);
        assert_eq!(o.changes.len(), 2);
        let c2 = vec![cand("ptt-b-audio", "b", Kind::Audio, Class::Slot, 0), cand("ptt-b-video", "b", Kind::Video, Class::Slot, 0)];
        t.sync(&c2, 3);
        let snap = t.snapshot("e");
        assert_eq!(snap.mlines.len(), 2);
        assert_eq!(snap.mlines[0].room_id.as_deref(), Some("b"));
        assert_eq!(snap.arev, 3);
    }

    #[test]
    fn slot_budget_mid_limit() {
        let tr = Transport::new(1, "u", Role::Both, 0);
        let t = Table::new(tr, PcMode::One, 2);
        assert!(!t.sync(&[cand("ptt-a-audio", "a", Kind::Audio, Class::Slot, 0), cand("ptt-a-video", "a", Kind::Video, Class::Slot, 0)], 1).mid_limit);
        let more = vec![
            cand("ptt-a-audio", "a", Kind::Audio, Class::Slot, 0),
            cand("ptt-a-video", "a", Kind::Video, Class::Slot, 0),
            cand("ptt-b-audio", "b", Kind::Audio, Class::Slot, 0),
        ];
        assert!(t.sync(&more, 2).mid_limit);
        assert_eq!(t.snapshot("e").mlines[0].mid, "32");
    }

    #[test]
    fn full_cap_reassigns_by_rank() {
        let tr = Transport::new(1, "u", Role::Both, 0);
        let t = Table::new(tr, PcMode::Two, 40);
        let mut c: Vec<Cand> = (0..FULL_VIDEO).map(|i| cand(&format!("v{i}"), "r", Kind::Video, Class::Full, 0)).collect();
        t.sync(&c, 1);
        c.push(cand("hot", "r", Kind::Video, Class::Full, 5));
        let o = t.sync(&c, 2);
        assert_eq!(o.changes.len(), 2);
        let snap = t.snapshot("e");
        assert_eq!(snap.mlines.len(), FULL_VIDEO);
        assert!(snap.mlines.iter().any(|m| m.track_id.as_deref() == Some("hot")));
    }
}
