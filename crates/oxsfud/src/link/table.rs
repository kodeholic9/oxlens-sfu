// author: kodeholic (powered by Claude)
// spec: v1.2 @8a0d8cf · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-SUBSCRIBER-TABLE · 연:SUB.type.MlineTable · 연:SUB.type.MlineEntry · 연:RULE-SUB-FILL · 연:RULE-SUB-TABLE-SERVER · 연:RULE-SUB-EGRESS-ACKED · 연:RULE-SUB-ASSIGN-RECOVERED · 연:RULE-ROOM-BUDGET-SPLIT · 연:RULE-LINK-EGRESS-REWRITE · 연:RULE-LINK-DOWNLINK-SSRC · 정:RULE-CONC-SANS-IO · model: claude-opus-5-5
//! 배정표(sans-IO) — 연결마다 하나: 줄(mid · 튜플 · 다운링크 SSRC · 실린 트랙) · 채우는 순서 셋(빈 줄 · 새 줄 · 재배정) · PT 씨앗 · 확인된 차례.

use std::collections::{BTreeMap, BTreeSet};

use oxsig::room::JoinCodec;
use oxsig::ses::PcMode;
use oxsig::sub::{MlineEntry, MlineTable};
use oxsig::trk::MediaKind;

pub const FULL_AUDIO_CAP: usize = 25;
pub const FULL_VIDEO_CAP: usize = 25;
pub const SUB_BUDGET_1PC: usize = 96;
pub const SUB_BUDGET_2PC: usize = 128;
pub const FMTP_H264: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
pub const FMTP_OPUS: &str = "minptime=10;useinbandfec=1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Full,
    Slot,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tuple {
    pub kind: MediaKind,
    pub name: String,
    pub params: String,
}

