// author: kodeholic (powered by Claude)
//! 내장 결과물 — Ogg Opus(RFC 7845 · RFC 3533 · SDK:RULE-PLAT-REC-API 「내장 결과물도 지점 위에」).
//! ★프레임(지점 ②)과 세그먼트 사건(지점 ③)만 쓴다 — 세그먼트 하나 = 파일 하나. 받은 Opus 패킷을 그대로 담는다(다시 인코딩하지 않는다).
//! ★결정론(SDK:RULE-PLAT-REC-PIPE) — 파일 이름 · 일련번호는 세그먼트에서 셈하고 벽시계를 넣지 않는다. 그래뉼 위치는 TOC 가 말하는 길이(RFC 6716 §3.1).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use super::pipeline::{Output, SegmentKind};

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
    w: OggOpus,
    file: OggFile,
}

/// 세그먼트마다 Ogg Opus 하나(음성 · opus 세그먼트만 · 나머지는 지나친다)
pub struct OggSink {
    dir: PathBuf,
    open: BTreeMap<u64, Open>,
    done: Vec<OggFile>,
    talks: std::collections::BTreeSet<String>,
}

impl OggSink {
    pub fn new(dir: impl Into<PathBuf>) -> io::Result<OggSink> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(OggSink { dir, open: BTreeMap::new(), done: Vec::new(), talks: std::collections::BTreeSet::new() })
    }

    /// 파이프라인 출력 하나
    pub fn write(&mut self, o: &Output) -> io::Result<()> {
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
                let w = OggOpus::create(&path, s.id as u32)?;
                let file = OggFile { path, segment: s.id, track_id: s.track_id.clone(), user_id: s.user_id.clone(), source: s.source, packets: 0, samples: 0, first_seq: None, last_seq: None, first_ts: None, end_ts: None, filled: 0, reason: None, talk: s.talk.clone() };
                self.open.insert(s.id, Open { w, file });
            }
            Output::Segment(s) => {
                if let Some(mut x) = self.open.remove(&s.id) {
                    x.file.samples = x.w.finish()?;
                    x.file.reason = s.reason.map(str::to_string);
                    if x.file.user_id.is_none() {
                        x.file.user_id = s.user_id.clone();
                    }
                    self.done.push(x.file);
                }
            }
            Output::Frame(f) => {
                if let Some(x) = f.segment.and_then(|id| self.open.get_mut(&id)) {
                    x.w.push(&f.data)?;
                    x.file.packets += 1;
                    x.file.filled += u64::from(f.filled);
                    if !f.filled {
                        x.file.first_seq.get_or_insert(f.seq_first);
                        x.file.last_seq = Some(f.seq_last);
                    }
                    x.file.first_ts.get_or_insert(f.rtp_ts);
                    x.file.end_ts = Some(f.rtp_ts.wrapping_add(opus_samples(&f.data) as u32));
                }
            }
            Output::Orphan(_) => {}
        }
        Ok(())
    }

    /// 닫힌 파일(닫힌 차례)
    pub fn files(&self) -> &[OggFile] {
        &self.done
    }
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

/// Ogg Opus 파일 하나 — 한 쪽에 한 패킷 · 마지막 쪽에 EOS
struct OggOpus {
    f: BufWriter<File>,
    serial: u32,
    seq: u32,
    granule: u64,
    pending: Option<Vec<u8>>,
}

impl OggOpus {
    fn create(path: &Path, serial: u32) -> io::Result<OggOpus> {
        let mut w = OggOpus { f: BufWriter::new(File::create(path)?), serial, seq: 0, granule: 0, pending: None };
        let mut head = b"OpusHead".to_vec();
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&0u16.to_le_bytes()); // pre-skip — RTP 로 받은 것이라 인코더 지연을 모른다
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // mapping family
        w.page(&head, 0x02, 0)?;
        let mut tags = b"OpusTags".to_vec();
        let vendor = b"oxpeer recorder";
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.page(&tags, 0x00, 0)?;
        Ok(w)
    }

    fn push(&mut self, packet: &[u8]) -> io::Result<()> {
        if let Some(p) = self.pending.take() {
            self.emit(&p, 0x00)?;
        }
        self.pending = Some(packet.to_vec());
        Ok(())
    }

    fn finish(mut self) -> io::Result<u64> {
        match self.pending.take() {
            Some(p) => self.emit(&p, 0x04)?,
            None => self.page(&[], 0x04, self.granule)?,
        }
        self.f.flush()?;
        Ok(self.granule)
    }

    fn emit(&mut self, p: &[u8], flags: u8) -> io::Result<()> {
        self.granule += opus_samples(p);
        let g = self.granule;
        self.page(p, flags, g)
    }

    fn page(&mut self, packet: &[u8], flags: u8, granule: u64) -> io::Result<()> {
        let mut lacing = vec![255u8; packet.len() / 255];
        lacing.push((packet.len() % 255) as u8);
        let mut h = b"OggS".to_vec();
        h.push(0);
        h.push(flags);
        h.extend_from_slice(&granule.to_le_bytes());
        h.extend_from_slice(&self.serial.to_le_bytes());
        h.extend_from_slice(&self.seq.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.push(lacing.len() as u8);
        h.extend_from_slice(&lacing);
        h.extend_from_slice(packet);
        let crc = crc(&h);
        h[22..26].copy_from_slice(&crc.to_le_bytes());
        self.seq += 1;
        self.f.write_all(&h)
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

/// Ogg 의 CRC-32(다항 0x04C11DB7 · 반사 없음 · 초기값 0)
fn crc(b: &[u8]) -> u32 {
    let mut c = 0u32;
    for &x in b {
        c ^= u32::from(x) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
    }
    c
}
