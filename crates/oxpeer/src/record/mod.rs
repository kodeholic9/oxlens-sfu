// author: kodeholic (powered by Claude)
//! 녹음 원시 기록(SDK:RULE-PLAT-REC-TAP · REC-RAW · REC-CLOCK · REC-PART · context 202611/20261109c) — 정리층 출구의 RTP 원본과 그 시점의 제어 사건을
//! 받은 차례대로 MCAP 파트에 적고(켰으면) 라이브 통(지점 ①)에 나눈다. 그 위에 파이프라인(분기 · 조립 · 시간축 · 분할 — REC-PIPE · REC-SPLIT)이
//! 프레임(지점 ②) · 세그먼트 사건(지점 ③)을 내고, 내장 결과물(`OggSink`)도 그 둘만 쓴다(REC-API).
//!
//! 채취는 직접 소유다 — 정리층이 `Out::Raw` · `Out::Gap` 을 내면 `deliver` 가 `MediaSink` 가 쥔 기록기를 부르고, core 는 사건을 낼 때 같은 기록기를 부른다.
//! `sequence` · `log_time` 은 기록기 잠금 안 한 곳에서 매긴다(REC-CLOCK) — 그래서 파일 차례로 둘 다 오른다.

mod file;
mod hls;
mod live;
mod mkv;
mod ogg;
mod pipeline;

pub use file::{FileSource, PartScan, scan_part};
pub(crate) use live::Shared;
pub use live::Records;
pub use hls::{HlsConfig, HlsSegment, HlsSink, HlsStream};
pub use mkv::{MkvConfig, MkvFile, MkvSink, MkvTrack};
pub use ogg::{OggFile, OggSink, opus_samples};
pub use pipeline::{Frame, Output, Pipeline, Policy, Segment, SegmentKind};

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mcap::records::{MessageHeader, Metadata};
use mcap::{WriteOptions, Writer};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::api::Event;

/// 원시 포맷 판(SDK:RULE-PLAT-REC-RAW `oxlens.recording` `format`)
pub const FORMAT: u32 = 1;

/// 청크 압축(실측 M9 — 기본은 없음)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compression {
    #[default]
    None,
    #[cfg(feature = "record-lz4")]
    Lz4,
    #[cfg(feature = "record-zstd")]
    Zstd,
}

/// 녹음 설정(SDK:RULE-PLAT-REC-PART 손잡이)
#[derive(Clone, Debug)]
pub struct RecordConfig {
    /// 파트를 둘 디렉터리 — 없으면 원시 기록 없이 라이브 통만(SDK:RULE-PLAT-REC-API 「원시 기록을 끈 구성」)
    pub dir: Option<PathBuf>,
    /// 녹음 세션 ID — 파트 이름 `<recording_id>-<파트 4자리>.mcap`
    pub recording_id: String,
    /// 파트 크기 S · 시간 T — 먼저 닿는 쪽에서 회전
    pub part_bytes: u64,
    pub part_time: Duration,
    /// 청크를 닫는 주기 F = 비정상 종료 손실 상한 — `tick` 경계에서 닫는다
    pub flush: Duration,
    /// `tick` 주기
    pub tick: Duration,
    /// 이 밑이면 기록을 멈추고 사건
    pub min_free_bytes: u64,
    /// 청크를 닫을 때마다 fsync(파트를 닫을 때는 늘 한다)
    pub fsync_on_flush: bool,
    pub compression: Compression,
}

impl RecordConfig {
    pub fn new(dir: impl Into<PathBuf>, recording_id: impl Into<String>) -> RecordConfig {
        RecordConfig { dir: Some(dir.into()), ..RecordConfig::live(recording_id) }
    }

    /// 원시 파트 없이 라이브 통만
    pub fn live(recording_id: impl Into<String>) -> RecordConfig {
        RecordConfig {
            dir: None,
            recording_id: recording_id.into(),
            part_bytes: 1 << 30,
            part_time: Duration::from_secs(3_600),
            flush: Duration::from_secs(2),
            tick: Duration::from_millis(500),
            min_free_bytes: 512 << 20,
            fsync_on_flush: false,
            compression: Compression::None,
        }
    }
}

/// 채널(SDK:RULE-PLAT-REC-RAW)
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    /// `rtp/<줄 SSRC 10진>`
    Rtp(u32),
    Track,
    Floor,
    Session,
    Sr,
    Gap,
    Tick,
}

