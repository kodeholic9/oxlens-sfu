// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:IMPL-SUB-TABLE · 정:IMPL-SUB-SUBSCRIBER-TABLE · 정:CAND-SUB-ROW-CLASS · 정:CAND-SUB-PAUSED-FREES-ROW · 연:SUB.type.MlineTable · 연:SUB.type.MlineEntry · 연:RULE-SUB-FILL · 연:RULE-SUB-EGRESS-ACKED · 연:RULE-SUB-ASSIGN-RECOVERED · 연:RULE-SUB-REMOVE-KEEPS-MID · 연:RULE-SUB-TABLE-SERVER · 연:RULE-LINK-DOWNLINK-SSRC · 연:RULE-LINK-EGRESS-REWRITE · 연:RULE-ROOM-BUDGET-SPLIT · 연:RULE-ROOM-BUDGET-MATH · 연:RULE-ROOM-JOIN-NUMBER-TAGS · 연:LINK.S.track_on · 연:LINK.S.track_on_full · 연:LINK.S.track_off · 연:LINK.S.track_off_norow · 연:LINK.S.reissue · 연:LINK.S.reassign · 연:LINK.S.ack · 연:LINK.S.ack_old · model: claude-opus-5-5
//! 배정표(순수) — 연결마다 하나. 줄 = 구독 m-line(mid · 튜플 · 다운링크 SSRC · 실린 트랙 · since). 빈 줄 → 새 줄 → 재배정 · PT 씨앗 · 줄은 지우지 않는다.

use std::collections::{BTreeMap, HashSet};

use oxsig::types::Kind;
use oxsig::{MlineEntry, MlineTable, PcMode};

pub const FULL_AUDIO_CAP: usize = 25;
pub const FULL_VIDEO_CAP: usize = 25;
pub const SUB_1PC: usize = 96;
pub const SUB_2PC: usize = 128;
pub const MID_BASE_1PC: u16 = 32;
pub const MID_BASE_2PC: u16 = 0;
pub const MID_MAX: u16 = 255;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tuple {
    pub kind: Kind,
    pub codec: String,
    pub key: String,
}

fn fmtp_param<'a>(fmtp: &'a str, name: &str) -> Option<&'a str> {
    fmtp.split(';').map(str::trim).find_map(|kv| kv.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

pub fn codec_norm(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "opus" => Some("opus"),
        "vp8" => Some("VP8"),
        "h264" => Some("H264"),
        _ => None,
    }
}

pub fn clock_of(codec: &str) -> u32 {
    if codec.eq_ignore_ascii_case("opus") { 48_000 } else { 90_000 }
}

impl Tuple {
    pub fn of(kind: Kind, codec: &str, fmtp: Option<&str>) -> Tuple {
        let norm = codec_norm(codec).map(str::to_string).unwrap_or_else(|| codec.to_string());
        let f = fmtp.unwrap_or("");
        let key = match norm.as_str() {
            "opus" => fmtp_param(f, "sprop-stereo").map(|v| format!("sprop-stereo={v}")).unwrap_or_default(),
            "H264" => {
                let pm = fmtp_param(f, "packetization-mode").unwrap_or("0");
                let prof = fmtp_param(f, "profile-level-id").map(|p| p.get(..4).unwrap_or(p).to_ascii_lowercase()).unwrap_or_else(|| "42e0".into());
                format!("packetization-mode={pm};profile={prof}")
            }
            _ => String::new(),
        };
        Tuple { kind, codec: norm, key }
    }

    pub fn fmtp(&self, room_fmtp: Option<&str>) -> Option<String> {
        match self.codec.as_str() {
            "opus" => {
                let mut f = "minptime=10;useinbandfec=1".to_string();
                if let Some(v) = self.key.strip_prefix("sprop-stereo=") {
                    f.push_str(&format!(";sprop-stereo={v}"));
                }
                Some(f)
            }
            "H264" => {
                let pm = fmtp_param(&self.key, "packetization-mode").unwrap_or("1");
                let plid = room_fmtp.and_then(|r| fmtp_param(r, "profile-level-id")).map(str::to_string).unwrap_or_else(|| "42e01f".into());
                Some(format!("level-asymmetry-allowed=1;packetization-mode={pm};profile-level-id={plid}"))
            }
            _ => None,
        }
    }

