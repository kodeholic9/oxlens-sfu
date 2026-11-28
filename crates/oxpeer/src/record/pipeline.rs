// author: kodeholic (powered by Claude)
//! 녹음 파이프라인(SDK:RULE-PLAT-REC-PIPE · REC-SPLIT) — 레코드 하나를 받아 출력 열을 돌려주는 ★순수 상태기.
//! 분기(줄 → 트랙 · CSRC → 출처) → 조립(차례가 선 패킷 · `gap` 은 깨진 프레임 · 영상은 키프레임부터) → 시간축(SR → UTC) → 분할 정책.
//! ★시스템 시계를 읽지 않는다 — 시간은 레코드의 `log_time` 과 RTP · SR 뿐이다. 그래서 라이브 레코드열과 파일 레코드열이 같으면 출력도 같다.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use super::{Channel, Record};
use crate::link::depack::{self, Codec};
use crate::link::transport::rtp_header;

/// NTP(1900) → 유닉스(1970)
const NTP_UNIX: u64 = 2_208_988_800;

/// 분할 정책(SDK:RULE-PLAT-REC-SPLIT)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// 줄마다 출처가 바뀌면 새 세그먼트 · 출처 없는 프레임은 세그먼트 밖(`Output::Orphan`)
    Utterance,
    /// 트랙마다 시간(ns) · 크기(bytes)가 지난 뒤 영상은 첫 키프레임 · 음성은 아무 프레임에서 새 세그먼트
    Span { time_ns: u64, bytes: u64 },
}

/// 프레임(지점 ②)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub segment: Option<u64>,
    pub room: String,
    pub track_id: String,
    /// 트랙 주인(개인 트랙) — 무전 슬롯은 없다
    pub owner: Option<String>,
    pub kind: String,
    pub codec: String,
    /// 줄(다운링크) SSRC
    pub ssrc: u32,
    /// 출처 번호(CSRC) · 그 주인 — 없으면 출처 없는 프레임(표지를 끈 연결 · 서버는 미디어 프레임을 짓지 않는다 — 20261111)
    pub source: Option<u32>,
    pub user_id: Option<String>,
    pub keyframe: bool,
    pub rtp_ts: u32,
    /// SR 로 이은 UTC(ns) — 첫 SR 전이면 없다
    pub utc_ns: Option<u64>,
    /// 첫 패킷 레코드의 시각
    pub log_time: u64,
    pub seq_first: u16,
    pub seq_last: u16,
    pub rec_first: u32,
    pub rec_last: u32,
    pub data: Vec<u8>,
    /// 메운 프레임(Opus PLC — SDK:RULE-PLAT-REC-GAP) — 받은 것이 아니다
    pub filled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Opened,
    Closed,
}

/// 세그먼트 사건(지점 ③)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub event: SegmentKind,
    pub id: u64,
    pub room: String,
    pub track_id: String,
    pub owner: Option<String>,
    pub kind: String,
    pub codec: String,
    pub source: Option<u32>,
    pub user_id: Option<String>,
    pub first_utc_ns: Option<u64>,
    pub last_utc_ns: Option<u64>,
    pub first_log_time: u64,
    pub last_log_time: u64,
    pub frames: u64,
    pub bytes: u64,
    /// 메운 프레임 수(SDK:RULE-PLAT-REC-GAP)
    pub filled: u64,
    /// 닫힌 까닭 — `taken` · `idle` · `span` · `gap` · `track` · `end`
    pub reason: Option<&'static str>,
    /// 발언 번호 — 발언 정책의 무전 슬롯 세그먼트(발언권 허가마다 · 같은 발언의 음성 · 영상이 같은 번호 — SDK:RULE-PLAT-REC-SPLIT)
    pub utterance: Option<u64>,
    /// 발언 id — 그 허가의 `taken` 이 실은 서버 발언 id(연:RULE-FLR-TLV `0x20` · 발언 기록의 `msg_id` — SDK:RULE-PLAT-REC-SPLIT) · 없으면(이 필드 전의 서버) `utterance` 를 쓴다
    pub talk: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Frame(Frame),
    Segment(Segment),
    /// 발언 정책이 어느 발언에도 담지 않은 프레임(출처 없음 · 지금 화자가 아님 · 허가 없이 1초)
    Orphan(Frame),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    room: String,
    track_id: String,
    kind: String,
    codec: String,
}