impl Channel {
    pub fn topic(&self) -> String {
        match self {
            Channel::Rtp(s) => format!("rtp/{s}"),
            Channel::Track => "ctl/track".into(),
            Channel::Floor => "ctl/floor".into(),
            Channel::Session => "ctl/session".into(),
            Channel::Sr => "clock/sr".into(),
            Channel::Gap => "gap".into(),
            Channel::Tick => "tick".into(),
        }
    }

    pub fn parse(topic: &str) -> Option<Channel> {
        Some(match topic {
            "ctl/track" => Channel::Track,
            "ctl/floor" => Channel::Floor,
            "ctl/session" => Channel::Session,
            "clock/sr" => Channel::Sr,
            "gap" => Channel::Gap,
            "tick" => Channel::Tick,
            t => Channel::Rtp(t.strip_prefix("rtp/")?.parse().ok()?),
        })
    }

    fn encoding(&self) -> &'static str {
        match self {
            Channel::Rtp(_) => "oxlens.rtp",
            _ => "json",
        }
    }
}

/// 레코드 하나(공개 지점 ① — 소유한 바이트 · 평범한 값)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub sequence: u32,
    /// UTC ns(REC-CLOCK — 세션 닻 + 단조 경과)
    pub log_time: u64,
    pub channel: Channel,
    pub data: Vec<u8>,
}

/// 기록기 — `MediaSink` · core 가 같이 쥔다
pub(crate) struct Recorder {
    inner: Mutex<Inner>,
}

struct Inner {
    cfg: RecordConfig,
    anchor_ns: u64,
    mono0: Instant,
    seq: u32,
    part_no: u32,
    prev_last: Option<u32>,
    part: Option<Part>,
    last_flush: u64,
    /// 원시 기록이 멈췄다(디스크 하한) — 라이브 통은 계속 받는다
    stopped: bool,
    /// 닫혔다 — 그 뒤 레코드는 받지 않는다(tick 이 새 파트를 열지 않게)
    closed: bool,
    snap: Snap,
    subs: Vec<Sub>,
    /// 꼬리 읽기로 갈아탄 통 — 기록기가 닫힐 때 알린다
    tailing: Vec<Arc<Shared>>,
    /// 꼬리 읽기로 갈아탄 통 누계(운영 `stat` `record_degraded`)
    degraded: u64,
    ev: mpsc::UnboundedSender<Event>,
    log: Arc<crate::log::Logger>,
}

struct Part {
    w: Writer<BufWriter<File>>,
    /// fsync 몫 — 쓰기 손잡이와 같은 파일
    file: File,
    channels: HashMap<Channel, u16>,
    start: u64,
    bytes: u64,
    last_seq: Option<u32>,
}

/// 파트 머리 스냅숏 재료(REC-PART) — 방마다 마지막 트랙 목록 · 발언권 · 마지막 세션 · 연결 사건 · 줄마다 마지막 SR
#[derive(Default)]
struct Snap {
    track: BTreeMap<String, Record>,
    floor: BTreeMap<String, Record>,
    session: BTreeMap<String, Record>,
    sr: BTreeMap<u32, Record>,
}

impl Snap {
    fn all(&self) -> Vec<&Record> {
        self.track.values().chain(self.floor.values()).chain(self.session.values()).chain(self.sr.values()).collect()
    }
}

/// 녹음 계수(운영 로그 `stat`)
#[derive(Default)]
pub(crate) struct RecStat {
    pub parts: u32,
    pub subscribers: usize,
    pub degraded: u64,
    pub stopped: bool,
    pub disk_free: Option<u64>,
}

/// 라이브 통 하나(지점 ① — SDK:RULE-PLAT-REC-API)
struct Sub {
    tx: mpsc::Sender<Record>,
    shared: Arc<Shared>,
}