    pub fn rtx(&self) -> bool {
        self.kind == Kind::Video
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowClass {
    Full,
    Slot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub mid: u16,
    pub kind: Kind,
    pub class: RowClass,
    pub tuple: Tuple,
    pub fmtp: Option<String>,
    pub ssrc: u32,
    pub rtx_ssrc: Option<u32>,
    pub track: Option<(String, String)>,
    pub since: u64,
    pub loaded_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pts {
    map: BTreeMap<Tuple, (u8, Option<u8>)>,
    reserved: HashSet<u8>,
    seeded: bool,
    pub ignored_tags: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub kind: Kind,
    pub pt: u8,
    pub name: String,
    pub fmtp: Option<String>,
    pub rtx_pt: Option<u8>,
}

impl Pts {
    fn used(&self) -> HashSet<u8> {
        let mut u: HashSet<u8> = self.reserved.clone();
        for (pt, rtx) in self.map.values() {
            u.insert(*pt);
            if let Some(r) = rtx {
                u.insert(*r);
            }
        }
        u
    }

    pub fn seed_tags(&mut self, tags: &[Tag]) -> bool {
        if self.seeded {
            let differs = tags.iter().any(|t| {
                let tu = Tuple::of(t.kind, &t.name, t.fmtp.as_deref());
                self.map.get(&tu).map(|(pt, _)| *pt != t.pt).unwrap_or(false)
            });
            if differs {
                self.ignored_tags += 1;
            }
            return !differs;
        }
        self.seeded = true;
        for t in tags {
            self.reserved.insert(t.pt);
            if let Some(r) = t.rtx_pt {
                self.reserved.insert(r);
            }
            if codec_norm(&t.name).is_none() {
                continue;
            }
            let tu = Tuple::of(t.kind, &t.name, t.fmtp.as_deref());
            self.map.entry(tu).or_insert((t.pt, t.rtx_pt));
        }
        true
    }

    fn next_free(&self, used: &HashSet<u8>, from: u8, to: u8) -> Option<u8> {
        (from..=to).find(|p| !used.contains(p))
    }

    pub fn of(&mut self, t: &Tuple) -> (u8, Option<u8>) {
        if let Some(v) = self.map.get(t) {
            return *v;
        }
        let used = self.used();
        let has = |c: &str| self.map.keys().any(|k| k.codec == c);
        let seed: Option<(u8, Option<u8>)> = match t.codec.as_str() {
            "opus" if !used.contains(&111) && !has("opus") => Some((111, None)),
            "VP8" if !used.contains(&96) && !used.contains(&97) && !has("VP8") => Some((96, Some(97))),
            "H264" if !used.contains(&102) && !used.contains(&103) && !has("H264") => Some((102, Some(103))),
            _ => None,
        };
        let v = seed.unwrap_or_else(|| {
            let mut u = used.clone();
            let pick = |u: &mut HashSet<u8>| {
                let p = self.next_free(u, 96, 127).or_else(|| self.next_free(u, 35, 63)).unwrap_or(127);
                u.insert(p);
                p
            };
            let pt = pick(&mut u);
            let rtx = t.rtx().then(|| pick(&mut u));
            (pt, rtx)
        });
        self.map.insert(t.clone(), v);
        v
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    Row(u16),
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub sfu_id: String,
    pub pc_mode: PcMode,
    pub rows: Vec<Row>,
    pub arev: u64,
    pub pts: Pts,
    dirty: bool,
    pub waiting: Vec<(String, String, Kind, Tuple, u64)>,
}

impl Table {
    pub fn new(sfu_id: &str, pc_mode: PcMode) -> Table {
        Table { sfu_id: sfu_id.to_string(), pc_mode, rows: Vec::new(), arev: 0, pts: Pts::default(), dirty: false, waiting: Vec::new() }
    }

    fn mid_base(&self) -> u16 {
        match self.pc_mode {
            PcMode::V1pc => MID_BASE_1PC,
            PcMode::V2pc => MID_BASE_2PC,
        }
    }

    pub fn budget(&self) -> usize {
        match self.pc_mode {
            PcMode::V1pc => SUB_1PC,
            PcMode::V2pc => SUB_2PC,
        }
    }

    fn next_arev(&mut self) -> u64 {
        self.dirty = true;
        self.arev + 1
    }

    pub fn commit(&mut self) -> bool {
        if self.dirty {
            self.arev += 1;
            self.dirty = false;
            return true;
        }
        false
    }

    pub fn rollback_to(&mut self, snapshot: Table) {
        let _ = std::mem::replace(self, snapshot);
    }

    pub fn occupied(&self, class: RowClass, kind: Option<Kind>) -> usize {
        self.rows.iter().filter(|r| r.class == class && r.track.is_some() && kind.map(|k| k == r.kind).unwrap_or(true)).count()
    }

    pub fn rows_of(&self, class: RowClass, kind: Kind) -> usize {
        self.rows.iter().filter(|r| r.class == class && r.kind == kind).count()
    }

    pub fn row_of(&self, track: &str, room: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.track.as_ref().map(|(t, rm)| t == track && rm == room).unwrap_or(false))
    }

    pub fn row_by_track(&self, track: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.track.as_ref().map(|(t, _)| t == track).unwrap_or(false))
    }

    fn cap(&self, class: RowClass, kind: Kind) -> usize {
        match (class, kind) {
            (RowClass::Full, Kind::Audio) => FULL_AUDIO_CAP,
            (RowClass::Full, Kind::Video) => FULL_VIDEO_CAP,
            (RowClass::Slot, _) => usize::MAX,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn place(&mut self, track: &str, room: &str, kind: Kind, class: RowClass, tuple: &Tuple, room_fmtp: Option<&str>, now_ms: u64, mut fresh_ssrc: impl FnMut() -> u32) -> Placed {
        if let Some(r) = self.row_of(track, room) {
            return Placed::Row(r.mid);
        }
        let arev = self.arev + 1;
        if let Some(i) = self.rows.iter().enumerate().filter(|(_, r)| r.track.is_none() && r.class == class && r.kind == kind && &r.tuple == tuple).min_by_key(|(_, r)| r.mid).map(|(i, _)| i) {
            self.dirty = true;
            let row = &mut self.rows[i];
            row.track = Some((track.to_string(), room.to_string()));
            row.since = arev;
            row.loaded_at = now_ms;
            return Placed::Row(row.mid);
        }
        let mid = self.mid_base() + self.rows.len() as u16;
        if self.rows_of(class, kind) >= self.cap(class, kind) || self.rows.len() >= self.budget() || mid > MID_MAX {
            return Placed::Full;
        }
        let _ = self.next_arev();
        self.pts.of(tuple);
        let ssrc = fresh_ssrc();
        let rtx_ssrc = tuple.rtx().then(&mut fresh_ssrc);
        self.rows.push(Row {
            mid,
            kind,
            class,
            tuple: tuple.clone(),
            fmtp: tuple.fmtp(room_fmtp),
            ssrc,
            rtx_ssrc,
            track: Some((track.to_string(), room.to_string())),
            since: arev,
            loaded_at: now_ms,
        });
        Placed::Row(mid)
    }

    pub fn clear(&mut self, track: &str, room: &str) -> Option<u16> {
        let arev = self.arev + 1;
        let i = self.rows.iter().position(|r| r.track.as_ref().map(|(t, rm)| t == track && rm == room).unwrap_or(false))?;
        self.dirty = true;
        let row = &mut self.rows[i];
        row.track = None;
        row.since = arev;
        Some(row.mid)
    }

    pub fn clear_room(&mut self, room: &str) -> Vec<u16> {
        let ids: Vec<(String, String)> = self.rows.iter().filter_map(|r| r.track.clone()).filter(|(_, rm)| rm == room).collect();
        self.waiting.retain(|w| w.1 != room);
        ids.iter().filter_map(|(t, rm)| self.clear(t, rm)).collect()
    }

    pub fn move_room(&mut self, track: &str, from: &str, to: &str) -> bool {
        let arev = self.arev + 1;
        let Some(i) = self.rows.iter().position(|r| r.track.as_ref().map(|(t, rm)| t == track && rm == from).unwrap_or(false)) else { return false };
        self.dirty = true;
        let row = &mut self.rows[i];
        row.track = Some((track.to_string(), to.to_string()));
        row.since = arev;
        true
    }

    pub fn reassign(&mut self, from_track: &str, from_room: &str, track: &str, room: &str, now_ms: u64) -> Option<u16> {
        let arev = self.arev + 1;
        let i = self.rows.iter().position(|r| r.track.as_ref().map(|(t, rm)| t == from_track && rm == from_room).unwrap_or(false))?;
        self.dirty = true;
        let row = &mut self.rows[i];
        row.track = Some((track.to_string(), room.to_string()));
        row.since = arev;
        row.loaded_at = now_ms;
        Some(row.mid)
    }

    pub fn wait(&mut self, track: &str, room: &str, kind: Kind, tuple: &Tuple, now_ms: u64) {
        if !self.waiting.iter().any(|w| w.0 == track && w.1 == room) {
            self.waiting.push((track.to_string(), room.to_string(), kind, tuple.clone(), now_ms));
        }
    }

    pub fn unwait(&mut self, track: &str, room: &str) -> bool {
        let before = self.waiting.len();
        self.waiting.retain(|w| !(w.0 == track && w.1 == room));
        before != self.waiting.len()
    }

    pub fn ssrcs(&self) -> HashSet<u32> {
        self.rows.iter().flat_map(|r| [Some(r.ssrc), r.rtx_ssrc]).flatten().collect()
    }

    pub fn entry(&self, r: &Row) -> MlineEntry {
        let (pt, rtx_pt) = self.pts.map.get(&r.tuple).copied().unwrap_or((0, None));
        MlineEntry {
            mid: r.mid.to_string(),
            kind: r.kind,
            pt,
            codec: r.tuple.codec.clone(),
            fmtp: r.fmtp.clone(),
            rtx_pt: if r.tuple.rtx() { rtx_pt } else { None },
            track_id: r.track.as_ref().map(|(t, _)| t.clone()),
            room_id: r.track.as_ref().map(|(_, rm)| rm.clone()),
            ssrc: r.ssrc,
            rtx_ssrc: r.rtx_ssrc,
            stream_id: match r.class {
                RowClass::Slot => "ox-ptt".to_string(),
                RowClass::Full => format!("ox-m{}", r.mid),
            },
        }
    }

    pub fn wire(&self) -> MlineTable {
        MlineTable { sfu_id: self.sfu_id.clone(), arev: self.arev, mlines: self.rows.iter().map(|r| self.entry(r)).collect() }
    }

    pub fn pt_of(&self, t: &Tuple) -> Option<(u8, Option<u8>)> {
        self.pts.map.get(t).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus() -> Tuple {
        Tuple::of(Kind::Audio, "opus", Some("minptime=10;useinbandfec=1"))
    }

    fn h264() -> Tuple {
        Tuple::of(Kind::Video, "H264", Some("packetization-mode=1;profile-level-id=42e01f"))
    }

    #[test]
    fn empty_row_first_then_new_row_and_mid_kept() {
        let mut n = 1000u32;
        let mut t = Table::new("sfu-x", PcMode::V2pc);
        let mut ssrc = || {
            n += 1;
            n
        };
        assert_eq!(t.place("a", "r", Kind::Audio, RowClass::Full, &opus(), None, 0, &mut ssrc), Placed::Row(0));
        assert_eq!(t.place("v", "r", Kind::Video, RowClass::Full, &h264(), None, 0, &mut ssrc), Placed::Row(1));
        assert!(t.commit());
        assert_eq!(t.arev, 1);
        assert_eq!(t.clear("a", "r"), Some(0));
        assert!(t.commit());
        assert_eq!(t.rows.len(), 2, "S-sym-IMPL-SUB-TABLE-1 줄은 지우지 않는다");
        assert_eq!(t.place("b", "r", Kind::Audio, RowClass::Full, &opus(), None, 1, &mut ssrc), Placed::Row(0), "S-sym-IMPL-SUB-TABLE-2 같은 kind · 튜플의 가장 작은 빈 줄부터");
        assert_eq!(t.place("w", "r", Kind::Video, RowClass::Slot, &h264(), None, 1, &mut ssrc), Placed::Row(2), "부류가 다르면 새 줄");
        t.commit();
        let w = t.wire();
        assert_eq!(w.mlines[0].pt, 111);
        assert_eq!(w.mlines[1].pt, 102);
        assert_eq!(w.mlines[1].rtx_pt, Some(103));
        assert_eq!(w.mlines[2].stream_id, "ox-ptt");
        assert_eq!(w.mlines[1].stream_id, "ox-m1");
    }

    #[test]
    fn caps_and_1pc_seed() {
        let mut t = Table::new("s", PcMode::V1pc);
        let mut k = 0u32;
        t.pts.seed_tags(&[Tag { kind: Kind::Audio, pt: 109, name: "opus".into(), fmtp: Some("minptime=10".into()), rtx_pt: None }, Tag { kind: Kind::Video, pt: 96, name: "VP8".into(), fmtp: None, rtx_pt: Some(97) }]);
        for i in 0..FULL_VIDEO_CAP {
            assert!(matches!(t.place(&format!("v{i}"), "r", Kind::Video, RowClass::Full, &h264(), None, 0, || {
                k += 1;
                k
            }), Placed::Row(_)));
        }
        assert_eq!(t.place("v99", "r", Kind::Video, RowClass::Full, &h264(), None, 0, || 7), Placed::Full, "연결마다 video 25");
        assert_eq!(t.rows[0].mid, MID_BASE_1PC, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-1 1pc 는 32 부터");
        let (pt, rtx) = t.pt_of(&h264()).expect("pt");
        assert!(pt != 96 && pt != 97 && Some(96) != rtx, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-2 신고된 PT 는 다른 튜플에 안 준다");
        assert_eq!(t.pts.of(&opus()).0, 109, "S-sym-IMPL-SUB-SUBSCRIBER-TABLE-3 같은 튜플은 번호표의 PT 재사용");
    }

    #[test]
    fn tuple_keys() {
        assert_eq!(Tuple::of(Kind::Audio, "opus", Some("minptime=10")), Tuple::of(Kind::Audio, "OPUS", Some("useinbandfec=1")));
        assert_ne!(Tuple::of(Kind::Audio, "opus", Some("sprop-stereo=1")), opus());
        assert_eq!(h264().fmtp(None).as_deref(), Some("level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"));
    }
}