struct Asm {
    ts: u32,
    data: Vec<u8>,
    key: bool,
    broken: bool,
    source: Option<u32>,
    log_time: u64,
    seq_first: u16,
    seq_last: u16,
    rec_first: u32,
    rec_last: u32,
}

#[derive(Default)]
struct Line {
    asm: Option<Asm>,
    need_key: bool,
}

/// 파이프라인 계수 — 결과물 요약 · 시험이 읽는다
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub records: u64,
    pub packets: u64,
    /// 줄을 모르는 패킷(`ctl/track` 전)
    pub unrouted: u64,
    pub frames: u64,
    pub broken: u64,
    /// 키프레임 전이라 버린 영상 프레임
    pub before_key: u64,
    /// 메운 PLC 프레임(SDK:RULE-PLAT-REC-GAP)
    pub filled: u64,
    /// 목록에서 빠진 개인 트랙으로 판정된 늦은 프레임(SDK:RULE-PLAT-REC-SPLIT 트랙이 사라짐) — 세그먼트에 넣지 않았다
    pub late: u64,
    /// 발언 정책 — 지금 화자가 아닌 출처의 슬롯 프레임(앞 화자의 늦은 패킷) · 담지 않았다
    pub other_speaker: u64,
    /// 발언 정책 — 허가 전에 와 들고 있던 프레임 중 그 허가와 안 맞거나 1초를 넘은 것 · 담지 않았다
    pub unheld: u64,
}

/// 방의 지금 화자(`ctl/floor` `taken`) — 허가 번호(MBCP SEQ — 같은 허가의 재전송 · 파트 머리 다시 적기를 가린다) · 출처 번호 · 발언 번호
#[derive(Clone)]
struct Speaker {
    seq: Option<u64>,
    sources: Vec<u32>,
    utt: u64,
    talk: Option<String>,
}

/// 끝나는 발언 — 반환 · 다음 허가 뒤 유예 동안 앞 화자의 늦은 패킷을 그 발언에 넣는다
struct Ending {
    old: Speaker,
    deadline: u64,
    reason: &'static str,
}

impl Speaker {
    fn owns(&self, src: Option<u32>) -> bool {
        match src {
            Some(c) => self.sources.is_empty() || self.sources.contains(&c),
            None => false,
        }
    }
}

/// 허가 전에 온 슬롯 프레임을 드는 시간(`log_time` ns — SDK:RULE-PLAT-REC-SPLIT 발언)
const HOLD_BEFORE_TAKEN_NS: u64 = 1_000_000_000;
/// 반환 · 다음 허가 뒤 앞 화자의 늦은 패킷을 그 발언에 받는 유예(`log_time` ns — 발언권 사건과 미디어는 길이 달라 뒤바뀐다)
const ENDING_GRACE_NS: u64 = 200_000_000;

/// 음성 세그먼트의 꼬리 — 다음 프레임이 이어 붙을 RTP 시각 · 앞 TOC · 넘긴 나머지(표본)
#[derive(Clone, Copy)]
struct Tail {
    end_ts: u32,
    toc: u8,
    carry: u64,
    utc_end: Option<u64>,
    ssrc: u32,
    end_log: u64,
}

/// 구멍 상한 기본(표본 — 5분 · Jitsi 기본과 같다)
const GAP_MAX_SAMPLES: u64 = 5 * 60 * 48_000;

