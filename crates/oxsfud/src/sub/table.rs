// author: kodeholic (powered by Claude)
// spec: v1.4 · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:IMPL-SUB-PIPELINE · 정:CAND-SUB-ROW-CLASS · 정:CAND-SUB-PAUSED-FREES-ROW · 정:CAND-SUB-REASSIGN-TICK · 연:SUB.type.MlineTable · 연:SUB.type.MlineEntry · 연:RULE-SUB-FILL · 연:RULE-SUB-EGRESS-ACKED · 연:RULE-SUB-ASSIGN-RECOVERED · 연:RULE-ROOM-BUDGET-SPLIT · 연:RULE-LINK-DOWNLINK-SSRC · 연:LINK.S.track_on · 연:LINK.S.track_off · 연:LINK.S.reissue · 연:LINK.S.reassign · 연:LINK.S.ack · model: claude-opus-5-5
//! 배정표(연결마다 하나) — 줄 하나 = 구독 m-line 하나. mid 는 최대 뒤로만 · 빈 줄은 같은 부류 · kind · 튜플 안에서 가장 작은 mid 부터 · 줄은 지우지 않는다 · 다운링크 SSRC 는 줄이 사는 동안 그대로 · 확인된 차례(ack) 뒤의 줄만 흐른다.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use oxsig::code::Code;
use oxsig::msg::RoomJoinReqCodecs;
use oxsig::types::{Kind, MlineEntry, MlineTable};

use crate::fwd::hot::Downlink;

pub const FULL_AUDIO: usize = 25;
pub const FULL_VIDEO: usize = 25;
pub const SUB_1PC: usize = 96;
pub const SUB_2PC: usize = 128;
pub const MID_START_1PC: u16 = 32;
pub const MID_START_2PC: u16 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Full,
    Slot,
}

