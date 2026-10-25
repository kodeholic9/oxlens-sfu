// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 정:IMPL-SUB-TABLE 정:IMPL-SUB-SUBSCRIBER-TABLE 정:CAND-SUB-ROW-CLASS 정:CAND-SUB-PAUSED-FREES-ROW 정:CAND-SUB-REASSIGN-TICK 연:SUB.type.MlineTable 연:SUB.type.MlineEntry 연:RULE-SUB-FILL 연:RULE-SUB-TABLE-SERVER 연:RULE-SUB-REMOVE-KEEPS-MID 연:RULE-SUB-ASSIGN-RECOVERED 연:RULE-LINK-DOWNLINK-SSRC 연:RULE-LINK-EGRESS-REWRITE 연:RULE-ROOM-BUDGET-SPLIT 연:RULE-ROOM-JOIN-NUMBER-TAGS 연:LINK.S.track_on 연:LINK.S.track_off 연:LINK.S.reissue 연:LINK.S.reassign · model: claude-opus-5-5
//! 배정표 — 연결마다 하나 · 줄은 지우지 않는다 · 줄의 튜플 · 다운링크 SSRC · `stream_id` 는 줄이 생길 때 정해진다 · 바뀔 때마다 `arev` 하나 · 확인 전 그 줄로 안 보낸다.

use std::collections::{BTreeSet, HashSet};

use oxsig::msg::{ExtMap, Kind, MlineEntry, MlineTable, RoomJoinReqCodecs};

use super::ext::ExtTable;
use crate::trk::codec;

pub const FULL_AUDIO: usize = 25;
pub const FULL_VIDEO: usize = 25;
pub const MID_START_1PC: u16 = 32;
pub const MID_START_2PC: u16 = 0;
pub const MID_MAX: u16 = 255;

pub const URI_MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
pub const URI_AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
pub const URI_ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
pub const URI_TWCC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
pub const URI_RID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
pub const URI_RRID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";