pub struct Pipeline {
    policy: Policy,
    gap_max: u64,
    tails: HashMap<String, Tail>,
    rows: HashMap<u32, Row>,
    owners: HashMap<String, Option<String>>,
    sources: HashMap<u32, String>,
    /// 출처 번호 → 개인 트랙(SDK:RULE-PLAT-REC-PIPE 분기 — 프레임의 트랙 = 출처 트랙)
    src_tracks: HashMap<u32, String>,
    /// 방마다 목록에 있는 개인 트랙(빠질 때 세그먼트를 닫는 데만 쓴다)
    listed: HashMap<String, BTreeSet<String>>,
    /// 방마다 지금 목록의 트랙 전부(개인 · 슬롯) — 늦은 프레임 판정은 이 스냅숏만 본다(SDK:RULE-PLAT-REC-PART 파트 k 는 앞 파트 없이 읽힌다)
    present: HashMap<String, BTreeSet<String>>,
    lines: HashMap<u32, Line>,
    sr: HashMap<u32, (u64, u32)>,
    open: BTreeMap<String, Segment>,
    next_id: u64,
    counts: Counts,
    speaking: HashMap<String, Speaker>,
    held: HashMap<String, Vec<Frame>>,
    ending: BTreeMap<String, Ending>,
    next_utt: u64,
    last_at: u64,
}

impl Pipeline {
    pub fn new(policy: Policy) -> Pipeline {
        Pipeline { policy, gap_max: GAP_MAX_SAMPLES, tails: HashMap::new(), rows: HashMap::new(), owners: HashMap::new(), sources: HashMap::new(), src_tracks: HashMap::new(), listed: HashMap::new(), present: HashMap::new(), lines: HashMap::new(), sr: HashMap::new(), open: BTreeMap::new(), next_id: 0, counts: Counts::default(), speaking: HashMap::new(), held: HashMap::new(), ending: BTreeMap::new(), next_utt: 0, last_at: 0 }
    }

    /// 구멍 상한(SDK:RULE-PLAT-REC-GAP — 넘으면 세그먼트를 새로 연다)
    pub fn with_gap_max(mut self, d: std::time::Duration) -> Pipeline {
        self.gap_max = (d.as_millis() as u64) * 48;
        self
    }

    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// 레코드 하나 → 출력 열
    pub fn push(&mut self, r: &Record) -> Vec<Output> {
        self.counts.records += 1;
        self.last_at = self.last_at.max(r.log_time);
        let mut out = Vec::new();
        let due: Vec<String> = self.ending.iter().filter(|(_, e)| r.log_time > e.deadline).map(|(k, _)| k.clone()).collect();
        for room in due {
            self.end_utterance(&room, r.log_time, &mut out);
        }
        match &r.channel {
            Channel::Rtp(ssrc) => self.rtp(*ssrc, r, &mut out),
            Channel::Track => self.track(&r.data, &mut out),
            Channel::Floor => self.floor(&r.data, r.log_time, &mut out),
            Channel::Sr => {
                if let Ok(v) = serde_json::from_slice::<Value>(&r.data)
                    && let (Some(s), Some(n), Some(t)) = (v["ssrc"].as_u64(), v["ntp"].as_u64(), v["rtp"].as_u64())
                {
                    self.sr.insert(s as u32, (n, t as u32));
                }
            }
            Channel::Gap => {
                if let Ok(v) = serde_json::from_slice::<Value>(&r.data)
                    && let Some(s) = v["ssrc"].as_u64()
                {
                    let video = self.rows.get(&(s as u32)).is_some_and(|x| x.kind == "video");
                    let line = self.lines.entry(s as u32).or_insert_with(|| Line { asm: None, need_key: video });
                    if let Some(a) = line.asm.as_mut() {
                        a.broken = true;
                    }
                    if video {
                        line.need_key = true;
                    }
                }
            }
            Channel::Session | Channel::Tick => {}
        }
        out
    }