impl Class {
    pub fn stream_id(self, mid: u8) -> String {
        match self {
            Class::Full => format!("ox-m{mid}"),
            Class::Slot => "ox-ptt".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tuple {
    pub kind: Kind,
    pub codec: String,
}

#[derive(Debug, Clone, Default)]
pub struct PtTable {
    map: BTreeMap<Tuple, (u8, Option<u8>)>,
    reserved: HashSet<u8>,
}

fn pt_ranges() -> impl Iterator<Item = u8> {
    (96u8..=127).chain(35u8..=63)
}

impl PtTable {
    pub fn seed_2pc() -> PtTable {
        let mut t = PtTable::default();
        t.map.insert(Tuple { kind: Kind::Audio, codec: "opus".into() }, (111, None));
        t.map.insert(Tuple { kind: Kind::Video, codec: "VP8".into() }, (96, Some(97)));
        t.map.insert(Tuple { kind: Kind::Video, codec: "H264".into() }, (102, Some(103)));
        t
    }

    pub fn seed_1pc(codecs: &[RoomJoinReqCodecs]) -> PtTable {
        let mut t = PtTable::default();
        for c in codecs {
            t.reserved.insert(c.pt);
            if let Some(r) = c.rtx_pt {
                t.reserved.insert(r);
            }
        }
        for c in codecs {
            let canon = match c.kind {
                Kind::Audio => crate::trk::canonical_audio(&c.name),
                Kind::Video => crate::trk::canonical_video(&c.name),
            };
            let Some(canon) = canon else { continue };
            if c.kind == Kind::Video && canon == "H264" && !crate::trk::h264_profile_matches(c.fmtp.as_deref(), crate::trk::FMTP_H264) {
                continue;
            }
            let key = Tuple { kind: c.kind, codec: canon.to_string() };
            t.map.entry(key).or_insert((c.pt, c.rtx_pt));
        }
        t
    }

    fn used(&self) -> HashSet<u8> {
        let mut u = self.reserved.clone();
        for (p, r) in self.map.values() {
            u.insert(*p);
            if let Some(r) = r {
                u.insert(*r);
            }
        }
        u
    }

    pub fn get(&mut self, tuple: &Tuple) -> Option<(u8, Option<u8>)> {
        if let Some(v) = self.map.get(tuple) {
            return Some(*v);
        }
        let used = self.used();
        let free: Vec<u8> = pt_ranges().filter(|p| !used.contains(p)).take(2).collect();
        let v = match tuple.kind {
            Kind::Audio => (*free.first()?, None),
            Kind::Video => (*free.first()?, free.get(1).copied()),
        };
        self.map.insert(tuple.clone(), v);
        Some(v)
    }
}

#[derive(Clone)]
pub struct Row {
    pub mid: u8,
    pub kind: Kind,
    pub class: Class,
    pub tuple: Tuple,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub track: Option<(String, String)>,
    pub since: u64,
    pub rank: u64,
    pub dl: Arc<Downlink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    pub track_id: String,
    pub room: String,
    pub kind: Kind,
    pub class: Class,
    pub tuple: Tuple,
    pub rank: u64,
    pub waiting_since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    On { mid: u8, track_id: String, room: String, label: &'static str },
    Off { mid: u8, track_id: String, room: String, label: &'static str },
    Wait { track_id: String, room: String },
}

pub struct Table {
    pub sfu_id: String,
    pub one_pc: bool,
    pub user: Arc<str>,
    rows: Vec<Row>,
    pub arev: u64,
    pub ack: u64,
    next_mid: u16,
    pub pts: PtTable,
    ssrcs: HashSet<u32>,
    waiting: Vec<Want>,
    slot_budget: usize,
    clock: u64,
}

fn random_ssrc(used: &HashSet<u32>) -> u32 {
    loop {
        let mut b = [0u8; 4];
        let _ = getrandom::fill(&mut b);
        let v = u32::from_be_bytes(b);
        if v > 1 && !used.contains(&v) {
            return v;
        }
    }
}

impl Table {
    pub fn new(sfu_id: &str, one_pc: bool, user: &str, pts: PtTable, slot_budget: usize, arev_from: u64) -> Table {
        Table {
            sfu_id: sfu_id.to_string(),
            one_pc,
            user: Arc::from(user),
            rows: Vec::new(),
            arev: arev_from,
            ack: arev_from,
            next_mid: if one_pc { MID_START_1PC } else { MID_START_2PC },
            pts,
            ssrcs: HashSet::new(),
            waiting: Vec::new(),
            slot_budget,
            clock: 0,
        }
    }

    pub fn reserve_ssrc(&mut self, s: u32) {
        self.ssrcs.insert(s);
    }

    pub fn downlink_ssrcs(&self) -> HashSet<u32> {
        let mut v = HashSet::new();
        for r in &self.rows {
            v.insert(r.ssrc);
            if let Some(x) = r.rtx_ssrc {
                v.insert(x);
            }
        }
        v
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn cap(&self) -> usize {
        if self.one_pc { SUB_1PC } else { SUB_2PC }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub fn wire(&self) -> MlineTable {
        let mut ml: Vec<MlineEntry> = self
            .rows
            .iter()
            .map(|r| MlineEntry {
                mid: r.mid.to_string(),
                kind: r.kind,
                pt: r.pt,
                codec: r.tuple.codec.clone(),
                fmtp: crate::trk::sub_fmtp(&r.tuple.codec).map(str::to_string),
                rtx_pt: r.rtx_pt,
                track_id: r.track.as_ref().map(|t| t.0.clone()),
                room_id: r.track.as_ref().map(|t| t.1.clone()),
                ssrc: r.ssrc,
                rtx_ssrc: r.rtx_ssrc,
                stream_id: r.class.stream_id(r.mid),
            })
            .collect();
        ml.sort_by_key(|e| e.mid.parse::<u16>().unwrap_or(0));
        MlineTable { sfu_id: self.sfu_id.clone(), arev: self.arev, mlines: ml }
    }

    fn slot_used(&self) -> usize {
        self.rows.iter().filter(|r| r.class == Class::Slot && r.track.is_some()).count()
    }

    fn full_rows(&self, kind: Kind) -> usize {
        self.rows.iter().filter(|r| r.class == Class::Full && r.kind == kind).count()
    }

    pub fn slot_room_fits(&self, kinds: usize) -> bool {
        let empty_slot = self.rows.iter().filter(|r| r.class == Class::Slot && r.track.is_none()).count();
        let need_new = kinds.saturating_sub(empty_slot);
        self.slot_used() + kinds <= self.slot_budget && self.rows.len() + need_new <= self.cap() && usize::from(self.next_mid) + need_new <= 256
    }

    fn new_row(&mut self, kind: Kind, class: Class, tuple: &Tuple, make: &dyn Fn(u8, Kind, bool, u32, u32, u8, u8) -> Arc<Downlink>) -> Option<usize> {
        if self.rows.len() >= self.cap() || self.next_mid > 255 {
            return None;
        }
        let (pt, rtx_pt) = self.pts.get(tuple)?;
        let mid = self.next_mid as u8;
        self.next_mid += 1;
        let ssrc = random_ssrc(&self.ssrcs);
        self.ssrcs.insert(ssrc);
        let rtx_ssrc = if kind == Kind::Video && rtx_pt.is_some() {
            let r = random_ssrc(&self.ssrcs);
            self.ssrcs.insert(r);
            Some(r)
        } else {
            None
        };
        let rtx_pt = if kind == Kind::Video { rtx_pt } else { None };
        let dl = make(mid, kind, class == Class::Slot, ssrc, rtx_ssrc.unwrap_or(0), pt, rtx_pt.unwrap_or(0));
        self.rows.push(Row { mid, kind, class, tuple: tuple.clone(), pt, rtx_pt, ssrc, rtx_ssrc, track: None, since: self.arev, rank: 0, dl });
        Some(self.rows.len() - 1)
    }

    fn empty_row(&self, kind: Kind, class: Class, tuple: &Tuple) -> Option<usize> {
        self.rows.iter().enumerate().filter(|(_, r)| r.track.is_none() && r.kind == kind && r.class == class && r.tuple == *tuple).min_by_key(|(_, r)| r.mid).map(|(i, _)| i)
    }

    fn put(&mut self, i: usize, track_id: &str, room: &str, rank: u64) {
        self.arev += 1;
        let r = &mut self.rows[i];
        r.track = Some((track_id.to_string(), room.to_string()));
        r.since = self.arev;
        r.rank = rank;
        r.dl.sendable.store(false, Ordering::Release);
    }

    pub fn find(&self, track_id: &str, room: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.track.as_ref().is_some_and(|t| t.0 == track_id && t.1 == room))
    }

    pub fn find_track(&self, track_id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.track.as_ref().is_some_and(|t| t.0 == track_id))
    }

    pub fn waiting(&self) -> &[Want] {
        &self.waiting
    }

    pub fn on(&mut self, w: Want, make: &dyn Fn(u8, Kind, bool, u32, u32, u8, u8) -> Arc<Downlink>) -> Result<Vec<Change>, Code> {
        if self.find(&w.track_id, &w.room).is_some() || self.waiting.iter().any(|x| x.track_id == w.track_id && x.room == w.room) {
            return Ok(Vec::new());
        }
        if let Some(i) = self.empty_row(w.kind, w.class, &w.tuple) {
            self.put(i, &w.track_id, &w.room, w.rank);
            return Ok(vec![Change::On { mid: self.rows[i].mid, track_id: w.track_id, room: w.room, label: "LINK.S.track_on" }]);
        }
        let under = match w.class {
            Class::Slot => self.slot_used() < self.slot_budget,
            Class::Full => self.full_rows(w.kind) < if w.kind == Kind::Audio { FULL_AUDIO } else { FULL_VIDEO },
        };
        if under && let Some(i) = self.new_row(w.kind, w.class, &w.tuple, make) {
            self.put(i, &w.track_id, &w.room, w.rank);
            return Ok(vec![Change::On { mid: self.rows[i].mid, track_id: w.track_id, room: w.room, label: "LINK.S.track_on" }]);
        }
        if w.class == Class::Slot {
            return Err(Code::MID_LIMIT);
        }
        let victim = self.rows.iter().enumerate().filter(|(_, r)| r.class == Class::Full && r.kind == w.kind && r.tuple == w.tuple && r.track.is_some()).min_by_key(|(_, r)| r.rank).map(|(i, r)| (i, r.rank));
        let t = self.tick();
        match victim {
            Some((i, rank)) if rank < w.rank => {
                let (old_t, old_r) = self.rows[i].track.clone().unwrap_or_default();
                self.waiting.push(Want { track_id: old_t.clone(), room: old_r.clone(), kind: w.kind, class: Class::Full, tuple: w.tuple.clone(), rank, waiting_since: t });
                self.put(i, &w.track_id, &w.room, w.rank);
                let mid = self.rows[i].mid;
                Ok(vec![Change::Off { mid, track_id: old_t, room: old_r, label: "LINK.S.reassign" }, Change::On { mid, track_id: w.track_id, room: w.room, label: "LINK.S.reassign" }])
            }
            _ => {
                let (track_id, room) = (w.track_id.clone(), w.room.clone());
                self.waiting.push(Want { waiting_since: t, ..w });
                Ok(vec![Change::Wait { track_id, room }])
            }
        }
    }

    pub fn off(&mut self, track_id: &str, room: &str) -> Vec<Change> {
        self.waiting.retain(|w| !(w.track_id == track_id && w.room == room));
        let Some(i) = self.rows.iter().position(|r| r.track.as_ref().is_some_and(|t| t.0 == track_id && t.1 == room)) else {
            return Vec::new();
        };
        self.arev += 1;
        let mid = self.rows[i].mid;
        self.rows[i].track = None;
        self.rows[i].since = self.arev;
        self.rows[i].dl.sendable.store(false, Ordering::Release);
        let mut out = vec![Change::Off { mid, track_id: track_id.to_string(), room: room.to_string(), label: "LINK.S.track_off" }];
        let (kind, class, tuple) = (self.rows[i].kind, self.rows[i].class, self.rows[i].tuple.clone());
        if let Some(wi) = self.waiting.iter().enumerate().filter(|(_, w)| w.kind == kind && w.class == class && w.tuple == tuple).max_by_key(|(_, w)| (w.rank, u64::MAX - w.waiting_since)).map(|(i, _)| i) {
            let w = self.waiting.remove(wi);
            self.put(i, &w.track_id, &w.room, w.rank);
            out.push(Change::On { mid, track_id: w.track_id, room: w.room, label: "LINK.S.reissue" });
        }
        out
    }

    pub fn off_room(&mut self, room: &str) -> Vec<Change> {
        let tracks: Vec<String> = self.rows.iter().filter_map(|r| r.track.as_ref().filter(|t| t.1 == room).map(|t| t.0.clone())).collect();
        self.waiting.retain(|w| w.room != room);
        let mut out = Vec::new();
        for t in tracks {
            out.extend(self.off(&t, room));
        }
        out
    }

    pub fn set_rank(&mut self, track_id: &str, room: &str, rank: u64) {
        for r in self.rows.iter_mut() {
            if r.track.as_ref().is_some_and(|t| t.0 == track_id && t.1 == room) {
                r.rank = rank;
            }
        }
        for w in self.waiting.iter_mut() {
            if w.track_id == track_id && w.room == room {
                w.rank = rank;
            }
        }
    }

    pub fn rehome(&mut self, track_id: &str, from: &str, to: &str) -> bool {
        let mut changed = false;
        for r in self.rows.iter_mut() {
            if let Some(t) = r.track.as_mut()
                && t.0 == track_id
                && t.1 == from
            {
                t.1 = to.to_string();
                changed = true;
            }
        }
        for w in self.waiting.iter_mut() {
            if w.track_id == track_id && w.room == from {
                w.room = to.to_string();
            }
        }
        if changed {
            self.arev += 1;
        }
        changed
    }

    pub fn ack(&mut self, a: u64) -> bool {
        if a <= self.ack || a > self.arev {
            return false;
        }
        self.ack = a;
        for r in &self.rows {
            r.dl.sendable.store(r.track.is_some() && r.since <= self.ack, Ordering::Release);
        }
        true
    }

    pub fn downlink_of(&self, mid: u8) -> Option<Arc<Downlink>> {
        self.rows.iter().find(|r| r.mid == mid).map(|r| r.dl.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sub::ext::ExtTable;

    fn mk() -> impl Fn(u8, Kind, bool, u32, u32, u8, u8) -> Arc<Downlink> {
        |mid, kind, slot, ssrc, rtx, pt, rpt| Arc::new(Downlink::new(mid, kind, slot, ssrc, rtx, pt, rpt, Arc::from("me"), std::sync::Weak::new(), ExtTable::default()))
    }

    fn want(t: &str, r: &str, kind: Kind, class: Class, rank: u64) -> Want {
        Want { track_id: t.into(), room: r.into(), kind, class, tuple: Tuple { kind, codec: if kind == Kind::Audio { "opus".into() } else { "H264".into() } }, rank, waiting_since: 0 }
    }

    #[test]
    fn rows_reuse_smallest_empty_and_keep_ssrc() {
        let mut t = Table::new("sfu", false, "me", PtTable::seed_2pc(), 40, 0);
        let m = mk();
        t.on(want("a", "r", Kind::Video, Class::Full, 1), &m).expect("a");
        t.on(want("b", "r", Kind::Video, Class::Full, 1), &m).expect("b");
        let ssrc0 = t.rows()[0].ssrc;
        t.off("a", "r");
        assert!(t.rows()[0].track.is_none());
        t.on(want("c", "r", Kind::Video, Class::Full, 1), &m).expect("c");
        assert_eq!(t.rows()[0].track.as_ref().map(|x| x.0.as_str()), Some("c"));
        assert_eq!(t.rows()[0].ssrc, ssrc0);
        assert_eq!(t.rows().len(), 2);
        assert_eq!(t.rows()[0].pt, 102);
        assert_eq!(t.rows()[0].rtx_pt, Some(103));
        let w = t.wire();
        assert_eq!(w.mlines[0].stream_id, "ox-m0");
        assert_eq!(w.arev, 4);
    }

    #[test]
    fn egress_only_after_ack() {
        let mut t = Table::new("sfu", false, "me", PtTable::seed_2pc(), 40, 0);
        let m = mk();
        t.on(want("a", "r", Kind::Audio, Class::Full, 1), &m).expect("a");
        assert!(!t.rows()[0].dl.sendable.load(Ordering::Acquire));
        assert!(t.ack(1));
        assert!(t.rows()[0].dl.sendable.load(Ordering::Acquire));
        assert!(!t.ack(1));
        assert!(!t.ack(9));
    }

    #[test]
    fn slot_budget_is_mid_limit() {
        let mut t = Table::new("sfu", true, "me", PtTable::seed_2pc(), 2, 0);
        let m = mk();
        t.on(want("s1", "r1", Kind::Audio, Class::Slot, 0), &m).expect("s1");
        t.on(want("s2", "r1", Kind::Video, Class::Slot, 0), &m).expect("s2");
        assert_eq!(t.on(want("s3", "r2", Kind::Audio, Class::Slot, 0), &m), Err(Code::MID_LIMIT));
        assert_eq!(t.rows()[0].mid, 32);
        assert!(!t.slot_room_fits(2));
    }

    #[test]
    fn full_cap_reassigns_lowest_rank() {
        let mut t = Table::new("sfu", false, "me", PtTable::seed_2pc(), 40, 0);
        let m = mk();
        for i in 0..FULL_VIDEO {
            t.on(want(&format!("v{i}"), "r", Kind::Video, Class::Full, 10 + i as u64), &m).expect("v");
        }
        let ch = t.on(want("hi", "r", Kind::Video, Class::Full, 100), &m).expect("hi");
        assert!(matches!(&ch[1], Change::On { label: "LINK.S.reassign", .. }));
        assert_eq!(t.waiting()[0].track_id, "v0");
        let ch = t.off("hi", "r");
        assert!(matches!(&ch[1], Change::On { label: "LINK.S.reissue", .. }));
    }
}
