// author: kodeholic (powered by Claude)
//! 녹음 유닛 결과물 — Ogg Opus(RFC 7845 · RFC 3533 · 정:IMPL-OPS-REC-UNIT 「결과물」 `talk`).
//! ★oxpeer 녹음 파이프라인의 프레임(지점 ②)과 세그먼트 사건(지점 ③)만 쓴다 — 세그먼트 하나 = 파일 하나. 받은 Opus 패킷을 그대로 담는다(다시 인코딩하지 않는다).
//! ★결정론(SDK:RULE-PLAT-REC-PIPE — 같은 레코드열이면 같은 출력열) — 파일 이름 · 일련번호는 세그먼트에서 셈하고 벽시계를 넣지 않는다. 그래뉼 위치는 TOC 가 말하는 길이(RFC 6716 §3.1).

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use oxpeer::record::{Frame, Output, SegmentKind, opus_samples};

/// 닫힌 Ogg 파일 하나
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OggFile {
    pub path: PathBuf,
    pub segment: u64,
    pub track_id: String,
    pub user_id: Option<String>,
    pub source: Option<u32>,
    pub packets: u64,
    /// 48kHz 표본 수(마지막 그래뉼)
    pub samples: u64,
    pub first_seq: Option<u16>,
    pub last_seq: Option<u16>,
    /// 첫 프레임 RTP 시각 · 마지막 프레임 끝(RTP 시각 + 길이) — 메운 뒤 `samples` 와 이 폭이 같아야 한다(SDK:RULE-PLAT-REC-GAP)
    pub first_ts: Option<u32>,
    pub end_ts: Option<u32>,
    /// 메운 PLC 프레임 수
    pub filled: u64,
    /// 닫힌 까닭(세그먼트 사건)
    pub reason: Option<String>,
    /// 발언 id(세그먼트의 `talk` — 서버가 지은 발언 기록 `msg_id`)
    pub talk: Option<String>,
}

struct Open {
    w: Option<OggOpus>,
    file: OggFile,
    name: PathBuf,
    stalled: Option<Stall>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OggConfig {
    pub min_free_bytes: u64,
}

impl Default for OggConfig {
    fn default() -> OggConfig {
        OggConfig { min_free_bytes: MIN_FREE_BYTES }
    }
}

/// 세그먼트마다 Ogg Opus 하나(음성 · opus 세그먼트만 · 나머지는 지나친다)
pub struct OggSink {
    dir: PathBuf,
    open: BTreeMap<u64, Open>,
    done: Vec<OggFile>,
    talks: std::collections::BTreeSet<String>,
    cfg: OggConfig,
    disk: Disk,
    dropped: u64,
    ended: bool,
}

impl OggSink {
    pub fn new(dir: impl Into<PathBuf>) -> io::Result<OggSink> {
        OggSink::with(dir, OggConfig::default())
    }

    pub fn with(dir: impl Into<PathBuf>, cfg: OggConfig) -> io::Result<OggSink> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(OggSink { dir, open: BTreeMap::new(), done: Vec::new(), talks: std::collections::BTreeSet::new(), cfg, disk: Disk::default(), dropped: 0, ended: false })
    }