impl Recorder {
    /// 만들기 — 원시 기록을 켰으면 첫 파트는 첫 레코드에서 연다 · `tick` 태스크를 띄운다(기록기가 사라지면 끝난다)
    pub(crate) fn start(cfg: RecordConfig, ev: mpsc::UnboundedSender<Event>, log: Arc<crate::log::Logger>) -> std::io::Result<Arc<Recorder>> {
        if let Some(d) = &cfg.dir {
            fs::create_dir_all(d)?;
        }
        let anchor_ns = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        let tick = cfg.tick;
        let r = Arc::new(Recorder { inner: Mutex::new(Inner { cfg, anchor_ns, mono0: Instant::now(), seq: 0, part_no: 0, prev_last: None, part: None, last_flush: anchor_ns, stopped: false, closed: false, snap: Snap::default(), subs: Vec::new(), tailing: Vec::new(), degraded: 0, ev, log }) });
        let weak: Weak<Recorder> = Arc::downgrade(&r);
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(tick);
            iv.tick().await;
            loop {
                iv.tick().await;
                let Some(r) = weak.upgrade() else { break };
                r.push(Channel::Tick, b"{}".to_vec());
            }
        });
        Ok(r)
    }

    /// 레코드 하나 — 순번 · 시각을 여기서 매긴다(REC-CLOCK)
    pub(crate) fn push(&self, ch: Channel, data: Vec<u8>) {
        let Ok(mut g) = self.inner.lock() else { return };
        g.push(ch, data);
    }

    /// 트랙 목록 — 방마다 앞과 같으면 적지 않는다(줄 재배정 · 목록 바뀜만 남긴다)
    pub(crate) fn track(&self, room: &str, body: Value) {
        let Ok(mut g) = self.inner.lock() else { return };
        let data = body.to_string().into_bytes();
        if g.snap.track.get(room).is_some_and(|r| r.data == data) {
            return;
        }
        g.push(Channel::Track, data);
    }

    /// 라이브 통을 붙인다 — ★지금 해석 상태(트랙 · 발언권 · 세션 · SR 의 마지막 레코드 — 원래 순번 · 시각 그대로)를 먼저 넣고 그 뒤 레코드 전량.
    /// 늦게 연 통도 줄 → 트랙 · 출처를 안다(파트 머리 스냅숏과 같은 생각 · SDK:RULE-PLAT-REC-API)
    pub(crate) fn subscribe(&self, tx: mpsc::Sender<Record>, shared: Arc<Shared>) {
        let Ok(mut g) = self.inner.lock() else { return };
        let mut head: Vec<Record> = g.snap.all().into_iter().cloned().collect();
        head.sort_by_key(|r| r.sequence);
        for r in head {
            let _ = tx.try_send(r);
        }
        if let Ok(mut st) = shared.state.lock() {
            st.raw = g.cfg.dir.clone().map(|d| (d, g.cfg.recording_id.clone()));
            st.tick = g.cfg.tick;
        }
        g.subs.push(Sub { tx, shared });
    }

    /// 운영 로그 프로세스 줄의 녹음 계수
    pub(crate) fn stat(&self) -> RecStat {
        let Ok(g) = self.inner.lock() else { return RecStat::default() };
        RecStat { parts: g.part_no, subscribers: g.subs.len() + g.tailing.len(), degraded: g.degraded, stopped: g.stopped, disk_free: g.cfg.dir.as_deref().and_then(free_bytes) }
    }

    pub(crate) fn close(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.closed = true;
            g.close_part("close");
            let mut all: Vec<Arc<Shared>> = g.subs.drain(..).map(|s| s.shared).collect();
            all.append(&mut g.tailing);
            for s in all {
                if let Ok(mut st) = s.state.lock() {
                    st.closed = true;
                }
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.close();
    }
}

impl Inner {
    fn now(&self) -> u64 {
        self.anchor_ns.saturating_add(self.mono0.elapsed().as_nanos() as u64)
    }

    fn push(&mut self, ch: Channel, data: Vec<u8>) {
        if self.closed {
            return;
        }
        let t = self.now();
        if self.cfg.dir.is_some() && !self.stopped {
            if let Some(p) = &self.part
                && (p.bytes >= self.cfg.part_bytes || t.saturating_sub(p.start) >= self.cfg.part_time.as_nanos() as u64)
            {
                let why = if p.bytes >= self.cfg.part_bytes { "size" } else { "time" };
                self.close_part(why);
            }
            if self.part.is_none() {
                self.open_part(t);
            }
        }
        self.write(ch.clone(), data, t);
        // 청크 닫기는 tick 경계 — 문턱을 반 tick 앞당긴다(tick 이 조금 일찍 와도 한 주기 밀리지 않게 · 정:IMPL-ROOM-STALL 과 같은 꼴)
        let due = (self.cfg.flush.as_nanos() as u64).saturating_sub(self.cfg.tick.as_nanos() as u64 / 2);
        if ch == Channel::Tick && t.saturating_sub(self.last_flush) >= due {
            self.last_flush = t;
            self.flush();
        }
    }