    /// 끝 — 열린 세그먼트를 닫는다(번호 차례) · 조립 중인 프레임은 끝이 없어 버린다
    pub fn finish(&mut self) -> Vec<Output> {
        let mut out = Vec::new();
        let ending: Vec<String> = self.ending.keys().cloned().collect();
        for room in ending {
            self.end_utterance(&room, self.last_at, &mut out);
        }
        let mut rooms: Vec<String> = self.held.keys().cloned().collect();
        rooms.sort();
        for r in rooms {
            for f in self.held.remove(&r).unwrap_or_default() {
                self.counts.unheld += 1;
                out.push(Output::Orphan(f));
            }
        }
        let mut segs: Vec<Segment> = std::mem::take(&mut self.open).into_values().collect();
        segs.sort_by_key(|s| s.id);
        out.extend(segs.into_iter().map(|mut s| {
            s.event = SegmentKind::Closed;
            s.reason = Some("end");
            Output::Segment(s)
        }));
        out
    }

    fn track(&mut self, data: &[u8], out: &mut Vec<Output>) {
        let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
        let Some(room) = v["room"].as_str() else { return };
        let mut personal: BTreeSet<String> = BTreeSet::new();
        let mut all: BTreeSet<String> = BTreeSet::new();
        for t in v["tracks"].as_array().into_iter().flatten() {
            let Some(tid) = t["track_id"].as_str() else { continue };
            all.insert(tid.to_string());
            self.owners.insert(tid.to_string(), t["user_id"].as_str().map(str::to_string));
            if let (Some(c), Some(u)) = (t["csrc"].as_u64(), t["user_id"].as_str()) {
                self.sources.insert(c as u32, u.to_string());
                self.src_tracks.insert(c as u32, tid.to_string());
            }
            if t["user_id"].as_str().is_some() {
                personal.insert(tid.to_string());
            }
        }
        self.present.insert(room.to_string(), all);
        // 목록에서 빠진 개인 트랙 — 세그먼트를 닫는다(SDK:RULE-PLAT-REC-SPLIT 트랙이 사라짐) · 늦은 프레임은 finish_frame 이 지금 목록으로 거른다
        let before = self.listed.insert(room.to_string(), personal.clone()).unwrap_or_default();
        for tid in before.difference(&personal) {
            self.close(&format!("track/{tid}"), "track", out);
        }
        let mut now: HashMap<u32, Row> = HashMap::new();
        for (k, x) in v["rows"].as_object().into_iter().flatten() {
            let Ok(ssrc) = k.parse::<u32>() else { continue };
            let row = match x {
                Value::String(tid) => Row { room: room.to_string(), track_id: tid.clone(), kind: String::new(), codec: String::new() },
                o => Row {
                    room: room.to_string(),
                    track_id: o["track_id"].as_str().unwrap_or_default().to_string(),
                    kind: o["kind"].as_str().unwrap_or_default().to_string(),
                    codec: o["codec"].as_str().unwrap_or_default().to_string(),
                },
            };
            now.insert(ssrc, row);
        }
        // 그 방의 줄 — 빠진 줄 · 트랙이 바뀐 줄은 조립을 버린다 · ★줄이 바뀐 것만으로는 세그먼트를 닫지 않는다 — 무전 슬롯은 그 트랙을 실은 줄이 없어질 때 닫는다
        let mut gone: Vec<u32> = self.rows.iter().filter(|(k, r)| r.room == room && now.get(k) != Some(r)).map(|(k, _)| *k).collect();
        gone.sort_unstable();
        let carried: HashSet<&str> = now.values().map(|r| r.track_id.as_str()).collect();
        let mut slots: BTreeSet<String> = BTreeSet::new();
        for k in gone {
            if let Some(old) = self.rows.remove(&k) {
                self.lines.remove(&k);
                if self.owners.get(&old.track_id).cloned().flatten().is_none() && !carried.contains(old.track_id.as_str()) {
                    slots.insert(old.track_id);
                }
            }
        }
        for tid in slots {
            self.close(&format!("track/{tid}"), "track", out);
        }
        for (k, r) in now {
            self.rows.entry(k).or_insert(r);
        }
    }