    /// 파이프라인 출력 하나
    pub fn write(&mut self, o: &Output) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        match o {
            Output::Segment(s) if s.event == SegmentKind::Opened => {
                if s.kind != "audio" || !s.codec.eq_ignore_ascii_case("opus") {
                    return Ok(());
                }
                let who = s.user_id.clone().unwrap_or_else(|| s.source.map(|c| format!("csrc{c}")).unwrap_or_else(|| "none".into()));
                // 발언 id 가 있으면 `{방}-{발언 id}-{화자}.ogg`(SDK:RULE-PLAT-REC-SPLIT 발언 번호 · 무전 슬롯 mkv 와 짝) · 같은 발언의 둘째 세그먼트(구멍 상한)는 세그먼트 번호를 덧붙인다
                let name = match &s.talk {
                    Some(t) if self.talks.insert(t.clone()) => format!("{}-{}-{}.ogg", safe(&s.room), safe(t), safe(&who)),
                    Some(t) => format!("{}-{}-{}-{:04}.ogg", safe(&s.room), safe(t), safe(&who), s.id),
                    None => format!("{}-{:04}-{}.ogg", safe(&s.room), s.id, safe(&who)),
                };
                let path = self.dir.join(name);
                let file = OggFile { path: path.clone(), segment: s.id, track_id: s.track_id.clone(), user_id: s.user_id.clone(), source: s.source, packets: 0, samples: 0, first_seq: None, last_seq: None, first_ts: None, end_ts: None, filled: 0, reason: None, talk: s.talk.clone() };
                self.open.insert(s.id, Open { w: None, file, name: path, stalled: None });
            }
            Output::Segment(s) => {
                if let Some(mut x) = self.open.remove(&s.id) {
                    if x.file.user_id.is_none() {
                        x.file.user_id.clone_from(&s.user_id);
                    }
                    if let Some(w) = x.w.take() {
                        let (file, lost) = close_ogg(w, x.file, s.reason);
                        self.dropped += lost;
                        self.done.push(file);
                    }
                }
            }
            Output::Frame(f) => self.frame(f)?,
            Output::Orphan(_) => {}
        }
        Ok(())
    }

    fn frame(&mut self, f: &Frame) -> io::Result<()> {
        let Some(id) = f.segment else { return Ok(()) };
        let Some(x) = self.open.get_mut(&id) else { return Ok(()) };
        if x.w.is_none() {
            match gate(x.stalled, f.log_time, || super::free_bytes(&self.dir), self.cfg.min_free_bytes) {
                Gate::Open => {}
                Gate::Wait => {
                    self.dropped += 1;
                    return Ok(());
                }
                Gate::Recheck => {
                    x.stalled = Some(Stall { at: f.log_time, space: true });
                    self.dropped += 1;
                    return Ok(());
                }
            }
            match OggOpus::create(&x.name, id as u32, &mut self.disk) {
                Ok((w, path)) => {
                    x.file = fresh(&x.file, path);
                    x.w = Some(w);
                    x.stalled = None;
                }
                Err(e) => {
                    let space = rotates(e.kind(), false);
                    x.stalled = Some(Stall { at: f.log_time, space });
                    self.dropped += 1;
                    return if space { Ok(()) } else { Err(e) };
                }
            }
        }
        let Some(w) = x.w.as_mut() else { return Ok(()) };
        match w.push(&f.data, Meta::of(f)) {
            Ok(written) => {
                if let Some(m) = written {
                    count(&mut x.file, m);
                }
            }
            Err(e) => {
                self.dropped += 1;
                if let Some(w) = x.w.take() {
                    let (file, lost) = close_ogg(w, x.file.clone(), Some("disk_low"));
                    self.dropped += lost;
                    self.done.push(file);
                }
                x.stalled = Some(Stall { at: f.log_time, space: rotates(e.kind(), false) });
            }
        }
        Ok(())
    }

    pub fn close(&mut self, reason: Option<&str>) -> io::Result<()> {
        let reason = reason.unwrap_or("end");
        for (_, mut x) in std::mem::take(&mut self.open) {
            if let Some(w) = x.w.take() {
                let (file, lost) = close_ogg(w, x.file, Some(reason));
                self.dropped += lost;
                self.done.push(file);
            }
        }
        self.ended = true;
        Ok(())
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// 닫힌 파일(닫힌 차례)
    pub fn files(&self) -> &[OggFile] {
        &self.done
    }
}