    fn remember(&mut self, r: &Record) {
        if matches!(r.channel, Channel::Rtp(_) | Channel::Gap | Channel::Tick) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&r.data) else { return };
        let key = |k: &str| v.get(k).and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_u64().map(|n| n.to_string())));
        match r.channel {
            Channel::Track => {
                if let Some(room) = key("room") {
                    self.snap.track.insert(room, r.clone());
                }
            }
            Channel::Floor => {
                if let Some(room) = key("room") {
                    self.snap.floor.insert(room, r.clone());
                }
            }
            Channel::Session => {
                if let Some(e) = key("event") {
                    let at = key("server").or_else(|| key("room")).unwrap_or_default();
                    self.snap.session.insert(format!("{e}/{at}"), r.clone());
                }
            }
            Channel::Sr => {
                if let Some(s) = key("ssrc").and_then(|s| s.parse().ok()) {
                    self.snap.sr.insert(s, r.clone());
                }
            }
            _ => {}
        }
    }

    /// 순번을 매겨 파트에 쓰고(켰으면) 라이브 통에 나눈다
    fn write(&mut self, ch: Channel, data: Vec<u8>, t: u64) {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        if let Some(p) = self.part.as_mut() {
            let id = match p.channels.get(&ch) {
                Some(id) => Some(*id),
                None => p.w.add_channel(0, &ch.topic(), ch.encoding(), &BTreeMap::new()).ok().inspect(|id| {
                    p.channels.insert(ch.clone(), *id);
                }),
            };
            if let Some(id) = id {
                let h = MessageHeader { channel_id: id, sequence: seq, log_time: t, publish_time: t };
                if p.w.write_to_known_channel(&h, &data).is_ok() {
                    p.bytes += 31 + data.len() as u64;
                    p.last_seq = Some(seq);
                }
            }
        }
        let r = Record { sequence: seq, log_time: t, channel: ch, data };
        self.remember(&r);
        self.tap(r);
    }

    /// 라이브 통마다 — ★넘치면 버리지 않는다: 원시 파트가 있으면 그 통은 넘친 첫 순번부터 꼬리 읽기로 갈아탄다 · 없으면 멈추고 사건(SDK:RULE-PLAT-REC-API 역압)
    fn tap(&mut self, r: Record) {
        if self.subs.is_empty() {
            return;
        }
        let tail_ok = self.cfg.dir.is_some() && !self.stopped && self.part.is_some();
        let part_now = self.part_no.saturating_sub(1);
        let ev = self.ev.clone();
        let log = self.log.clone();
        let mut tailing = Vec::new();
        self.subs.retain(|s| match s.tx.try_send(r.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                if let Ok(mut st) = s.shared.state.lock() {
                    if tail_ok {
                        st.degraded = Some((r.sequence, part_now));
                        tailing.push(s.shared.clone());
                        log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-REC-API").key("in.from_sequence", r.sequence).key("in.part", part_now).msg("live record consumer overflowed - tail read from raw part"));
                    } else {
                        st.stopped = true;
                        let _ = ev.send(Event::RecordConsumerStopped { reason: "overflow".into() });
                        log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-REC-API").key("in.from_sequence", r.sequence).msg("live record consumer overflowed without raw parts - stopped"));
                    }
                }
                false
            }
        });
        self.degraded += tailing.len() as u64;
        self.tailing.extend(tailing);
    }

    fn open_part(&mut self, t: u64) -> bool {
        let Some(dir) = self.cfg.dir.clone() else { return false };
        if free_bytes(&dir).is_some_and(|f| f < self.cfg.min_free_bytes) {
            self.stop();
            return false;
        }
        let path = dir.join(format!("{}-{:04}.mcap", self.cfg.recording_id, self.part_no));
        let Some((f, file)) = File::create(&path).ok().and_then(|f| f.try_clone().ok().map(|c| (f, c))) else {
            self.stop();
            return false;
        };
        let opts = WriteOptions::new().use_chunks(true).chunk_size(None).compression(compression(self.cfg.compression)).profile("oxlens");
        let Ok(mut w) = opts.create(BufWriter::new(f)) else {
            self.stop();
            return false;
        };
        let mut rec = BTreeMap::new();
        rec.insert("format".to_string(), FORMAT.to_string());
        rec.insert("recording_id".to_string(), self.cfg.recording_id.clone());
        rec.insert("sdk".to_string(), format!("oxpeer {}", env!("CARGO_PKG_VERSION")));
        let mut part = BTreeMap::new();
        part.insert("part".to_string(), self.part_no.to_string());
        if let Some(l) = self.prev_last {
            part.insert("prev_last_sequence".to_string(), l.to_string());
        }
        if w.write_metadata(&Metadata { name: "oxlens.recording".into(), metadata: rec }).is_err() || w.write_metadata(&Metadata { name: "oxlens.part".into(), metadata: part }).is_err() {
            self.stop();
            return false;
        }
        self.part = Some(Part { w, file, channels: HashMap::new(), start: t, bytes: 0, last_seq: None });
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-REC-PART").key("in.part", self.part_no).key("in.prev_last_sequence", self.prev_last.map(|x| x.to_string()).unwrap_or_default()).msg("raw part opened"));
        self.part_no += 1;
        // 파트 머리 스냅숏(REC-PART) — 트랙 · 발언권 · 세션 · SR
        let head: Vec<(Channel, Vec<u8>)> = self.snap.all().into_iter().map(|r| (r.channel.clone(), r.data.clone())).collect();
        for (ch, d) in head {
            self.write(ch, d, t);
        }
        true
    }

    fn flush(&mut self) {
        let Some(p) = self.part.as_mut() else { return };
        let _ = p.w.flush();
        if self.cfg.fsync_on_flush {
            let _ = p.file.sync_data();
        }
        if self.cfg.dir.as_deref().and_then(free_bytes).is_some_and(|f| f < self.cfg.min_free_bytes) {
            self.stop();
        }
    }

    fn stop(&mut self) {
        self.close_part("disk_low");
        self.stopped = true;
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).level(crate::log::Level::Warn).spec("SD-RULE-PLAT-REC-PART").key("in.min_free_bytes", self.cfg.min_free_bytes).msg("raw recording stopped - disk low or part not writable"));
        let _ = self.ev.send(Event::RecordStopped { reason: "disk_low".into() });
    }

    fn close_part(&mut self, reason: &str) {
        let Some(mut p) = self.part.take() else { return };
        let mut end = BTreeMap::new();
        if let Some(l) = p.last_seq {
            end.insert("last_sequence".to_string(), l.to_string());
        }
        end.insert("reason".to_string(), reason.to_string());
        let _ = p.w.write_metadata(&Metadata { name: "oxlens.part_end".into(), metadata: end });
        let _ = p.w.finish();
        let mut bw = p.w.into_inner();
        let _ = bw.flush();
        let _ = bw.get_ref().sync_all();
        self.prev_last = p.last_seq.or(self.prev_last);
        self.log.emit(crate::log::Line::new(crate::log::Kind::Diag).spec("SD-RULE-PLAT-REC-PART").key("in.reason", reason).key("in.last_sequence", p.last_seq.map(|x| x.to_string()).unwrap_or_default()).key("in.bytes", p.bytes).msg("raw part closed"));
    }
}