fn fmtp_param<'a>(fmtp: &'a str, key: &str) -> Option<&'a str> {
    fmtp.split(';').map(str::trim).find_map(|kv| kv.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

impl Tuple {
    pub fn of(kind: MediaKind, name: &str, fmtp: Option<&str>) -> Tuple {
        let lname = name.to_ascii_lowercase();
        let f = fmtp.unwrap_or("");
        let params = match lname.as_str() {
            "h264" => {
                let pm = fmtp_param(f, "packetization-mode").unwrap_or("0");
                let prof = fmtp_param(f, "profile-level-id").map(|p| p.get(..4).unwrap_or(p).to_ascii_lowercase()).unwrap_or_default();
                format!("pm={pm};prof={prof}")
            }
            "opus" => fmtp_param(f, "sprop-stereo").map(|s| format!("sprop-stereo={s}")).unwrap_or_default(),
            "rtx" => fmtp_param(f, "apt").map(|s| format!("apt={s}")).unwrap_or_default(),
            _ => String::new(),
        };
        Tuple { kind, name: lname, params }
    }

    pub fn room_codec(kind: MediaKind, name: &str) -> (Tuple, Option<String>) {
        let fmtp = match name.to_ascii_lowercase().as_str() {
            "h264" => Some(FMTP_H264.to_string()),
            "opus" => Some(FMTP_OPUS.to_string()),
            _ => None,
        };
        (Tuple::of(kind, name, fmtp.as_deref()), fmtp)
    }

    fn seed(&self) -> Option<(u8, Option<u8>)> {
        match (self.kind, self.name.as_str()) {
            (MediaKind::Audio, "opus") => Some((111, None)),
            (MediaKind::Video, "vp8") => Some((96, Some(97))),
            (MediaKind::Video, "h264") => Some((102, Some(103))),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pts {
    pub map: BTreeMap<Tuple, (u8, Option<u8>)>,
    pub used: BTreeSet<u8>,
    pub ignored_tags: u32,
}

fn free_pt(used: &BTreeSet<u8>) -> Option<u8> {
    (96..=127u8).chain(35..=63u8).find(|p| !used.contains(p))
}

impl Pts {
    pub fn seed_tags(&mut self, codecs: &[JoinCodec]) {
        for c in codecs {
            self.used.insert(c.pt);
            if let Some(r) = c.rtx_pt {
                self.used.insert(r);
            }
        }
        for c in codecs {
            if c.name.eq_ignore_ascii_case("rtx") {
                continue;
            }
            let t = Tuple::of(c.kind, &c.name, c.fmtp.as_deref());
            self.map.entry(t).or_insert((c.pt, c.rtx_pt));
        }
    }

    pub fn of(&mut self, t: &Tuple) -> Option<(u8, Option<u8>)> {
        if let Some(v) = self.map.get(t) {
            return Some(*v);
        }
        let want_rtx = t.kind == MediaKind::Video;
        let pick = match t.seed() {
            Some((p, r)) if !self.used.contains(&p) && r.map(|r| !self.used.contains(&r)).unwrap_or(true) => (p, r.filter(|_| want_rtx)),
            _ => {
                let p = free_pt(&self.used)?;
                self.used.insert(p);
                let r = if want_rtx { free_pt(&self.used) } else { None };
                self.used.remove(&p);
                (p, r)
            }
        };
        self.used.insert(pick.0);
        if let Some(r) = pick.1 {
            self.used.insert(r);
        }
        self.map.insert(t.clone(), pick);
        Some(pick)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub mid: u16,
    pub kind: MediaKind,
    pub class: Class,
    pub tuple: Tuple,
    pub codec: String,
    pub fmtp: Option<String>,
    pub pt: u8,
    pub rtx_pt: Option<u8>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub track: Option<(String, String)>,
    pub since: u64,
    pub since_ms: u64,
    pub used_at: u64,
}

impl Row {
    pub fn entry(&self) -> MlineEntry {
        MlineEntry {
            mid: self.mid.to_string(),
            kind: self.kind,
            pt: self.pt,
            codec: self.codec.clone(),
            fmtp: self.fmtp.clone(),
            rtx_pt: self.rtx_pt,
            track_id: self.track.as_ref().map(|t| t.0.clone()),
            room_id: self.track.as_ref().map(|t| t.1.clone()),
            ssrc: self.ssrc,
            rtx_ssrc: self.rtx_ssrc,
            stream_id: match self.class {
                Class::Full => format!("ox-m{}", self.mid),
                Class::Slot => "ox-ptt".to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub track_id: String,
    pub room_id: String,
    pub kind: MediaKind,
    pub class: Class,
    pub codec: String,
    pub rank: u64,
    pub since_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Load { mid: u16, track_id: String, room_id: String },
    Unload { mid: u16, track_id: String, room_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fill {
    Loaded(u16),
    Waiting,
    Over,
    NoPt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub mode: PcMode,
    pub arev: u64,
    pub ack: u64,
    pub rows: Vec<Row>,
    pub next_mid: u16,
    pub pts: Pts,
    pub ssrcs: BTreeSet<u32>,
    pub uplink: BTreeSet<u32>,
    pub waiting: Vec<Want>,
    pub slot_reserve: usize,
}

impl Table {
    pub fn new(mode: PcMode, slot_reserve: usize) -> Table {
        Table {
            mode,
            arev: 0,
            ack: 0,
            rows: Vec::new(),
            next_mid: if mode == PcMode::One { 32 } else { 0 },
            pts: Pts::default(),
            ssrcs: BTreeSet::new(),
            uplink: BTreeSet::new(),
            waiting: Vec::new(),
            slot_reserve,
        }
    }

    pub fn budget(&self) -> usize {
        if self.mode == PcMode::One { SUB_BUDGET_1PC } else { SUB_BUDGET_2PC }
    }

    pub fn wire(&self, sfu_id: &str) -> MlineTable {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|r| r.mid);
        MlineTable { sfu_id: sfu_id.to_string(), arev: self.arev, mlines: rows.iter().map(|r| r.entry()).collect() }
    }

    fn count(&self, class: Class, kind: MediaKind) -> usize {
        self.rows.iter().filter(|r| r.class == class && r.kind == kind).count()
    }

    pub fn slot_rows(&self) -> usize {
        self.rows.iter().filter(|r| r.class == Class::Slot).count()
    }

    fn cap(&self, class: Class, kind: MediaKind) -> usize {
        match (class, kind) {
            (Class::Full, MediaKind::Audio) => FULL_AUDIO_CAP,
            (Class::Full, MediaKind::Video) => FULL_VIDEO_CAP,
            (Class::Slot, _) => self.slot_reserve,
        }
    }

    pub fn row_of(&self, track_id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.track.as_ref().map(|t| t.0.as_str()) == Some(track_id))
    }

    pub fn row_by_ssrc(&self, ssrc: u32) -> Option<&Row> {
        self.rows.iter().find(|r| r.ssrc == ssrc || r.rtx_ssrc == Some(ssrc))
    }

    pub fn fresh_ssrc(&mut self, mut rnd: impl FnMut() -> u32) -> u32 {
        loop {
            let s = rnd() | 0x8000_0000;
            if !self.ssrcs.contains(&s) && !self.uplink.contains(&s) {
                self.ssrcs.insert(s);
                return s;
            }
        }
    }

    fn bump(&mut self) -> u64 {
        self.arev += 1;
        self.arev
    }

    fn load(&mut self, i: usize, w: &Want, arev: u64, now: u64) -> Change {
        let r = &mut self.rows[i];
        r.track = Some((w.track_id.clone(), w.room_id.clone()));
        r.since = arev;
        r.since_ms = now;
        r.used_at = now;
        Change::Load { mid: r.mid, track_id: w.track_id.clone(), room_id: w.room_id.clone() }
    }

    fn new_row(&mut self, w: &Want, rnd: &mut impl FnMut() -> u32) -> Option<usize> {
        let (tuple, fmtp) = Tuple::room_codec(w.kind, &w.codec);
        let (pt, rtx_pt) = self.pts.of(&tuple)?;
        let ssrc = self.fresh_ssrc(&mut *rnd);
        let rtx_ssrc = rtx_pt.map(|_| self.fresh_ssrc(&mut *rnd));
        let mid = self.next_mid;
        self.next_mid += 1;
        self.rows.push(Row { mid, kind: w.kind, class: w.class, tuple, codec: w.codec.clone(), fmtp, pt, rtx_pt, ssrc, rtx_ssrc, track: None, since: 0, since_ms: 0, used_at: 0 });
        Some(self.rows.len() - 1)
    }

    fn retuple(&mut self, i: usize, w: &Want) -> bool {
        let (tuple, fmtp) = Tuple::room_codec(w.kind, &w.codec);
        let Some((pt, rtx_pt)) = self.pts.of(&tuple) else { return false };
        let r = &mut self.rows[i];
        r.tuple = tuple;
        r.codec = w.codec.clone();
        r.fmtp = fmtp;
        r.pt = pt;
        r.rtx_pt = rtx_pt;
        true
    }

    pub fn can_add_slots(&self, n: usize) -> bool {
        let empty_slots = |k| self.rows.iter().filter(|r| r.class == Class::Slot && r.kind == k && r.track.is_none()).count();
        let need_a = n.saturating_sub(empty_slots(MediaKind::Audio));
        let need_v = n.saturating_sub(empty_slots(MediaKind::Video));
        self.slot_rows() + need_a + need_v <= self.slot_reserve && self.rows.len() + need_a + need_v <= self.budget()
    }

    pub fn want(&mut self, w: Want, now: u64, mut rnd: impl FnMut() -> u32) -> (Fill, Vec<Change>) {
        if let Some(r) = self.row_of(&w.track_id) {
            return (Fill::Loaded(r.mid), Vec::new());
        }
        let (tuple, _) = Tuple::room_codec(w.kind, &w.codec);
        let empty = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.class == w.class && r.kind == w.kind && r.track.is_none() && r.tuple == tuple)
            .min_by_key(|(_, r)| r.mid)
            .map(|(i, _)| i);
        if let Some(i) = empty {
            let a = self.bump();
            let c = self.load(i, &w, a, now);
            return (Fill::Loaded(self.rows[i].mid), vec![c]);
        }
        let under = self.count(w.class, w.kind) < self.cap(w.class, w.kind) && self.rows.len() < self.budget();
        if under {
            let Some(i) = self.new_row(&w, &mut rnd) else { return (Fill::NoPt, Vec::new()) };
            let a = self.bump();
            let c = self.load(i, &w, a, now);
            return (Fill::Loaded(self.rows[i].mid), vec![c]);
        }
        if w.class == Class::Slot {
            return (Fill::Over, Vec::new());
        }
        let other_empty = self.rows.iter().enumerate().filter(|(_, r)| r.class == w.class && r.kind == w.kind && r.track.is_none()).min_by_key(|(_, r)| r.mid).map(|(i, _)| i);
        if let Some(i) = other_empty {
            if !self.retuple(i, &w) {
                return (Fill::NoPt, Vec::new());
            }
            let a = self.bump();
            let c = self.load(i, &w, a, now);
            return (Fill::Loaded(self.rows[i].mid), vec![c]);
        }
        self.waiting.retain(|x| x.track_id != w.track_id);
        self.waiting.push(w);
        (Fill::Waiting, Vec::new())
    }

    pub fn victim(&self, kind: MediaKind, rank_of: &dyn Fn(&str) -> u64, rank: u64) -> Option<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.class == Class::Full && r.kind == kind && r.track.is_some())
            .map(|(i, r)| (i, rank_of(r.track.as_ref().map(|t| t.0.as_str()).unwrap_or("")), r.used_at))
            .filter(|(_, rk, _)| *rk < rank)
            .min_by_key(|(_, rk, at)| (*rk, *at))
            .map(|(i, _, _)| i)
    }

    pub fn reassign(&mut self, rank_of: &dyn Fn(&str) -> u64, now: u64) -> Vec<Change> {
        let mut out = Vec::new();
        let mut wait = std::mem::take(&mut self.waiting);
        wait.sort_by(|a, b| b.rank.cmp(&a.rank).then(a.since_ms.cmp(&b.since_ms)));
        let mut keep = Vec::new();
        for w in wait {
            let (tuple, _) = Tuple::room_codec(w.kind, &w.codec);
            let Some(i) = self.victim(w.kind, rank_of, w.rank).filter(|i| self.rows[*i].tuple == tuple) else {
                keep.push(w);
                continue;
            };
            let Some((tid, rid)) = self.rows[i].track.clone() else { continue };
            let vr = rank_of(&tid);
            out.push(Change::Unload { mid: self.rows[i].mid, track_id: tid.clone(), room_id: rid.clone() });
            let a = self.bump();
            out.push(self.load(i, &w, a, now));
            keep.push(Want { track_id: tid, room_id: rid, kind: w.kind, class: Class::Full, codec: self.rows[i].codec.clone(), rank: vr, since_ms: now });
        }
        self.waiting = keep;
        out
    }

    pub fn unwant(&mut self, track_id: &str, now: u64) -> Vec<Change> {
        self.waiting.retain(|w| w.track_id != track_id);
        let Some(i) = self.rows.iter().position(|r| r.track.as_ref().map(|t| t.0.as_str()) == Some(track_id)) else { return Vec::new() };
        let (tid, rid) = self.rows[i].track.take().unwrap_or_default();
        let a = self.bump();
        self.rows[i].since = a;
        self.rows[i].since_ms = now;
        let mut out = vec![Change::Unload { mid: self.rows[i].mid, track_id: tid, room_id: rid }];
        out.extend(self.refill(i, now));
        out
    }

    fn refill(&mut self, i: usize, now: u64) -> Vec<Change> {
        let kind = self.rows[i].kind;
        let class = self.rows[i].class;
        let tuple = self.rows[i].tuple.clone();
        let pos = self
            .waiting
            .iter()
            .enumerate()
            .filter(|(_, w)| w.kind == kind && w.class == class && Tuple::room_codec(w.kind, &w.codec).0 == tuple)
            .min_by_key(|(_, w)| w.since_ms)
            .map(|(j, _)| j);
        let Some(j) = pos else { return Vec::new() };
        let w = self.waiting.remove(j);
        let a = self.bump();
        vec![self.load(i, &w, a, now)]
    }

    pub fn unwant_room(&mut self, room_id: &str, now: u64) -> Vec<Change> {
        let ids: Vec<String> = self.rows.iter().filter_map(|r| r.track.as_ref().filter(|t| t.1 == room_id).map(|t| t.0.clone())).collect();
        self.waiting.retain(|w| w.room_id != room_id);
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.unwant(&id, now));
        }
        out
    }

    pub fn rehome(&mut self, track_id: &str, room_id: &str) -> bool {
        let mut changed = false;
        for r in self.rows.iter_mut() {
            if let Some(t) = r.track.as_mut().filter(|t| t.0 == track_id && t.1 != room_id) {
                t.1 = room_id.to_string();
                changed = true;
            }
        }
        for w in self.waiting.iter_mut().filter(|w| w.track_id == track_id) {
            w.room_id = room_id.to_string();
        }
        if changed {
            self.bump();
        }
        changed
    }

    pub fn acked(&mut self, arev: u64) -> bool {
        if arev > self.ack && arev <= self.arev {
            self.ack = arev;
            return true;
        }
        false
    }

    pub fn sendable(&self, mid: u16) -> bool {
        self.rows.iter().find(|r| r.mid == mid).map(|r| r.track.is_some() && r.since <= self.ack).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnd() -> impl FnMut() -> u32 {
        let mut n = 0u32;
        move || {
            n += 1;
            n
        }
    }

    fn want(id: &str, kind: MediaKind, class: Class, rank: u64) -> Want {
        let codec = if kind == MediaKind::Audio { "opus" } else { "H264" };
        Want { track_id: id.into(), room_id: "r".into(), kind, class, codec: codec.into(), rank, since_ms: 0 }
    }

    #[test]
    fn mid_starts_by_mode_and_seeds_pt() {
        let mut t = Table::new(PcMode::Two, 40);
        let (f, _) = t.want(want("a", MediaKind::Audio, Class::Full, 0), 1, rnd());
        assert_eq!(f, Fill::Loaded(0));
        assert_eq!(t.rows[0].pt, 111);
        let (f, _) = t.want(want("v", MediaKind::Video, Class::Full, 0), 1, rnd());
        assert_eq!(f, Fill::Loaded(1));
        assert_eq!((t.rows[1].pt, t.rows[1].rtx_pt), (102, Some(103)));
        assert!(t.rows.iter().all(|r| r.ssrc >= 0x8000_0000));
        let mut o = Table::new(PcMode::One, 40);
        o.pts.seed_tags(&[JoinCodec { kind: MediaKind::Video, pt: 102, name: "VP8".into(), fmtp: None, rtx_pt: Some(103) }]);
        o.want(want("v", MediaKind::Video, Class::Full, 0), 1, rnd());
        assert_eq!(o.rows[0].mid, 32);
        assert_ne!(o.rows[0].pt, 102, "신고된 PT 는 다른 튜플에 안 준다");
    }

    #[test]
    fn empty_row_is_reused_before_new_row() {
        let mut t = Table::new(PcMode::Two, 40);
        t.want(want("a", MediaKind::Audio, Class::Full, 0), 1, rnd());
        t.want(want("b", MediaKind::Audio, Class::Full, 0), 1, rnd());
        t.unwant("a", 2);
        let (f, _) = t.want(want("c", MediaKind::Audio, Class::Full, 0), 3, rnd());
        assert_eq!(f, Fill::Loaded(0), "S-sym-IMPL-SUB-TABLE-1 연:RULE-SUB-FILL ①");
        assert_eq!(t.rows.len(), 2);
    }

    #[test]
    fn cap_waits_then_recovers() {
        let mut t = Table::new(PcMode::Two, 40);
        for i in 0..FULL_AUDIO_CAP {
            t.want(want(&format!("a{i}"), MediaKind::Audio, Class::Full, 0), 1, rnd());
        }
        let (f, _) = t.want(want("late", MediaKind::Audio, Class::Full, 0), 2, rnd());
        assert_eq!(f, Fill::Waiting);
        let ch = t.unwant("a3", 3);
        assert!(ch.iter().any(|c| matches!(c, Change::Load { track_id, .. } if track_id == "late")), "S-sym-IMPL-SUB-TABLE-2 연:RULE-SUB-ASSIGN-RECOVERED");
    }

    #[test]
    fn ack_gates_changed_rows() {
        let mut t = Table::new(PcMode::Two, 40);
        t.want(want("a", MediaKind::Audio, Class::Full, 0), 1, rnd());
        assert!(!t.sendable(0), "S-sym-IMPL-SUB-TABLE-3 연:RULE-SUB-EGRESS-ACKED");
        assert!(t.acked(1));
        assert!(t.sendable(0));
        assert!(!t.acked(1));
    }
}