    /// 발언권 사건 — ★발언 경계(SDK:RULE-PLAT-REC-SPLIT 발언) · 출처 번호 → 사용자(식별)
    fn floor(&mut self, data: &[u8], at: u64, out: &mut Vec<Output>) {
        let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
        let Some(room) = v["room"].as_str().map(str::to_string) else { return };
        // ★`granted` = 내 허가(나는 내 발언을 받지 않는다) — 지금 화자 발언은 다음 허가처럼 닫고 · 새 화자는 열지 않는다
        let (taken, opens) = match v["event"].as_str() {
            Some("taken") => (true, true),
            Some("granted") => (true, false),
            Some("idle") => (false, false),
            _ => return,
        };
        let sources: Vec<u32> = v["sources"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|c| c as u32).collect();
        if let Some(u) = v["user_id"].as_str() {
            for c in &sources {
                self.sources.insert(*c, u.to_string());
            }
        }
        if self.policy != Policy::Utterance {
            return;
        }
        let seq = v["seq"].as_u64();
        let talk = v["talk"].as_str().filter(|t| !t.is_empty());
        if opens && let Some(sp) = self.speaking.get_mut(&room).filter(|sp| same_talk(sp, seq, talk)) {
            sp.seq = seq;
            if !sources.is_empty() {
                sp.sources = sources;
            }
            return;
        }
        if let Some(old) = self.speaking.remove(&room) {
            if self.ending.contains_key(&room) {
                self.end_utterance(&room, at, out);
            }
            let open = self.open.values().any(|s| s.room == room && s.owner.is_none());
            if open {
                self.ending.insert(room.clone(), Ending { old, deadline: at.saturating_add(ENDING_GRACE_NS), reason: if taken { "taken" } else { "idle" } });
            }
        }
        if opens {
            let sp = Speaker { seq, sources, utt: self.next_utt, talk: v["talk"].as_str().filter(|t| !t.is_empty()).map(str::to_string) };
            self.next_utt += 1;
            self.speaking.insert(room.clone(), sp);
        }
        if !self.ending.contains_key(&room) {
            self.release_held(&room, at, out);
        }
    }

    /// 끝나는 발언을 닫고(유예가 지났다) 그동안 들고 있던 새 화자 프레임을 낸다
    fn end_utterance(&mut self, room: &str, at: u64, out: &mut Vec<Output>) {
        let Some(e) = self.ending.remove(room) else { return };
        let slots: Vec<String> = self.open.iter().filter(|(_, s)| s.room == room && s.owner.is_none()).map(|(k, _)| k.clone()).collect();
        for k in slots {
            self.close(&k, e.reason, out);
        }
        self.release_held(room, at, out);
    }

    /// 들고 있던 프레임 — 지금 화자의 것이고 1초 안이면 그 발언에 · 다른 사람 것이면 센다 · 화자가 아직 없으면 1초까지 더 든다
    fn release_held(&mut self, room: &str, at: u64, out: &mut Vec<Output>) {
        let held = self.held.remove(room).unwrap_or_default();
        let sp = self.speaking.get(room).cloned();
        for f in held {
            let fresh = at.saturating_sub(f.log_time) <= HOLD_BEFORE_TAKEN_NS;
            match &sp {
                Some(sp) if fresh && sp.owns(f.source) => self.split(f, out),
                Some(_) if fresh => {
                    self.counts.other_speaker += 1;
                    out.push(Output::Orphan(f));
                }
                None if fresh => self.held.entry(room.to_string()).or_default().push(f),
                _ => {
                    self.counts.unheld += 1;
                    out.push(Output::Orphan(f));
                }
            }
        }
    }