fn compression(c: Compression) -> Option<mcap::Compression> {
    match c {
        Compression::None => None,
        #[cfg(feature = "record-lz4")]
        Compression::Lz4 => Some(mcap::Compression::Lz4),
        #[cfg(feature = "record-zstd")]
        Compression::Zstd => Some(mcap::Compression::Zstd),
    }
}

/// 남은 공간(bytes) — 유닉스 계열만(statvfs)
fn free_bytes(dir: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(dir).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

/// 제어 사건 본문 — `ctl/session`(REC-RAW)
pub(crate) fn session_body(e: &Event) -> Option<Value> {
    Some(match e {
        Event::Session(s) => json!({ "event": "session", "state": format!("{:?}", s.state), "recovering": s.recovering, "reason": s.reason }),
        Event::Link(l) => json!({ "event": "link", "server": l.server, "state": format!("{:?}", l.state) }),
        Event::Joined { room } => json!({ "event": "joined", "room": room }),
        Event::JoinFailed { room, error } => json!({ "event": "join_failed", "room": room, "error": error.to_string() }),
        Event::Left { room } => json!({ "event": "left", "room": room }),
        Event::Forced { room, cause } => json!({ "event": "forced", "room": room, "cause": cause }),
        Event::Rebuilt => json!({ "event": "rebuilt" }),
        Event::Closed { code, name, retryable } => json!({ "event": "closed", "code": code, "name": name, "retryable": retryable }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