pub fn server_extmap() -> Vec<ExtMap> {
    [(1, URI_MID), (4, URI_AUDIO_LEVEL), (5, URI_ABS_SEND_TIME), (6, URI_TWCC), (10, URI_RID), (11, URI_RRID)]
        .iter()
        .map(|(id, uri)| ExtMap { id: *id, uri: (*uri).to_string() })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubExt {
    pub audio: ExtTable,
    pub video: ExtTable,
}

impl SubExt {
    pub fn of(kind: Kind, e: &SubExt) -> ExtTable {
        match kind {
            Kind::Audio => e.audio,
            Kind::Video => e.video,
        }
    }

    pub fn build(client: Option<&[ExtMap]>) -> SubExt {
        let server = server_extmap();
        let id = |uri: &str| -> Option<u8> {
            client.and_then(|c| c.iter().find(|e| e.uri == uri).map(|e| e.id)).or_else(|| server.iter().find(|e| e.uri == uri).map(|e| e.id))
        };
        let audio = ExtTable { mid: id(URI_MID), audio_level: id(URI_AUDIO_LEVEL), abs_send_time: id(URI_ABS_SEND_TIME), twcc: id(URI_TWCC) };
        let video = ExtTable { audio_level: None, ..audio };
        SubExt { audio, video }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Full,
    Slot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Load {
    pub room: String,
    pub track_id: String,
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub mid: u8,
    pub kind: Kind,
    pub class: Class,
    pub codec: String,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub load: Option<Load>,
    pub since: u64,
    pub at_ms: u64,
    pub score: u64,
}

impl Row {
    pub fn stream_id(&self) -> String {
        match self.class {
            Class::Full => format!("ox-m{}", self.mid),
            Class::Slot => "ox-ptt".to_string(),
        }
    }

    pub fn entry(&self) -> MlineEntry {
        MlineEntry {
            mid: self.mid.to_string(),
            kind: self.kind,
            pt: self.pt,
            codec: self.codec.clone(),
            fmtp: codec::row_fmtp(&self.codec).map(str::to_string),
            rtx_pt: self.rtx_pt,
            track_id: self.load.as_ref().map(|l| l.track_id.clone()),
            room_id: self.load.as_ref().map(|l| l.room.clone()),
            ssrc: self.ssrc,
            rtx_ssrc: self.rtx_ssrc,
            stream_id: self.stream_id(),
        }
    }

    fn holds(&self, room: &str, track_id: &str) -> bool {
        self.load.as_ref().is_some_and(|l| l.room == room && l.track_id == track_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    pub room: String,
    pub track_id: String,
    pub user: String,
    pub kind: Kind,
    pub codec: String,
    pub score: u64,
    pub since_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidLimit {
    pub limit: usize,
    pub current: usize,
}

pub struct Table {
    pub sfu_id: String,
    pub arev: u64,
    pub rows: Vec<Row>,
    pub waiting: Vec<Want>,
    pub ext: SubExt,
    pub one_pc: bool,
    next_mid: u16,
    pts: Vec<(String, u8, Option<u8>)>,
    declared: BTreeSet<u8>,
    declared_tuples: Vec<(String, u8, Option<u8>)>,
    ssrcs: HashSet<u32>,
    pub avoid: HashSet<u32>,
    slot_budget: usize,
    dirty: BTreeSet<usize>,
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    u32::from_be_bytes(b)
}

impl Table {
    pub fn new(sfu_id: &str, one_pc: bool, slot_budget: u16, ext: SubExt) -> Table {
        Table {
            sfu_id: sfu_id.to_string(),
            arev: 0,
            rows: Vec::new(),
            waiting: Vec::new(),
            ext,
            one_pc,
            next_mid: if one_pc { MID_START_1PC } else { MID_START_2PC },
            pts: Vec::new(),
            declared: BTreeSet::new(),
            declared_tuples: Vec::new(),
            ssrcs: HashSet::new(),
            avoid: HashSet::new(),
            slot_budget: slot_budget as usize,
            dirty: BTreeSet::new(),
        }
    }

    pub fn seed(&mut self, codecs: &[RoomJoinReqCodecs]) {
        for c in codecs {
            self.declared.insert(c.pt);
            if let Some(r) = c.rtx_pt {
                self.declared.insert(r);
            }
            if let Some(k) = codec::tuple_key(c.kind, &c.name, c.fmtp.as_deref()) {
                self.declared_tuples.push((k, c.pt, c.rtx_pt));
            }
        }
    }

    fn used(&self, pt: u8) -> bool {
        self.declared.contains(&pt) || self.pts.iter().any(|(_, a, b)| *a == pt || *b == Some(pt))
    }

    fn free_pt(&self) -> Option<u8> {
        (96u8..=127).chain(35u8..=63).find(|p| !self.used(*p))
    }

    fn free_pair(&self) -> Option<(u8, u8)> {
        let pool: Vec<u8> = (96u8..=127).chain(35u8..=63).collect();
        let mut first = None;
        for p in &pool {
            if self.used(*p) {
                continue;
            }
            match first {
                None => first = Some(*p),
                Some(f) => return Some((f, *p)),
            }
        }
        None
    }

    pub fn pt_for(&mut self, kind: Kind, name: &str) -> (u8, Option<u8>) {
        let fmtp = codec::row_fmtp(name);
        let key = codec::tuple_key(kind, name, fmtp).unwrap_or_else(|| name.to_string());
        if let Some((_, p, r)) = self.pts.iter().find(|(k, _, _)| *k == key) {
            return (*p, *r);
        }
        let video = kind == Kind::Video;
        let chosen = if let Some((_, p, r)) = self.declared_tuples.iter().find(|(k, _, _)| *k == key) {
            let rtx = if video { r.or_else(|| self.free_pt()) } else { None };
            (*p, rtx)
        } else if !self.one_pc {
            let seed = match codec::canon(name) {
                Some("opus") => Some((111, None)),
                Some("VP8") => Some((96, Some(97))),
                Some("H264") => Some((102, Some(103))),
                _ => None,
            };
            match seed.filter(|(p, r)| !self.used(*p) && r.is_none_or(|x| !self.used(x))) {
                Some(s) => s,
                None => match self.free_pair() {
                    Some((a, b)) => (a, video.then_some(b)),
                    None => (self.free_pt().unwrap_or(127), None),
                },
            }
        } else {
            match self.free_pair() {
                Some((a, b)) => (a, video.then_some(b)),
                None => (self.free_pt().unwrap_or(127), None),
            }
        };
        self.pts.push((key, chosen.0, chosen.1));
        chosen
    }

    fn new_ssrc(&mut self) -> u32 {
        loop {
            let s = rand_u32();
            if s > 1 && !self.ssrcs.contains(&s) && !self.avoid.contains(&s) {
                self.ssrcs.insert(s);
                return s;
            }
        }
    }

    pub fn downlink_ssrcs(&self) -> HashSet<u32> {
        self.ssrcs.clone()
    }

    fn new_row(&mut self, kind: Kind, class: Class, name: &str, now: u64) -> Option<usize> {
        if self.next_mid > MID_MAX {
            return None;
        }
        let mid = self.next_mid as u8;
        self.next_mid += 1;
        let (pt, rtx_pt) = self.pt_for(kind, name);
        let ssrc = self.new_ssrc();
        let rtx_ssrc = rtx_pt.map(|_| self.new_ssrc());
        self.rows.push(Row { mid, kind, class, codec: name.to_string(), pt, rtx_pt, ssrc, rtx_ssrc, load: None, since: 0, at_ms: now, score: 0 });
        let i = self.rows.len() - 1;
        self.dirty.insert(i);
        Some(i)
    }

    fn empty_row(&self, kind: Kind, class: Class, name: &str) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == kind && r.class == class && r.codec == name && r.load.is_none())
            .min_by_key(|(_, r)| r.mid)
            .map(|(i, _)| i)
    }

    fn put(&mut self, i: usize, load: Load, score: u64, now: u64) {
        let r = &mut self.rows[i];
        r.load = Some(load);
        r.score = score;
        r.at_ms = now;
        self.dirty.insert(i);
    }

    fn clear(&mut self, i: usize, now: u64) {
        let r = &mut self.rows[i];
        r.load = None;
        r.score = 0;
        r.at_ms = now;
        self.dirty.insert(i);
    }

    pub fn slot_occupied(&self) -> usize {
        self.rows.iter().filter(|r| r.class == Class::Slot && r.load.is_some()).count()
    }

    pub fn add_slots(&mut self, room: &str, audio: &str, video: &str, now: u64) -> Result<(), MidLimit> {
        if self.rows.iter().any(|r| r.class == Class::Slot && r.load.as_ref().is_some_and(|l| l.room == room)) {
            return Ok(());
        }
        let cur = self.slot_occupied();
        if cur + 2 > self.slot_budget {
            return Err(MidLimit { limit: self.slot_budget, current: cur });
        }
        for (kind, name) in [(Kind::Audio, audio), (Kind::Video, video)] {
            let i = match self.empty_row(kind, Class::Slot, name) {
                Some(i) => i,
                None => self.new_row(kind, Class::Slot, name, now).ok_or(MidLimit { limit: self.slot_budget, current: cur })?,
            };
            let load = Load { room: room.to_string(), track_id: crate::trk::slot_id(room, kind), user: None };
            self.put(i, load, 0, now);
        }
        Ok(())
    }

    fn cap(kind: Kind) -> usize {
        match kind {
            Kind::Audio => FULL_AUDIO,
            Kind::Video => FULL_VIDEO,
        }
    }

    fn full_count(&self, kind: Kind) -> usize {
        self.rows.iter().filter(|r| r.kind == kind && r.class == Class::Full).count()
    }

    pub fn holds(&self, room: &str, track_id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.holds(room, track_id))
    }

    pub fn want(&mut self, w: Want, now: u64) {
        if self.rows.iter().any(|r| r.holds(&w.room, &w.track_id)) {
            return;
        }
        self.waiting.retain(|x| !(x.room == w.room && x.track_id == w.track_id));
        let load = Load { room: w.room.clone(), track_id: w.track_id.clone(), user: Some(w.user.clone()) };
        if let Some(i) = self.empty_row(w.kind, Class::Full, &w.codec) {
            self.put(i, load, w.score, now);
            return;
        }
        if self.full_count(w.kind) < Self::cap(w.kind)
            && let Some(i) = self.new_row(w.kind, Class::Full, &w.codec, now)
        {
            self.put(i, load, w.score, now);
            return;
        }
        let same = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == w.kind && r.class == Class::Full && r.codec == w.codec)
            .min_by_key(|(_, r)| (r.score, r.at_ms))
            .map(|(i, _)| i);
        match same {
            Some(i) if self.rows[i].score < w.score => {
                if let Some(old) = self.rows[i].load.clone() {
                    let kind = self.rows[i].kind;
                    let codec = self.rows[i].codec.clone();
                    self.waiting.push(Want {
                        room: old.room,
                        track_id: old.track_id,
                        user: old.user.unwrap_or_default(),
                        kind,
                        codec,
                        score: self.rows[i].score,
                        since_ms: now,
                    });
                }
                self.put(i, load, w.score, now);
            }
            Some(_) => self.waiting.push(w),
            None => {
                let lru = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.kind == w.kind && r.class == Class::Full)
                    .min_by_key(|(_, r)| (r.load.is_some(), r.score, r.at_ms))
                    .map(|(i, _)| i);
                match lru {
                    Some(i) if self.rows[i].load.is_none() || self.rows[i].score < w.score => {
                        let (pt, rtx) = self.pt_for(w.kind, &w.codec);
                        if let Some(old) = self.rows[i].load.clone() {
                            let kind = self.rows[i].kind;
                            let codec = self.rows[i].codec.clone();
                            let score = self.rows[i].score;
                            self.waiting.push(Want {
                                room: old.room,
                                track_id: old.track_id,
                                user: old.user.unwrap_or_default(),
                                kind,
                                codec,
                                score,
                                since_ms: now,
                            });
                        }
                        let r = &mut self.rows[i];
                        r.codec = w.codec.clone();
                        r.pt = pt;
                        r.rtx_pt = rtx;
                        self.put(i, load, w.score, now);
                    }
                    _ => self.waiting.push(w),
                }
            }
        }
    }

    fn refill(&mut self, i: usize, now: u64) {
        let (kind, name) = (self.rows[i].kind, self.rows[i].codec.clone());
        if self.rows[i].class != Class::Full {
            return;
        }
        let pick = self.waiting.iter().enumerate().filter(|(_, w)| w.kind == kind && w.codec == name).min_by_key(|(_, w)| w.since_ms).map(|(j, _)| j);
        if let Some(j) = pick {
            let w = self.waiting.remove(j);
            let load = Load { room: w.room, track_id: w.track_id, user: Some(w.user) };
            self.put(i, load, w.score, now);
        }
    }

    pub fn unwant(&mut self, room: &str, track_id: &str, now: u64) {
        self.waiting.retain(|w| !(w.room == room && w.track_id == track_id));
        let hit: Vec<usize> = self.rows.iter().enumerate().filter(|(_, r)| r.holds(room, track_id)).map(|(i, _)| i).collect();
        for i in hit {
            self.clear(i, now);
            self.refill(i, now);
        }
    }

    pub fn drop_room(&mut self, room: &str, now: u64) {
        self.waiting.retain(|w| w.room != room);
        let hit: Vec<usize> = self.rows.iter().enumerate().filter(|(_, r)| r.load.as_ref().is_some_and(|l| l.room == room)).map(|(i, _)| i).collect();
        for i in hit {
            self.clear(i, now);
            self.refill(i, now);
        }
    }

    pub fn move_room(&mut self, track_id: &str, from: &str, to: &str) {
        for (i, r) in self.rows.iter_mut().enumerate() {
            if let Some(l) = r.load.as_mut()
                && l.track_id == track_id
                && l.room == from
            {
                l.room = to.to_string();
                self.dirty.insert(i);
            }
        }
        for w in self.waiting.iter_mut() {
            if w.track_id == track_id && w.room == from {
                w.room = to.to_string();
            }
        }
    }

    pub fn set_score(&mut self, room: &str, track_id: &str, score: u64) {
        for r in self.rows.iter_mut() {
            if r.holds(room, track_id) {
                r.score = score;
            }
        }
        for w in self.waiting.iter_mut() {
            if w.room == room && w.track_id == track_id {
                w.score = score;
            }
        }
    }

    pub fn finish(&mut self) -> Option<u64> {
        if self.dirty.is_empty() {
            return None;
        }
        self.arev += 1;
        let a = self.arev;
        for i in std::mem::take(&mut self.dirty) {
            if let Some(r) = self.rows.get_mut(i) {
                r.since = a;
            }
        }
        Some(a)
    }

    pub fn discard(&mut self) {
        self.dirty.clear();
    }

    pub fn wire(&self) -> MlineTable {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|r| r.mid);
        MlineTable { sfu_id: self.sfu_id.clone(), arev: self.arev, mlines: rows.iter().map(|r| r.entry()).collect() }
    }

    pub fn snapshot(&self) -> (Vec<Row>, u64) {
        (self.rows.clone(), self.arev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t2() -> Table {
        Table::new("e", false, 40, SubExt::build(None))
    }

    fn w(room: &str, id: &str, kind: Kind, codec: &str, score: u64) -> Want {
        Want { room: room.into(), track_id: id.into(), user: "u".into(), kind, codec: codec.into(), score, since_ms: 0 }
    }

    #[test]
    fn seeds_2pc() {
        let mut t = t2();
        assert_eq!(t.pt_for(Kind::Audio, "opus"), (111, None));
        assert_eq!(t.pt_for(Kind::Video, "VP8"), (96, Some(97)));
        assert_eq!(t.pt_for(Kind::Video, "H264"), (102, Some(103)));
        assert_eq!(t.pt_for(Kind::Video, "VP8"), (96, Some(97)), "같은 튜플은 늘 같은 PT");
    }

    #[test]
    fn seeds_1pc_avoid_declared() {
        let mut t = Table::new("e", true, 40, SubExt::build(None));
        t.seed(&[
            RoomJoinReqCodecs { kind: Kind::Audio, pt: 111, name: "opus".into(), fmtp: Some("minptime=10;useinbandfec=1".into()), rtx_pt: None },
            RoomJoinReqCodecs { kind: Kind::Video, pt: 96, name: "VP8".into(), fmtp: None, rtx_pt: Some(97) },
            RoomJoinReqCodecs { kind: Kind::Video, pt: 98, name: "VP9".into(), fmtp: None, rtx_pt: Some(99) },
            RoomJoinReqCodecs {
                kind: Kind::Video,
                pt: 109,
                name: "H264".into(),
                fmtp: Some("packetization-mode=1;profile-level-id=42e01f".into()),
                rtx_pt: Some(114),
            },
        ]);
        assert_eq!(t.pt_for(Kind::Audio, "opus"), (111, None));
        assert_eq!(t.pt_for(Kind::Video, "H264"), (109, Some(114)), "같은 튜플은 그 PT 재사용");
        assert_eq!(t.next_mid, MID_START_1PC);
    }

    #[test]
    fn slots_are_atomic_and_budgeted() {
        let mut t = Table::new("e", false, 4, SubExt::build(None));
        assert!(t.add_slots("r1", "opus", "H264", 0).is_ok());
        assert!(t.add_slots("r2", "opus", "H264", 0).is_ok());
        assert_eq!(t.add_slots("r3", "opus", "H264", 0), Err(MidLimit { limit: 4, current: 4 }));
        t.drop_room("r1", 0);
        assert!(t.add_slots("r3", "opus", "H264", 0).is_ok());
        assert_eq!(t.rows.len(), 4, "빈 슬롯 줄을 먼저 쓴다");
        assert!(t.rows.iter().all(|r| r.stream_id() == "ox-ptt"));
    }

    #[test]
    fn empty_row_reused_by_same_kind_then_new_then_reassign() {
        let mut t = t2();
        t.want(w("r", "a", Kind::Video, "H264", 0), 0);
        assert_eq!(t.finish(), Some(1));
        t.unwant("r", "a", 1);
        assert_eq!(t.finish(), Some(2));
        t.want(w("r", "b", Kind::Video, "H264", 0), 2);
        t.finish();
        assert_eq!(t.rows.len(), 1, "S-sym-IMPL-SUB-TABLE-1 같은 kind 의 빈 줄 먼저");
        for i in 0..30 {
            t.want(w("r", &format!("v{i}"), Kind::Video, "H264", 0), 3);
        }
        assert_eq!(t.full_count(Kind::Video), FULL_VIDEO);
        assert!(!t.waiting.is_empty());
        t.want(w("r", "vip", Kind::Video, "H264", 9), 4);
        assert!(t.holds("r", "vip").is_some(), "높은 priority 는 낮은 것을 밀어낸다");
        t.unwant("r", "b", 5);
        let refilled = t.rows.iter().filter(|r| r.load.is_some()).count();
        assert_eq!(refilled, FULL_VIDEO, "줄이 비면 오래 기다린 것부터 다시 싣는다");
    }

    #[test]
    fn row_keeps_ssrc_and_stream_id() {
        let mut t = t2();
        t.want(w("r", "a", Kind::Audio, "opus", 0), 0);
        let s0 = t.rows[0].ssrc;
        t.unwant("r", "a", 1);
        t.want(w("r", "b", Kind::Audio, "opus", 0), 2);
        assert_eq!(t.rows[0].ssrc, s0, "연:RULE-LINK-DOWNLINK-SSRC");
        assert_eq!(t.rows[0].stream_id(), "ox-m0");
        assert!(t.rows[0].ssrc > 1);
    }

    #[test]
    fn move_room_changes_only_room() {
        let mut t = t2();
        t.want(w("r1", "a", Kind::Video, "H264", 0), 0);
        t.finish();
        let mid = t.rows[0].mid;
        t.move_room("a", "r1", "r3");
        assert_eq!(t.finish(), Some(2));
        assert_eq!(t.rows[0].mid, mid);
        assert_eq!(t.rows[0].load.as_ref().map(|l| l.room.as_str()), Some("r3"));
    }
}