    fn rtp(&mut self, ssrc: u32, r: &Record, out: &mut Vec<Output>) {
        self.counts.packets += 1;
        let Some(row) = self.rows.get(&ssrc).cloned() else {
            self.counts.unrouted += 1;
            return;
        };
        let Some(h) = rtp_header(&r.data) else { return };
        let codec = Codec::of(&row.codec);
        let video = row.kind == "video";
        let line = self.lines.entry(ssrc).or_insert_with(|| Line { asm: None, need_key: video });
        let mut done = Vec::new();
        if line.asm.as_ref().is_some_and(|a| a.ts != h.ts)
            && let Some(a) = line.asm.take()
        {
            done.push(a);
        }
        let payload = &r.data[h.start..h.end];
        if !payload.is_empty() {
            let piece = depack::unpack(codec, payload);
            let a = line.asm.get_or_insert_with(|| Asm {
                ts: h.ts,
                data: Vec::new(),
                key: false,
                broken: piece.as_ref().is_some_and(|x| !x.start),
                source: h.csrc,
                log_time: r.log_time,
                seq_first: h.seq,
                seq_last: h.seq,
                rec_first: r.sequence,
                rec_last: r.sequence,
            });
            a.seq_last = h.seq;
            a.rec_last = r.sequence;
            match piece {
                Some(x) => {
                    a.data.extend_from_slice(&x.data);
                    a.key |= x.key;
                }
                None => a.broken = true,
            }
            if (h.marker || !video)
                && let Some(a) = line.asm.take()
            {
                done.push(a);
            }
        }
        for a in done {
            self.finish_frame(ssrc, &row, a, out);
        }
    }

    fn finish_frame(&mut self, ssrc: u32, row: &Row, a: Asm, out: &mut Vec<Output>) {
        let video = row.kind == "video";
        // 분기 — 프레임의 트랙 = 출처 트랙(개인) · 아니면 줄 트랙(SDK:RULE-PLAT-REC-PIPE) · 목록에서 빠진 트랙의 늦은 프레임은 센다(키프레임 대기보다 먼저)
        let track_id = a.source.and_then(|c| self.src_tracks.get(&c).cloned()).unwrap_or_else(|| row.track_id.clone());
        if !listed_now(&self.present, &track_id) {
            self.counts.late += 1;
            return;
        }
        let Some(line) = self.lines.get_mut(&ssrc) else { return };
        if a.broken {
            self.counts.broken += 1;
            if video {
                line.need_key = true;
            }
            return;
        }
        if video {
            if a.key {
                line.need_key = false;
            } else if line.need_key {
                self.counts.before_key += 1;
                return;
            }
        }
        self.counts.frames += 1;
        let clock = u64::from(Codec::of(&row.codec).clock(video));
        let utc_ns = self.sr.get(&ssrc).map(|(ntp, rtp)| {
            let base = ((ntp >> 32).saturating_sub(NTP_UNIX)) * 1_000_000_000 + (((ntp & 0xffff_ffff) * 1_000_000_000) >> 32);
            let d = i64::from(a.ts.wrapping_sub(*rtp) as i32) * 1_000_000_000 / clock as i64;
            (base as i64 + d).max(0) as u64
        });
        let frame = Frame {
            segment: None,
            room: row.room.clone(),
            owner: self.owners.get(&track_id).cloned().flatten(),
            track_id,
            kind: row.kind.clone(),
            codec: row.codec.clone(),
            ssrc,
            source: a.source,
            user_id: a.source.and_then(|c| self.sources.get(&c).cloned()),
            keyframe: a.key,
            rtp_ts: a.ts,
            utc_ns,
            log_time: a.log_time,
            seq_first: a.seq_first,
            seq_last: a.seq_last,
            rec_first: a.rec_first,
            rec_last: a.rec_last,
            data: a.data,
            filled: false,
        };
        self.split(frame, out);
    }