fn fresh(base: &OggFile, path: PathBuf) -> OggFile {
    OggFile { path, segment: base.segment, track_id: base.track_id.clone(), user_id: base.user_id.clone(), source: base.source, packets: 0, samples: 0, first_seq: None, last_seq: None, first_ts: None, end_ts: None, filled: 0, reason: None, talk: base.talk.clone() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Meta {
    filled: bool,
    seq_first: u16,
    seq_last: u16,
    ts: u32,
    end_ts: u32,
}

impl Meta {
    fn of(f: &Frame) -> Meta {
        Meta { filled: f.filled, seq_first: f.seq_first, seq_last: f.seq_last, ts: f.rtp_ts, end_ts: f.rtp_ts.wrapping_add(opus_samples(&f.data) as u32) }
    }
}

fn count(file: &mut OggFile, m: Meta) {
    file.packets += 1;
    file.filled += u64::from(m.filled);
    if !m.filled {
        file.first_seq.get_or_insert(m.seq_first);
        file.last_seq = Some(m.seq_last);
    }
    file.first_ts.get_or_insert(m.ts);
    file.end_ts = Some(m.end_ts);
}

fn close_ogg(mut w: OggOpus, mut file: OggFile, reason: Option<&str>) -> (OggFile, u64) {
    let r = w.finish();
    file.samples = w.granule;
    match r {
        Ok(last) => {
            if let Some(m) = last {
                count(&mut file, m);
            }
            file.reason = reason.map(str::to_string);
            (file, 0)
        }
        Err(_) => {
            file.reason = Some("disk_low".into());
            (file, u64::from(w.pending.is_some()))
        }
    }
}

pub fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

/// Ogg Opus 파일 하나 — 한 쪽에 한 패킷 · 마지막 쪽에 EOS
struct OggOpus {
    f: Dest,
    len: u64,
    serial: u32,
    seq: u32,
    granule: u64,
    pending: Option<(Vec<u8>, Meta)>,
}

impl OggOpus {
    fn create(path: &Path, serial: u32, disk: &mut Disk) -> io::Result<(OggOpus, PathBuf)> {
        let (f, path) = open_dest(path, false, disk)?;
        let mut w = OggOpus { f, len: 0, serial, seq: 0, granule: 0, pending: None };
        match w.head() {
            Ok(()) => Ok((w, path)),
            Err(e) => {
                drop(w);
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }

    fn head(&mut self) -> io::Result<()> {
        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&0u16.to_le_bytes()); // pre-skip — RTP 로 받은 것이라 인코더 지연을 모른다
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // mapping family
        self.page(&head, 0x02, 0)?;
        let mut tags = b"OpusTags".to_vec();
        let vendor = b"oxpeer recorder";
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        self.page(&tags, 0x00, 0)
    }

    fn push(&mut self, packet: &[u8], m: Meta) -> io::Result<Option<Meta>> {
        let written = match self.pending.take() {
            Some((p, pm)) => {
                if let Err(e) = self.emit(&p, 0x00) {
                    self.pending = Some((p, pm));
                    return Err(e);
                }
                Some(pm)
            }
            None => None,
        };
        self.pending = Some((packet.to_vec(), m));
        Ok(written)
    }

    fn finish(&mut self) -> io::Result<Option<Meta>> {
        let last = match self.pending.take() {
            Some((p, m)) => {
                if let Err(e) = self.emit(&p, 0x04) {
                    self.pending = Some((p, m));
                    return Err(e);
                }
                Some(m)
            }
            None => {
                self.page(&[], 0x04, self.granule)?;
                None
            }
        };
        self.f.flush()?;
        Ok(last)
    }

    fn emit(&mut self, p: &[u8], flags: u8) -> io::Result<()> {
        let g = self.granule + opus_samples(p);
        self.page(p, flags, g)?;
        self.granule = g;
        Ok(())
    }

    fn page(&mut self, packet: &[u8], flags: u8, granule: u64) -> io::Result<()> {
        let h = page_bytes(packet, flags, granule, self.serial, self.seq);
        if let Err(e) = self.f.write_all(&h) {
            let _ = self.f.cut(self.len);
            return Err(e);
        }
        self.len += h.len() as u64;
        self.seq += 1;
        Ok(())
    }
}

pub(super) fn page_bytes(packet: &[u8], flags: u8, granule: u64, serial: u32, seq: u32) -> Vec<u8> {
    let mut lacing = vec![255u8; packet.len() / 255];
    lacing.push((packet.len() % 255) as u8);
    let mut h = b"OggS".to_vec();
    h.push(0);
    h.push(flags);
    h.extend_from_slice(&granule.to_le_bytes());
    h.extend_from_slice(&serial.to_le_bytes());
    h.extend_from_slice(&seq.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.push(lacing.len() as u8);
    h.extend_from_slice(&lacing);
    h.extend_from_slice(packet);
    let crc = crc(&h);
    h[22..26].copy_from_slice(&crc.to_le_bytes());
    h
}


/// Ogg 의 CRC-32(다항 0x04C11DB7 · 반사 없음 · 초기값 0)
pub(super) fn crc(b: &[u8]) -> u32 {
    let mut c = 0u32;
    for &x in b {
        c ^= u32::from(x) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
    }
    c
}

pub(super) const RETRY_NS: u64 = 1_000_000_000;
const MAX_NUMBER: u32 = 9_999;

pub(super) use super::MIN_FREE_BYTES;

pub(super) fn retry_due(stalled: Option<u64>, now: u64) -> bool {
    stalled.is_none_or(|at| now.saturating_sub(at) >= RETRY_NS)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Stall {
    pub(super) at: u64,
    pub(super) space: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gate {
    Open,
    Wait,
    Recheck,
}

pub(super) fn gate(stall: Option<Stall>, now: u64, free: impl FnOnce() -> Option<u64>, min_free: u64) -> Gate {
    let Some(s) = stall else { return Gate::Open };
    if !retry_due(Some(s.at), now) {
        return Gate::Wait;
    }
    if !s.space || free().is_none_or(|f| f >= min_free) {
        return Gate::Open;
    }
    Gate::Recheck
}

pub(super) fn rotates(kind: io::ErrorKind, on_open_file: bool) -> bool {
    on_open_file || matches!(kind, io::ErrorKind::StorageFull | io::ErrorKind::WriteZero | io::ErrorKind::QuotaExceeded | io::ErrorKind::FileTooLarge)
}

pub(super) fn numbered(path: &Path, n: u32) -> PathBuf {
    if n == 0 {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match path.extension() {
        Some(e) => format!("{stem}-{n:04}.{}", e.to_string_lossy()),
        None => format!("{stem}-{n:04}"),
    };
    path.with_file_name(name)
}

pub(super) fn open_dest(path: &Path, read: bool, disk: &mut Disk) -> io::Result<(Dest, PathBuf)> {
    let mut n = 0u32;
    loop {
        let p = numbered(path, n);
        match OpenOptions::new().read(read).write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((disk.dest(f), p)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < MAX_NUMBER => n += 1,
            Err(e) => return Err(e),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub path: PathBuf,
    pub media_frames: u64,
    pub duration_ms: u64,
    pub reason: String,
    pub video: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Page {
    pub(super) end: usize,
    pub(super) flags: u8,
    pub(super) granule: u64,
    pub(super) serial: u32,
    pub(super) seq: u32,
    pub(super) body: usize,
}

pub(super) fn read_page(b: &[u8], at: usize) -> Option<Page> {
    let h = b.get(at..at + 27)?;
    if &h[..4] != b"OggS" || h[4] != 0 {
        return None;
    }
    let n = usize::from(h[26]);
    let lacing = b.get(at + 27..at + 27 + n)?;
    let body: usize = lacing.iter().map(|x| usize::from(*x)).sum();
    let end = at + 27 + n + body;
    let mut page = b.get(at..end)?.to_vec();
    let want = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
    page[22..26].fill(0);
    if crc(&page) != want {
        return None;
    }
    Some(Page {
        end,
        flags: h[5],
        granule: u64::from_le_bytes(h[6..14].try_into().ok()?),
        serial: u32::from_le_bytes(h[14..18].try_into().ok()?),
        seq: u32::from_le_bytes(h[18..22].try_into().ok()?),
        body,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct OggScan {
    pub(super) cut: u64,
    pub(super) pages: u64,
    pub(super) frames: u64,
    pub(super) serial: u32,
    pub(super) next_seq: u32,
    pub(super) granule: u64,
    pub(super) eos: bool,
}

pub(super) fn scan_ogg(b: &[u8]) -> OggScan {
    let mut s = OggScan::default();
    let mut at = 0;
    while let Some(p) = read_page(b, at) {
        if s.pages > 0 && p.serial != s.serial {
            break;
        }
        if s.pages >= 2 && p.body > 0 {
            s.frames += 1;
        }
        s.pages += 1;
        s.serial = p.serial;
        s.next_seq = p.seq.wrapping_add(1);
        s.granule = p.granule;
        s.eos = p.flags & 0x04 != 0;
        at = p.end;
        if s.eos {
            break;
        }
    }
    s.cut = at as u64;
    s
}

pub fn recover_ogg(path: &Path) -> io::Result<Recovered> {
    let b = std::fs::read(path)?;
    let s = scan_ogg(&b);
    if s.pages == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no whole Ogg page"));
    }
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.set_len(s.cut)?;
    if !s.eos {
        f.seek(SeekFrom::Start(s.cut))?;
        f.write_all(&page_bytes(&[], 0x04, s.granule, s.serial, s.next_seq))?;
    }
    f.flush()?;
    Ok(Recovered { path: path.to_path_buf(), media_frames: s.frames, duration_ms: s.granule / 48, reason: "crash".into(), video: false })
}

#[derive(Clone, Debug, Default)]
pub(super) struct Disk {
    after: Option<u64>,
    sticky: bool,
    budget: Option<Arc<Budget>>,
}

impl Disk {
    fn dest(&mut self, f: File) -> Dest {
        let Some(left) = self.after else { return Dest::Plain(f) };
        let sticky = self.sticky;
        let budget = self.budget.get_or_insert_with(|| Arc::new(Budget { left: AtomicU64::new(left), sticky, tripped: AtomicBool::new(false) }));
        Dest::Faulty(f, budget.clone())
    }
}

#[derive(Debug)]
pub(super) struct Budget {
    left: AtomicU64,
    sticky: bool,
    tripped: AtomicBool,
}

impl Budget {
    fn take(&self, n: usize) -> io::Result<usize> {
        if n == 0 || (self.tripped.load(Ordering::Relaxed) && !self.sticky) {
            return Ok(n);
        }
        let left = self.left.load(Ordering::Relaxed);
        if left == 0 {
            self.tripped.store(true, Ordering::Relaxed);
            return Err(io::Error::new(io::ErrorKind::StorageFull, "write budget spent"));
        }
        let k = n.min(usize::try_from(left).unwrap_or(usize::MAX));
        self.left.store(left - k as u64, Ordering::Relaxed);
        Ok(k)
    }
}

pub(super) enum Dest {
    Plain(File),
    Faulty(File, Arc<Budget>),
}

impl Dest {
    fn file(&mut self) -> &mut File {
        match self {
            Dest::Plain(f) | Dest::Faulty(f, _) => f,
        }
    }

    pub(super) fn cut(&mut self, at: u64) -> io::Result<()> {
        let f = self.file();
        f.set_len(at)?;
        f.seek(SeekFrom::Start(at)).map(|_| ())
    }
}

impl Write for Dest {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        match self {
            Dest::Plain(f) => f.write(b),
            Dest::Faulty(f, budget) => {
                let k = budget.take(b.len())?;
                f.write(&b[..k])
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file().flush()
    }
}

impl Seek for Dest {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file().seek(pos)
    }
}

#[cfg(test)]
impl Disk {
    pub(super) fn full_after(bytes: u64, sticky: bool) -> Disk {
        let budget = Arc::new(Budget { left: AtomicU64::new(bytes), sticky, tripped: AtomicBool::new(false) });
        Disk { after: Some(bytes), sticky, budget: Some(budget) }
    }

    pub(super) fn fill(&self) {
        if let Some(b) = &self.budget {
            b.left.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
impl OggSink {
    pub(super) fn inject(&mut self, disk: Disk) {
        self.disk = disk;
    }
}