    fn split(&mut self, mut f: Frame, out: &mut Vec<Output>) {
        // ★열쇠 = 트랙 — 두 정책 같다(SDK:RULE-PLAT-REC-SPLIT)
        let key = format!("track/{}", f.track_id);
        match self.policy {
            Policy::Utterance => {
                if f.source.is_none() {
                    out.push(Output::Orphan(f));
                    return;
                }
                if f.owner.is_none() && let Some(e) = self.ending.get(&f.room) {
                    if !(e.old.owns(f.source) && f.log_time <= e.deadline) {
                        self.held.entry(f.room.clone()).or_default().push(f);
                        return;
                    }
                } else if f.owner.is_none() {
                    match self.speaking.get(&f.room) {
                        Some(sp) if !sp.owns(f.source) => {
                            self.counts.other_speaker += 1;
                            out.push(Output::Orphan(f));
                            return;
                        }
                        Some(_) => {}
                        None => {
                            let room = f.room.clone();
                            let now = f.log_time;
                            let held = self.held.entry(room).or_default();
                            let keep: Vec<Frame> = std::mem::take(held);
                            for h in keep {
                                if now.saturating_sub(h.log_time) > HOLD_BEFORE_TAKEN_NS {
                                    self.counts.unheld += 1;
                                    out.push(Output::Orphan(h));
                                } else {
                                    held.push(h);
                                }
                            }
                            held.push(f);
                            return;
                        }
                    }
                }
            }
            Policy::Span { time_ns, bytes } => {
                let cut = self.open.get(&key).is_some_and(|s| f.log_time.saturating_sub(s.first_log_time) >= time_ns || s.bytes >= bytes);
                if cut && (f.kind != "video" || f.keyframe) {
                    self.close(&key, "span", out);
                }
            }
        }
        if !self.open.contains_key(&key) {
            // ★영상 세그먼트는 키프레임에서만 연다(SDK:RULE-PLAT-REC-SPLIT)
            if f.kind == "video" && !f.keyframe {
                self.counts.before_key += 1;
                return;
            }
            let s = Segment {
                event: SegmentKind::Opened,
                id: self.next_id,
                room: f.room.clone(),
                track_id: f.track_id.clone(),
                owner: f.owner.clone(),
                kind: f.kind.clone(),
                codec: f.codec.clone(),
                source: f.source,
                user_id: f.user_id.clone().or_else(|| f.owner.clone()),
                first_utc_ns: f.utc_ns,
                last_utc_ns: f.utc_ns,
                first_log_time: f.log_time,
                last_log_time: f.log_time,
                frames: 0,
                bytes: 0,
                filled: 0,
                reason: None,
                utterance: if self.policy == Policy::Utterance && f.owner.is_none() { self.speaking.get(&f.room).map(|sp| sp.utt) } else { None },
                talk: if self.policy == Policy::Utterance && f.owner.is_none() { self.speaking.get(&f.room).and_then(|sp| sp.talk.clone()) } else { None },
            };
            self.tails.remove(&key);
            self.next_id += 1;
            out.push(Output::Segment(s.clone()));
            self.open.insert(key.clone(), s);
        }
        // 음성 구멍 — 같은 세그먼트 안에서 앞 프레임 끝보다 늦게 시작하면 PLC 로 메운다 · 상한을 넘으면 세그먼트를 새로 연다(SDK:RULE-PLAT-REC-GAP)
        if f.kind == "audio" && f.codec.eq_ignore_ascii_case("opus") && !f.data.is_empty() {
            if let Some(t) = self.tails.get(&key).copied() {
                let hole = if t.ssrc == f.ssrc { i64::from(f.rtp_ts.wrapping_sub(t.end_ts) as i32) } else { moved_hole(t.end_log, f.log_time) as i64 };
                if hole > 0 && hole as u64 > self.gap_max {
                    self.close(&key, "gap", out);
                    return self.split(f, out);
                }
                if hole > 0 {
                    self.fill(&key, &f, t, hole as u64, out);
                }
            }
            let len = opus_samples(&f.data);
            let carry = self.tails.get(&key).map(|t| t.carry).unwrap_or(0);
            self.tails.insert(key.clone(), Tail { end_ts: f.rtp_ts.wrapping_add(len as u32), toc: f.data[0], carry, utc_end: f.utc_ns.map(|u| u + len * 1_000_000_000 / 48_000), ssrc: f.ssrc, end_log: f.log_time + len * 1_000_000_000 / 48_000 });
        }
        if let Some(s) = self.open.get_mut(&key) {
            s.frames += 1;
            s.bytes += f.data.len() as u64;
            s.last_log_time = f.log_time;
            if f.utc_ns.is_some() {
                s.last_utc_ns = f.utc_ns;
                s.first_utc_ns = s.first_utc_ns.or(f.utc_ns);
            }
            if s.user_id.is_none() {
                s.user_id = f.user_id.clone();
            }
            f.segment = Some(s.id);
        }
        out.push(Output::Frame(f));
    }

    /// 구멍 하나를 PLC 패킷으로(앞 TOC 의 config · code 0 · 빈 프레임) — 개수 = (구멍 + 넘긴 나머지) ÷ 앞 프레임 길이 · 나머지는 넘긴다
    fn fill(&mut self, key: &str, f: &Frame, t: Tail, hole: u64, out: &mut Vec<Output>) {
        let toc = t.toc & 0xFC;
        let per = opus_samples(&[toc]).max(1);
        let total = hole + t.carry;
        let n = total / per;
        if let Some(x) = self.tails.get_mut(key) {
            x.carry = total % per;
        }
        let Some(seg) = self.open.get_mut(key) else { return };
        let moved = t.ssrc != f.ssrc;
        for i in 0..n {
            let ts = t.end_ts.wrapping_add((i * per) as u32);
            seg.frames += 1;
            seg.filled += 1;
            seg.bytes += 1;
            out.push(Output::Frame(Frame {
                segment: Some(seg.id),
                keyframe: true,
                rtp_ts: ts,
                utc_ns: t.utc_end.map(|u| u + i * per * 1_000_000_000 / 48_000),
                seq_first: f.seq_first,
                seq_last: f.seq_first,
                rec_first: f.rec_first,
                rec_last: f.rec_first,
                data: vec![toc],
                filled: true,
                ssrc: if moved { t.ssrc } else { f.ssrc },
                log_time: if moved { t.end_log + i * per * 1_000_000_000 / 48_000 } else { f.log_time },
                ..f.clone()
            }));
        }
        self.counts.filled += n;
    }

    fn close(&mut self, key: &str, why: &'static str, out: &mut Vec<Output>) {
        if let Some(mut s) = self.open.remove(key) {
            s.event = SegmentKind::Closed;
            s.reason = Some(why);
            out.push(Output::Segment(s));
        }
    }
}

/// 프레임의 트랙이 지금 어느 방 목록에든 있나 — 없으면 목록에서 빠진 트랙의 늦은 프레임(SDK:RULE-PLAT-REC-SPLIT).
/// ★지금 스냅숏의 순수 함수다 — 앞 목록과의 차이(이력)로 가르면 파트 머리 스냅숏만으로 읽는 파트 k 가 같은 패킷을 프레임으로 낸다(202611/20261119c D17).
/// 방을 가리지 않는다 — 출처 트랙(CSRC)이 다른 방 목록에 있는 청취방 화자는 늦지 않다.
pub(crate) fn listed_now(present: &HashMap<String, BTreeSet<String>>, track_id: &str) -> bool {
    present.values().any(|s| s.contains(track_id))
}

fn moved_hole(end_log: u64, at: u64) -> u64 {
    (u128::from(at.saturating_sub(end_log)) * 48_000 / 1_000_000_000) as u64
}

fn same_talk(sp: &Speaker, seq: Option<u64>, talk: Option<&str>) -> bool {
    match (talk, sp.talk.as_deref()) {
        (Some(a), Some(b)) => a == b,
        _ => seq.is_some() && sp.seq == seq,
    }
}

/// Opus 패킷 하나의 길이(48kHz 표본) — TOC 의 config · 프레임 수(RFC 6716 §3.1)
pub fn opus_samples(p: &[u8]) -> u64 {
    let Some(&toc) = p.first() else { return 0 };
    let config = toc >> 3;
    let per = match config {
        0..=11 => [480, 960, 1920, 2880][usize::from(config % 4)],
        12..=15 => [480, 960][usize::from(config % 2)],
        _ => [120, 240, 480, 960][usize::from(config % 4)],
    };
    let frames = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        _ => p.get(1).map(|b| u64::from(b & 0x3f)).unwrap_or(0),
    };
    per * frames
}
