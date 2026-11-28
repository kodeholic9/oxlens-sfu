// author: kodeholic (powered by Claude)
//! 내장 결과물 — Matroska(SDK:RULE-PLAT-REC-MKV · 202611/20261110a 옮김 · ★20261110e 사람마다 영상 · 음성 한 파일 — 묶음 · 여는 보류 · 파일 경계 · 시각 기준).
//! ★프레임(지점 ②)과 세그먼트 사건(지점 ③)만 쓴다(SDK:RULE-PLAT-REC-API). 받은 코덱 프레임을 그대로 담는다(다시 인코딩하지 않는다).
//! VP8 은 그대로 · H264 는 Annex-B 를 길이 앞붙임(avcC)으로 바꾼다(Matroska `V_MPEG4/ISO/AVC`) · Opus 는 그대로(`A_OPUS`).
//! 블록 시각 = 파일 원점에서 트랙 첫 프레임까지 + 트랙 RTP 경과 — 줄이 바뀌면 두 프레임의 `log_time` 차로 잇는다. 기준(UTC · `log_time`)은 묶음에서 한 번(HLS 와 같다).
//! 파일 시작 벽시계(DateUTC) = 파일 원점.
//! ★결정론(SDK:RULE-PLAT-REC-PIPE) — 시스템 시계를 읽지 않는다 · 파일 이름 · 트랙 번호는 묶음 · 파일 차례에서 셈한다.
//! Segment · Cluster 는 크기 모르는 꼴(라이브 Matroska)로 열어 쓰고 ★닫을 때 고쳐 쓴다 — 클러스터는 다음 클러스터를 열 때 크기를,
//! 파일은 닫을 때 Segment 크기 · Duration · Cues(키프레임 클러스터 찾기 표) · SeekHead(Info · Tracks · Cues 자리)를. 도중에 죽어도 라이브 꼴로 읽힌다.
//! (20261107 재생기 실측 — 크기 모르는 꼴만으로는 ffprobe 길이가 N/A 였다 · 시간 이동은 ffmpeg 가 파일을 훑어 됐다 — Cues 는 훑지 않는 재생기 몫.)

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::cover::{Cover, Coverage};
use super::ogg::{Dest, Faults, Recovered, open_dest, retry_due};
use super::pipeline::{Output, Segment, SegmentKind};
use super::{Channel, Record};

const ID_EBML: u32 = 0x1A45_DFA3;
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_INFO: u32 = 0x1549_A966;
const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_CUES: u32 = 0x1C53_BB6B;
const ID_SEEKHEAD: u32 = 0x114D_9B74;
const ID_VOID: u32 = 0xEC;
/// SeekHead 를 나중에 채울 빈 자리(Void 전체 바이트)
const SEEKHEAD_ROOM: usize = 96;
const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
/// Matroska 날짜의 바닥(2001-01-01T00:00:00Z) — 유닉스 초
const MKV_EPOCH: u64 = 978_307_200;
/// 클러스터 하나의 블록 상대 시각은 i16(ms) — 넉넉히 끊는다
const CLUSTER_SPAN_MS: u64 = 30_000;
const AUDIO_CLUSTER_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Vp8,
    H264,
    Opus,
}

impl Codec {
    fn of(name: &str) -> Option<Codec> {
        match name.to_ascii_lowercase().as_str() {
            "vp8" => Some(Codec::Vp8),
            "h264" => Some(Codec::H264),
            "opus" => Some(Codec::Opus),
            _ => None,
        }
    }

    fn clock(self) -> u64 {
        if self == Codec::Opus { 48_000 } else { 90_000 }
    }
}

fn vint_size(n: u64) -> Vec<u8> {
    for len in 1..=8u32 {
        if n < (1u64 << (7 * len)) - 1 {
            let mut b = n.to_be_bytes()[8 - len as usize..].to_vec();
            b[0] |= 0x80 >> (len - 1);
            return b;
        }
    }
    UNKNOWN_SIZE.to_vec()
}

fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().take_while(|x| **x == 0).count();
    b[skip..].to_vec()
}

fn el(id: u32, body: &[u8]) -> Vec<u8> {
    let mut v = id_bytes(id);
    v.extend(vint_size(body.len() as u64));
    v.extend_from_slice(body);
    v
}

/// 크기 8바이트 꼴(나중에 같은 자리를 고쳐 쓴다)
fn size8(n: u64) -> [u8; 8] {
    let mut b = n.to_be_bytes();
    b[0] = 0x01;
    b
}

fn uint(id: u32, n: u64) -> Vec<u8> {
    let b = n.to_be_bytes();
    let skip = b.iter().take_while(|x| **x == 0).count().min(7);
    el(id, &b[skip..])
}

fn float(id: u32, f: f64) -> Vec<u8> {
    el(id, &f.to_be_bytes())
}

fn text(id: u32, s: &str) -> Vec<u8> {
    el(id, s.as_bytes())
}

/// 빈 자리(Void) — 전체 `n` 바이트(머리 포함)
fn void(n: usize) -> Vec<u8> {
    let mut v = id_bytes(ID_VOID);
    v.extend(vint_size((n - 2) as u64));
    v.resize(n, 0);
    v
}

/// Annex-B → NAL 목록
pub(super) fn nals(b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|n| n - 3).unwrap_or(b.len());
        if k + 1 < starts.len() && e > s && b[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&b[s..e]);
        }
    }
    out
}

/// avcC(ISO 14496-15 §5.2.4.1) — SPS · PPS 가 없으면 줄의 profile(기본 42e0 · level 1f)로 빈 목록
pub(super) fn avcc(sps: Option<&[u8]>, pps: Option<&[u8]>) -> Vec<u8> {
    let (p, c, l) = match sps {
        Some(s) if s.len() >= 4 => (s[1], s[2], s[3]),
        _ => (0x42, 0xe0, 0x1f),
    };
    let mut v = vec![1, p, c, l, 0xff];
    match sps {
        Some(s) => {
            v.push(0xe1);
            v.extend((s.len() as u16).to_be_bytes());
            v.extend_from_slice(s);
        }
        None => v.push(0xe0),
    }
    match pps {
        Some(s) => {
            v.push(1);
            v.extend((s.len() as u16).to_be_bytes());
            v.extend_from_slice(s);
        }
        None => v.push(0),
    }
    v
}

/// VP8 키프레임 머리(RFC 6386 §9.1) — 시작 부호 9d 01 2a 뒤 14비트 너비 · 높이
fn vp8_dims(f: &[u8]) -> Option<(u64, u64)> {
    if f.len() < 10 || f[0] & 1 != 0 || f[3..6] != [0x9d, 0x01, 0x2a] {
        return None;
    }
    let w = u64::from(u16::from_le_bytes([f[6], f[7]]) & 0x3fff);
    let h = u64::from(u16::from_le_bytes([f[8], f[9]]) & 0x3fff);
    Some((w, h))
}

/// 비트 읽개(지수 골롬 — H.264 §9.1) · 에뮬레이션 방지 바이트를 뗀 RBSP 위에서
struct Bits {
    b: Vec<u8>,
    i: usize,
}

impl Bits {
    fn new(nal: &[u8]) -> Bits {
        let mut b = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &x in nal {
            if zeros >= 2 && x == 3 {
                zeros = 0;
                continue;
            }
            zeros = if x == 0 { zeros + 1 } else { 0 };
            b.push(x);
        }
        Bits { b, i: 0 }
    }

    fn bit(&mut self) -> Option<u64> {
        let byte = *self.b.get(self.i / 8)?;
        let v = (byte >> (7 - self.i % 8)) & 1;
        self.i += 1;
        Some(u64::from(v))
    }

    fn bits(&mut self, n: u32) -> Option<u64> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    fn ue(&mut self) -> Option<u64> {
        let mut z = 0;
        while self.bit()? == 0 {
            z += 1;
            if z > 31 {
                return None;
            }
        }
        Some((1u64 << z) - 1 + self.bits(z)?)
    }

    fn se(&mut self) -> Option<i64> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

/// SPS(H.264 §7.3.2.1.1) → 잘라 낸 뒤 화면 크기
pub(super) fn h264_dims(sps: &[u8]) -> Option<(u64, u64)> {
    let mut r = Bits::new(sps.get(1..)?);
    let profile = r.bits(8)?;
    r.bits(16)?;
    r.ue()?;
    let mut chroma = 1;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = r.ue()?;
        if chroma == 3 {
            r.bit()?;
        }
        r.ue()?;
        r.ue()?;
        r.bit()?;
        if r.bit()? == 1 {
            for k in 0..if chroma == 3 { 12 } else { 8 } {
                if r.bit()? == 1 {
                    let n = if k < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i64, 8i64);
                    for _ in 0..n {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    r.ue()?;
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?;
    r.bit()?;
    let w_mbs = r.ue()? + 1;
    let h_units = r.ue()? + 1;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?;
    }
    r.bit()?;
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bit()? == 1 {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let (sx, sy) = match chroma {
        0 | 3 => (1, 2 - frame_mbs_only),
        2 => (2, 2 - frame_mbs_only),
        _ => (2, 2 * (2 - frame_mbs_only)),
    };
    let w = (w_mbs * 16).checked_sub(sx * (cl + cr))?;
    let h = ((2 - frame_mbs_only) * h_units * 16).checked_sub(sy * (ct + cb))?;
    Some((w, h))
}

/// Annex-B 프레임 → 4바이트 길이 앞붙임
fn length_prefixed(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(b.len() + 8);
    for n in nals(b) {
        v.extend((n.len() as u32).to_be_bytes());
        v.extend_from_slice(n);
    }
    v
}

/// 파일 안 트랙 하나의 머리 재료 — 트랙 번호 · 코덱 · 첫 프레임(코덱 사적 자료 · 화면 크기)
struct TrackDef {
    num: u64,
    codec: Codec,
    first: Vec<u8>,
}

/// Matroska 파일 하나(트랙 여럿) — 블록 시각은 파일 원점 기준 ms(묶음이 셈해 준다)
struct Mkv {
    f: Dest,
    path: PathBuf,
    date_at: u64,
    duration_at: u64,
    seg_size_at: u64,
    seg_data: u64,
    seekhead_at: u64,
    info_at: u64,
    tracks_at: u64,
    /// 열린 클러스터의 (크기 자리, 본문 시작)
    cluster_open: Option<(u64, u64)>,
    /// 찾기 표 — (클러스터 시각 ms, Segment 본문 기준 클러스터 위치)
    cues: Vec<(u64, u64)>,
    cluster_at: Option<u64>,
    /// 영상 트랙 번호(있으면 Cues · 클러스터를 영상 키프레임에서)
    video: Option<u64>,
    end_ms: u64,
}

fn track_entry(t: &TrackDef, uid: u64) -> Vec<u8> {
    let (id, kind, private, media) = match t.codec {
        Codec::Vp8 => {
            let (w, h) = vp8_dims(&t.first).unwrap_or((0, 0));
            ("V_VP8", 1, None, el(0xE0, &[uint(0xB0, w), uint(0xBA, h)].concat()))
        }
        Codec::H264 => {
            let ns = nals(&t.first);
            let sps = ns.iter().find(|n| n[0] & 0x1f == 7).copied();
            let pps = ns.iter().find(|n| n[0] & 0x1f == 8).copied();
            let (w, h) = sps.and_then(h264_dims).unwrap_or((0, 0));
            ("V_MPEG4/ISO/AVC", 1, Some(avcc(sps, pps)), el(0xE0, &[uint(0xB0, w), uint(0xBA, h)].concat()))
        }
        Codec::Opus => {
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(1);
            head.extend_from_slice(&0u16.to_le_bytes());
            head.extend_from_slice(&48_000u32.to_le_bytes());
            head.extend_from_slice(&0i16.to_le_bytes());
            head.push(0);
            ("A_OPUS", 2, Some(head), el(0xE1, &[float(0xB5, 48_000.0), uint(0x9F, 1)].concat()))
        }
    };
    let mut entry = [uint(0xD7, t.num), uint(0x73C5, uid * 16 + t.num), uint(0x83, kind), text(0x86, id), uint(0x9C, 0)].concat();
    if let Some(p) = private {
        entry.extend(el(0x63A2, &p));
    }
    entry.extend(media);
    el(0xAE, &entry)
}

impl Mkv {
    /// 연다 — 트랙 목록(머리)과 시작 벽시계(DateUTC · 유닉스 ns)를 쓴다
    fn create(path: &Path, tracks: &[TrackDef], uid: u64, start_ns: u64, faults: Faults) -> io::Result<Mkv> {
        let (mut f, path) = open_dest(path, true, faults)?;
        let ebml = [uint(0x4286, 1), uint(0x42F7, 1), uint(0x42F2, 4), uint(0x42F3, 8), text(0x4282, "matroska"), uint(0x4287, 4), uint(0x4285, 2)].concat();
        f.write_all(&el(ID_EBML, &ebml))?;
        let mut seg = id_bytes(ID_SEGMENT);
        let seg_size_at = f.stream_position()? + seg.len() as u64;
        seg.extend_from_slice(&UNKNOWN_SIZE);
        f.write_all(&seg)?;
        let seg_data = f.stream_position()?;
        let seekhead_at = seg_data;
        f.write_all(&void(SEEKHEAD_ROOM))?;
        // Info — Duration 은 8바이트 자리를 두고 닫을 때 고쳐 쓴다 · DateUTC 는 열 때 안다(파일 원점)
        let info_at = f.stream_position()?;
        let head = [uint(0x2AD7B1, 1_000_000), text(0x4D80, "oxpeer recorder"), text(0x5741, "oxpeer recorder")].concat();
        let date_ns = (i128::from(start_ns) - i128::from(MKV_EPOCH) * 1_000_000_000) as i64;
        let date = el(0x4461, &date_ns.to_be_bytes());
        let dur = el(0x4489, &[0u8; 8]);
        let mut info = id_bytes(ID_INFO);
        info.extend(vint_size((head.len() + date.len() + dur.len()) as u64));
        info.extend(&head);
        let date_at = info_at + info.len() as u64 + (date.len() - 8) as u64;
        info.extend(&date);
        let duration_at = info_at + info.len() as u64 + (dur.len() - 8) as u64;
        info.extend(&dur);
        f.write_all(&info)?;
        let tracks_at = f.stream_position()?;
        let body: Vec<u8> = tracks.iter().flat_map(|t| track_entry(t, uid)).collect();
        f.write_all(&el(ID_TRACKS, &body))?;
        let video = tracks.iter().find(|t| t.codec != Codec::Opus).map(|t| t.num);
        Ok(Mkv { f, path, date_at, duration_at, seg_size_at, seg_data, seekhead_at, info_at, tracks_at, cluster_open: None, cues: Vec::new(), cluster_at: None, video, end_ms: 0 })
    }

    /// 블록 하나 — 트랙 번호 · 파일 원점 기준 ms · 키프레임 · 본문(H264 는 길이 앞붙임으로) · 이 블록이 끝나는 ms
    fn push(&mut self, num: u64, codec: Codec, ms: u64, key: bool, data: &[u8], end_ms: u64) -> io::Result<()> {
        let at = self.f.stream_position()?;
        let keep = (self.cluster_open, self.cues.len(), self.cluster_at, self.end_ms);
        let r = self.put(num, codec, ms, key, data, end_ms);
        if r.is_err() {
            (self.cluster_open, self.cluster_at, self.end_ms) = (keep.0, keep.2, keep.3);
            self.cues.truncate(keep.1);
            let _ = self.f.cut(at);
        }
        r
    }

    fn put(&mut self, num: u64, codec: Codec, ms: u64, key: bool, data: &[u8], end_ms: u64) -> io::Result<()> {
        let video_key = self.video == Some(num) && key;
        let new_cluster = match self.cluster_at {
            None => true,
            Some(c) => {
                let rel = ms as i64 - c as i64;
                !(-32_768..=32_767).contains(&rel) || ms.saturating_sub(c) >= CLUSTER_SPAN_MS || video_key || (self.video.is_none() && ms.saturating_sub(c) >= AUDIO_CLUSTER_MS)
            }
        };
        if new_cluster {
            self.close_cluster()?;
            let at = self.f.stream_position()?;
            let mut cl = id_bytes(ID_CLUSTER);
            cl.extend_from_slice(&UNKNOWN_SIZE);
            let size_at = at + 4;
            let data_at = at + cl.len() as u64;
            cl.extend(uint(0xE7, ms));
            self.f.write_all(&cl)?;
            self.cluster_open = Some((size_at, data_at));
            if self.video.is_none() || video_key {
                self.cues.push((ms, at - self.seg_data));
            }
            self.cluster_at = Some(ms);
        }
        let rel = (ms as i64 - self.cluster_at.unwrap_or(ms) as i64) as i16;
        let body = if codec == Codec::H264 { length_prefixed(data) } else { data.to_vec() };
        let mut sb = vint_size(num);
        sb.extend(rel.to_be_bytes());
        sb.push(if key { 0x80 } else { 0x00 });
        sb.extend(body);
        self.f.write_all(&el(0xA3, &sb))?;
        self.end_ms = self.end_ms.max(end_ms);
        Ok(())
    }

    /// 열린 클러스터의 크기를 고쳐 쓴다(끝에서 돌아와)
    fn close_cluster(&mut self) -> io::Result<()> {
        if let Some((size_at, data_at)) = self.cluster_open.take() {
            let end = self.f.stream_position()?;
            self.f.seek(SeekFrom::Start(size_at))?;
            self.f.write_all(&size8(end - data_at))?;
            self.f.seek(SeekFrom::Start(end))?;
        }
        Ok(())
    }

    /// 시작 벽시계(DateUTC)를 고쳐 쓴다 — `log_time` 기준으로 연 파일이 닫기 전에 SR 을 받으면
    fn set_date(&mut self, start_ns: u64) -> io::Result<()> {
        let end = self.f.stream_position()?;
        let date_ns = (i128::from(start_ns) - i128::from(MKV_EPOCH) * 1_000_000_000) as i64;
        self.f.seek(SeekFrom::Start(self.date_at))?;
        self.f.write_all(&date_ns.to_be_bytes())?;
        self.f.seek(SeekFrom::Start(end))?;
        Ok(())
    }

    /// 닫는다 — Cues · Segment 크기 · SeekHead · Duration
    fn finish(mut self) -> io::Result<()> {
        self.close_cluster()?;
        let cue_track = self.video.unwrap_or(1);
        let cues_at = self.f.stream_position()?;
        let points: Vec<u8> = self
            .cues
            .iter()
            .flat_map(|(ms, pos)| el(0xBB, &[uint(0xB3, *ms), el(0xB7, &[uint(0xF7, cue_track), uint(0xF1, *pos)].concat())].concat()))
            .collect();
        self.f.write_all(&el(ID_CUES, &points))?;
        let end = self.f.stream_position()?;
        self.f.seek(SeekFrom::Start(self.seg_size_at))?;
        self.f.write_all(&size8(end - self.seg_data))?;
        let seek = |id: u32, at: u64| el(0x4DBB, &[el(0x53AB, &id.to_be_bytes()), uint(0x53AC, at)].concat());
        let rel = |at: u64| at - self.seg_data;
        let head = el(ID_SEEKHEAD, &[seek(ID_INFO, rel(self.info_at)), seek(ID_TRACKS, rel(self.tracks_at)), seek(ID_CUES, rel(cues_at))].concat());
        let mut room = head.clone();
        room.extend(void(SEEKHEAD_ROOM - head.len()));
        self.f.seek(SeekFrom::Start(self.seekhead_at))?;
        self.f.write_all(&room)?;
        self.f.seek(SeekFrom::Start(self.duration_at))?;
        self.f.write_all(&(self.end_ms as f64).to_be_bytes())?;
        self.f.flush()
    }
}

/// Matroska 결과물 손잡이
#[derive(Clone, Debug)]
pub struct MkvConfig {
    /// 길이 정책(파일 시작에서 이만큼 지난 뒤 영상 첫 키프레임에서 새 파일) — 없으면 발언 정책
    pub span: Option<std::time::Duration>,
    /// 여는 보류 — 묶음의 열린 트랙이 다 첫 프레임을 낼 때까지 · 또는 이만큼(`log_time`)
    pub hold: std::time::Duration,
}

impl Default for MkvConfig {
    fn default() -> MkvConfig {
        MkvConfig { span: None, hold: std::time::Duration::from_secs(2) }
    }
}

/// 파일 안 트랙 하나의 셈
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MkvTrack {
    pub kind: String,
    pub track_id: String,
    pub codec: String,
    pub num: u64,
    pub frames: u64,
    pub keyframes: u64,
    /// UTC 있는 프레임 수(SR 로 이은 것)
    pub synced: u64,
    /// 메운 PLC 프레임 수(SDK:RULE-PLAT-REC-GAP)
    pub filled: u64,
    pub first_rtp: Option<u32>,
    pub last_rtp: Option<u32>,
    /// 파일 원점에서 이 트랙 첫 블록까지(ms)
    pub first_ms: u64,
    /// 마지막 블록 시각(파일 원점에서 ms)
    pub last_ms: u64,
    /// 이 파일 안에서 줄이 바뀐 횟수(`log_time` 차로 이었다)
    pub rebased: u64,
}

/// 닫힌 Matroska 파일 하나(사람마다 · 방 슬롯마다 영상 · 음성)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MkvFile {
    pub path: PathBuf,
    /// 묶음 열쇠 · 묶음 안 파일 차례
    pub group: String,
    pub index: u64,
    pub room: String,
    /// 개인 트랙 주인 — 무전 슬롯은 없다
    pub owner: Option<String>,
    /// 출처 사용자(개인 = 주인 · 슬롯 = 화자)
    pub user_id: Option<String>,
    pub source: Option<u32>,
    pub tracks: Vec<MkvTrack>,
    /// 트랙 사이 시각 기준 — `utc` · `log_time`
    pub base: &'static str,
    /// 파일 시작 벽시계(DateUTC · 유닉스 ms)와 그 출처 — `utc` · `log_time`(log_time 으로 연 파일도 닫기 전에 SR 을 받으면 utc 로 고쳐 쓴다)
    pub start_ms: u64,
    pub date: &'static str,
    /// `log_time` 기준 파일의 DateUTC 를 UTC 로 고쳐 쓸 때 닻이 된 트랙(`audio` · 없으면 `video`) — 영상은 늦은 SR 맞춤으로 음성에 맞춘다. `utc` 기준 파일은 없음
    pub date_track: Option<&'static str>,
    /// 늦은 SR 맞춤의 영상 목표(µs · SDK:RULE-PLAT-REC-HLS 「늦은 SR 맞춤」 — 파일마다) · 없으면 맞춤 없음
    pub video_shift_us: Option<i64>,
    pub duration_ms: u64,
    /// 닫힌 까닭 — `end` · `track`(트랙이 방에서 사라짐) · `source`(화자 바뀜) · `span` · `late_track`(연 뒤 새 종류가 듦) · `no_source` · `gap`
    pub reason: Option<String>,
    /// 발언 id(무전 슬롯 — 세그먼트의 `talk` · 서버가 지은 발언 기록 `msg_id`) · 개인 트랙 · 옛 서버는 없다
    pub talk: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Video = 0,
    Audio = 1,
}

/// 묶음 안 트랙 하나(파일을 넘어 이어지는 시각)
struct GTrk {
    track_id: String,
    codec_name: String,
    codec: Codec,
    seg: u64,
    open: bool,
    have: bool,
    reopen: bool,
    ssrc: u32,
    last_ts: u32,
    last_log: u64,
    ticks: u64,
    first_log: u64,
    first_utc: Option<(u64, u64)>,
    /// 앞 파일에 쓴 마지막 블록의 끝(기준 · 시각 ns) — 길이 정책으로 이어지는 파일의 원점
    last_end: Option<(bool, i128)>,
}

impl GTrk {
    /// 트랙 경과 틱 → 시각(ns) — UTC 기준이면 첫 UTC 프레임에서 RTP 경과로 · 아니면 첫 프레임 `log_time` 에서
    fn abs(&self, ticks: u64, utc: bool) -> i128 {
        let base = match (utc, self.first_utc) {
            (true, Some((u, at))) => i128::from(u) - i128::from(at) * 1_000_000_000 / i128::from(self.codec.clock()),
            _ => i128::from(self.first_log),
        };
        base + i128::from(ticks) * 1_000_000_000 / i128::from(self.codec.clock())
    }
}

/// 열기를 기다리는 프레임(여는 보류)
struct Pend {
    kind: Kind,
    ticks: u64,
    key: bool,
    data: Vec<u8>,
    filled: bool,
    utc: bool,
    rtp: u32,
    rebased: bool,
}

struct OpenFile {
    m: Mkv,
    path: PathBuf,
    index: u64,
    origin: i128,
    utc: bool,
    source: Option<u32>,
    user_id: Option<String>,
    tracks: Vec<(Kind, MkvTrack)>,
    /// 영상이 키프레임을 받았나(파일 안 영상은 키프레임부터)
    keyed: bool,
    start: Option<(Kind, u64)>,
    asked: bool,
    vshift: Option<i128>,
    vshift_at: i128,
}

struct Group {
    key: String,
    room: String,
    owner: Option<String>,
    video: Option<GTrk>,
    audio: Option<GTrk>,
    file: Option<OpenFile>,
    pend: Vec<Pend>,
    pend_since: Option<u64>,
    pend_source: Option<u32>,
    pend_user: Option<String>,
    /// 지금 파일(또는 모으는 더미)의 발언 번호 — 무전 슬롯은 이것이 바뀐 세그먼트에서 새 파일(SDK:RULE-PLAT-REC-MKV)
    utt: Option<u64>,
    /// 지금 발언의 발언 id · 화자 — 무전 슬롯 파일 이름 `{방}-{발언 id}-{화자}.mkv`(SDK:RULE-PLAT-REC-MKV · 없으면 옛 꼴)
    talk: Option<(String, String)>,
    /// 지금 발언 안에서 연 파일 수(같은 발언의 둘째 파일만 번호를 덧붙인다)
    talk_n: u64,
    /// 길이 정책으로 자른 뒤 모으는 중 — 다음에 여는 파일은 앞 파일을 잇는다
    cont: bool,
    n: u64,
    stalled: Option<u64>,
}

impl Group {
    fn trk(&mut self, k: Kind) -> Option<&mut GTrk> {
        match k {
            Kind::Video => self.video.as_mut(),
            Kind::Audio => self.audio.as_mut(),
        }
    }
}

/// 묶음마다 Matroska(사람마다 · 방 슬롯마다 영상 · 음성 한 파일 — SDK:RULE-PLAT-REC-MKV)
pub struct MkvSink {
    dir: PathBuf,
    cfg: MkvConfig,
    groups: BTreeMap<String, Group>,
    segs: std::collections::HashMap<u64, (String, Kind)>,
    done: Vec<MkvFile>,
    unknown: u64,
    wants: Vec<String>,
    late_key: u64,
    faults: Faults,
    dropped: u64,
    ended: bool,
    cover: Cover,
}

impl MkvSink {
    pub fn new(dir: impl Into<PathBuf>) -> io::Result<MkvSink> {
        MkvSink::with(dir, MkvConfig::default())
    }

    pub fn with(dir: impl Into<PathBuf>, cfg: MkvConfig) -> io::Result<MkvSink> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(MkvSink { dir, cfg, groups: BTreeMap::new(), segs: std::collections::HashMap::new(), done: Vec::new(), unknown: 0, wants: Vec::new(), late_key: 0, faults: Faults::default(), dropped: 0, ended: false, cover: Cover::default() })
    }

    pub fn record(&mut self, r: &Record) {
        if r.channel == Channel::Track {
            self.cover.listed(r.log_time, &r.data);
        } else {
            self.cover.seen(r.log_time);
        }
    }

    pub fn coverage(&self) -> Coverage {
        self.cover.summary()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn close(&mut self, reason: Option<&str>) -> io::Result<()> {
        self.close_all(reason.unwrap_or("end"));
        self.ended = true;
        Ok(())
    }

    /// 닫힌 파일(닫힌 차례)
    pub fn files(&self) -> &[MkvFile] {
        &self.done
    }

    /// 지금 열린 파일 수
    pub fn open_files(&self) -> usize {
        self.groups.values().filter(|g| g.file.is_some()).count()
    }

    /// 코덱을 몰라 지나친 세그먼트 수
    pub fn unknown(&self) -> u64 {
        self.unknown
    }

    /// 새 파일에서 키프레임 전이라 버린 영상 프레임 수
    pub fn before_key(&self) -> u64 {
        self.late_key
    }

    /// 키프레임을 요청할 영상 트랙(꺼내면 비운다 — 길이 정책 · 앱이 `Client::request_keyframe` 로)
    pub fn take_keyframe_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.wants)
    }

    /// 그 트랙이 마지막으로 담은 RTP 시각(열린 파일)
    pub fn last_rtp(&self, track_id: &str) -> Option<u32> {
        self.groups.values().filter_map(|g| g.file.as_ref()).flat_map(|f| f.tracks.iter()).find(|(_, t)| t.track_id == track_id).and_then(|(_, t)| t.last_rtp)
    }

    /// 파이프라인 출력 하나
    pub fn write(&mut self, o: &Output) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        let (key, now) = match o {
            Output::Segment(s) if s.event == SegmentKind::Opened => (kind_of(s).map(|k| self.group_key(s, k)), s.first_log_time),
            Output::Segment(s) => (self.segs.get(&s.id).map(|x| x.0.clone()), s.last_log_time),
            Output::Frame(f) => {
                self.cover.loaded(f.log_time, &f.track_id);
                (f.segment.and_then(|id| self.segs.get(&id)).map(|x| x.0.clone()), f.log_time)
            }
            Output::Orphan(_) => (None, 0),
        };
        let r = match o {
            Output::Segment(s) if s.event == SegmentKind::Opened => self.opened(s),
            Output::Segment(s) => self.closed(s),
            Output::Frame(f) => self.frame(f),
            Output::Orphan(_) => Ok(()),
        };
        match (r, key) {
            (Err(_), Some(k)) => {
                self.fail(&k, now);
                Ok(())
            }
            (r, _) => r,
        }
    }

    fn fail(&mut self, key: &str, now: u64) {
        let Some(g) = self.groups.get_mut(key) else { return };
        let _ = close_file(&self.dir, g, "disk_low", &mut self.done);
        self.dropped += g.pend.len() as u64;
        g.pend.clear();
        g.pend_since = None;
        g.cont = false;
        g.stalled = Some(now);
    }

    fn close_all(&mut self, reason: &str) {
        let keys: Vec<String> = self.groups.keys().cloned().collect();
        for k in keys {
            let Some(g) = self.groups.get_mut(&k) else { continue };
            let now = g.video.iter().chain(g.audio.iter()).map(|t| t.last_log).max().unwrap_or(0);
            let opened = if g.pend.is_empty() { Ok(()) } else { open_file(&self.dir, g, &mut self.late_key, self.faults) };
            let r = match opened {
                Ok(()) => close_file(&self.dir, g, reason, &mut self.done),
                e => e,
            };
            if r.is_err() {
                self.fail(&k, now);
            }
        }
    }

    fn group_key(&self, s: &Segment, kind: Kind) -> String {
        let base = match &s.owner {
            Some(o) => format!("{}-{}", s.room, o),
            None => format!("{}-slot", s.room),
        };
        let busy = self.groups.get(&base).is_some_and(|g| match kind {
            Kind::Video => g.video.as_ref().is_some_and(|t| t.track_id != s.track_id && t.open),
            Kind::Audio => g.audio.as_ref().is_some_and(|t| t.track_id != s.track_id && t.open),
        });
        if busy { format!("{base}-{}", s.track_id) } else { base }
    }

    fn opened(&mut self, s: &Segment) -> io::Result<()> {
        let Some(kind) = kind_of(s) else { return Ok(()) };
        let ok = match kind {
            Kind::Video => matches!(Codec::of(&s.codec), Some(Codec::Vp8 | Codec::H264)),
            Kind::Audio => Codec::of(&s.codec) == Some(Codec::Opus),
        };
        if !ok {
            self.unknown += 1;
            return Ok(());
        }
        let codec = Codec::of(&s.codec).unwrap_or(Codec::Opus);
        let key = self.group_key(s, kind);
        let g = self.groups.entry(key.clone()).or_insert_with(|| Group {
            key: key.clone(),
            room: s.room.clone(),
            owner: s.owner.clone(),
            video: None,
            audio: None,
            file: None,
            pend: Vec::new(),
            pend_since: None,
            pend_source: None,
            pend_user: None,
            utt: None,
            talk: None,
            talk_n: 0,
            cont: false,
            n: 0,
            stalled: None,
        });
        self.segs.insert(s.id, (key.clone(), kind));
        // 슬롯 — 발언 번호가 바뀌면 새 파일(발언 하나 = 파일 하나 · CSRC 로 견주지 않는다) · 열린 파일에 없는 종류가 들면 새 파일
        // ★모으던 더미(아직 안 연 파일)도 발언이 바뀌면 그 자리에서 파일로 내고 닫는다 — 안 그러면 다음 화자의 첫 프레임이 앞 화자 더미에 섞인다
        let busy_now = g.file.is_some() || !g.pend.is_empty();
        let other_source = g.owner.is_none() && busy_now && g.utt != s.utterance;
        let new_kind = g.file.as_ref().is_some_and(|f| !f.tracks.iter().any(|(k, _)| *k == kind));
        if other_source || new_kind {
            if g.file.is_none() && !g.pend.is_empty() {
                open_file(&self.dir, g, &mut self.late_key, self.faults)?;
            }
            close_file(&self.dir, g, if other_source { "utterance" } else { "late_track" }, &mut self.done)?;
        }
        g.utt = s.utterance;
        if g.owner.is_none() {
            if g.talk.as_ref().map(|x| &x.0) != s.talk.as_ref() {
                g.talk_n = 0;
            }
            g.talk = s.talk.clone().map(|t| (t, s.user_id.clone().or_else(|| s.source.map(|c| format!("csrc{c}"))).unwrap_or_else(|| "none".into())));
        }
        let same = match kind {
            Kind::Video => g.video.as_ref().is_some_and(|t| t.track_id == s.track_id),
            Kind::Audio => g.audio.as_ref().is_some_and(|t| t.track_id == s.track_id),
        };
        if same {
            if let Some(t) = g.trk(kind) {
                t.seg = s.id;
                t.open = true;
                t.reopen = true;
            }
        } else {
            let t = GTrk { track_id: s.track_id.clone(), codec_name: s.codec.clone(), codec, seg: s.id, open: true, have: false, reopen: false, ssrc: 0, last_ts: 0, last_log: 0, ticks: 0, first_log: 0, first_utc: None, last_end: None };
            match kind {
                Kind::Video => g.video = Some(t),
                Kind::Audio => g.audio = Some(t),
            }
        }
        Ok(())
    }

    fn closed(&mut self, s: &super::pipeline::Segment) -> io::Result<()> {
        let Some((key, kind)) = self.segs.remove(&s.id) else { return Ok(()) };
        let Some(g) = self.groups.get_mut(&key) else { return Ok(()) };
        if let Some(t) = g.trk(kind)
            && t.seg == s.id
        {
            t.open = false;
        }
        let any_open = g.video.as_ref().is_some_and(|t| t.open) || g.audio.as_ref().is_some_and(|t| t.open);
        if !any_open {
            if !g.pend.is_empty() {
                open_file(&self.dir, g, &mut self.late_key, self.faults)?;
            }
            close_file(&self.dir, g, s.reason.unwrap_or("end"), &mut self.done)?;
        }
        Ok(())
    }

    fn frame(&mut self, f: &super::pipeline::Frame) -> io::Result<()> {
        let Some(id) = f.segment else { return Ok(()) };
        let Some((key, kind)) = self.segs.get(&id).cloned() else { return Ok(()) };
        let hold = self.cfg.hold.as_nanos() as u64;
        let span = self.cfg.span.map(|d| d.as_nanos() as u64);
        let Some(g) = self.groups.get_mut(&key) else { return Ok(()) };
        let Some(t) = g.trk(kind) else { return Ok(()) };
        let clock = t.codec.clock();
        // 트랙 시각 — 첫 프레임 · 줄 바뀜 · 다시 열림은 `log_time` 차로 잇고 · 그 밖은 RTP 차(넘침 펼침)
        let mut rebased = false;
        if !t.have {
            t.have = true;
            t.first_log = f.log_time;
            t.ticks = 0;
        } else if t.ssrc != f.ssrc || t.reopen {
            let gap = f.log_time.saturating_sub(t.last_log);
            t.ticks += (u128::from(gap) * u128::from(clock) / 1_000_000_000).max(1) as u64;
            rebased = true;
        } else {
            t.ticks = t.ticks.saturating_add_signed(i64::from(f.rtp_ts.wrapping_sub(t.last_ts) as i32));
        }
        t.reopen = false;
        t.ssrc = f.ssrc;
        t.last_ts = f.rtp_ts;
        t.last_log = f.log_time;
        if t.first_utc.is_none()
            && let Some(u) = f.utc_ns
        {
            t.first_utc = Some((u, t.ticks));
        }
        let p = Pend { kind, ticks: t.ticks, key: kind == Kind::Audio || f.keyframe, data: f.data.clone(), filled: f.filled, utc: f.utc_ns.is_some(), rtp: f.rtp_ts, rebased };
        let track_id = t.track_id.clone();
        if !retry_due(g.stalled, f.log_time) {
            self.dropped += 1;
            return Ok(());
        }
        g.stalled = None;
        // 파일이 없으면 모은다 — 열린 트랙이 다 첫 프레임을 냈거나 보류가 지나면 연다
        if g.file.is_none() {
            if g.pend.is_empty() {
                g.pend_since = Some(f.log_time);
                g.pend_source = f.source;
                g.pend_user = f.user_id.clone().or_else(|| g.owner.clone());
            }
            if g.pend_user.is_none() {
                g.pend_user = f.user_id.clone();
            }
            g.pend.push(p);
            // ★모은 프레임에 음성과 영상 키프레임이 다 있고 두 트랙에 UTC(SR)가 있으면 연다 — 아니면 보류 H 뒤에
            //   (묶음 트랙의 「첫 프레임을 냈나」로 재면 앞 발언 때 켜진 표시가 남아 새 발언 파일이 음성만으로 열린다 — 1층에서 잡힘)
            let both = g.pend.iter().any(|p| p.kind == Kind::Audio) && g.pend.iter().any(|p| p.kind == Kind::Video && p.key);
            let synced = g.video.as_ref().is_some_and(|t| t.first_utc.is_some()) && g.audio.as_ref().is_some_and(|t| t.first_utc.is_some());
            let ready = both && synced;
            let waited = f.log_time.saturating_sub(g.pend_since.unwrap_or(f.log_time)) >= hold;
            if ready || waited {
                open_file(&self.dir, g, &mut self.late_key, self.faults)?;
            }
            return Ok(());
        }
        // 열린 파일에 없는 종류 — 그 파일을 닫고 이 프레임부터 다시 모은다(N3)
        if g.file.as_ref().is_some_and(|fl| !fl.tracks.iter().any(|(k, _)| *k == kind)) {
            close_file(&self.dir, g, "late_track", &mut self.done)?;
            g.pend_since = Some(f.log_time);
            g.pend_source = f.source;
            g.pend_user = f.user_id.clone().or_else(|| g.owner.clone());
            g.pend.push(p);
            return Ok(());
        }
        // 길이 정책 — 이끄는 트랙(영상 · 없으면 음성)이 시작에서 T 지난 키프레임이면 이 프레임 앞에서 새 파일 · T − 1초에 키프레임 요청
        if let Some(span) = span {
            let lead = if g.file.as_ref().is_some_and(|fl| fl.tracks.iter().any(|(k, _)| *k == Kind::Video)) { Kind::Video } else { Kind::Audio };
            if kind == lead {
                let fl = g.file.as_mut().expect("file");
                let start = *fl.start.get_or_insert((lead, p.ticks));
                let el_ns = u128::from(p.ticks.saturating_sub(start.1)) * 1_000_000_000 / u128::from(clock);
                if kind == Kind::Video && !fl.asked && el_ns + 1_000_000_000 >= u128::from(span) {
                    fl.asked = true;
                    self.wants.push(track_id.clone());
                }
                if p.key && el_ns >= u128::from(span) {
                    close_file(&self.dir, g, "span", &mut self.done)?;
                    g.pend_since = Some(f.log_time);
                    g.pend_source = f.source;
                            g.pend_user = f.user_id.clone().or_else(|| g.owner.clone());
                    g.pend.push(p);
                    // ★이어지는 파일도 음성을 보고 연다(보류 길은 같다) — 음성 RTP 시각이 영상보다 느리면 자른 키 뒤에 오는 음성이 키보다 이르다.
                    //   키 하나로 열면 그 음성이 원점 앞으로 떨어져 0 에 눌린다(2층 장시간 판 · 파일마다 1.4초씩 · 202611/20261110e)
                    g.cont = true;
                    return Ok(());
                }
            }
        }
        write_one(g, p, &mut self.late_key)
    }

    /// 끝 — 아직 열린 파일 · 모으던 프레임을 닫는다(파이프라인 `finish` 의 세그먼트 닫힘을 먼저 넣는다)
    pub fn finish(&mut self) -> io::Result<()> {
        self.close_all("end");
        Ok(())
    }
}

/// 모은 프레임으로 파일을 연다 — 머리(트랙 목록)는 모은 프레임에 든 트랙(영상은 키프레임이 있을 때만) · 시각 기준은 묶음에서 처음 한 번(N4)
fn open_file(dir: &Path, g: &mut Group, late_key: &mut u64, faults: Faults) -> io::Result<()> {
    let cont = std::mem::take(&mut g.cont);
    let pend = std::mem::take(&mut g.pend);
    if pend.is_empty() {
        return Ok(());
    }
    // 영상은 키프레임부터 — 첫 키프레임 앞 영상은 버린다
    let first_vkey = pend.iter().position(|p| p.kind == Kind::Video && p.key);
    let pend: Vec<Pend> = pend
        .into_iter()
        .enumerate()
        .filter(|(i, p)| {
            let keep = p.kind == Kind::Audio || first_vkey.is_some_and(|k| *i >= k);
            if !keep {
                *late_key += 1;
            }
            keep
        })
        .map(|(_, p)| p)
        .collect();
    if pend.is_empty() {
        return Ok(());
    }
    // 든 종류 = 모은 프레임의 종류 · 길이 정책으로 이어지는 파일은 묶음이 아는 음성 트랙도(음성 머리는 첫 프레임이 없어도 짓는다 — 안 담으면 다음 음성에서 또 갈린다)
    let kinds: Vec<Kind> = [Kind::Video, Kind::Audio]
        .into_iter()
        .filter(|k| pend.iter().any(|p| p.kind == *k) || (cont && *k == Kind::Audio && g.audio.as_ref().is_some_and(|a| a.open && a.have)))
        .collect();
    // 시각 기준 — 파일마다(Matroska 파일은 서로 독립): 든 트랙 모두에 UTC 가 있으면 utc · 하나라도 없으면 전부 log_time(섞지 않는다)
    let utc = kinds.iter().all(|k| g.trk(*k).is_some_and(|t| t.first_utc.is_some()));
    // 원점 = 모은 프레임 가운데 가장 이른 시각 · 길이 정책으로 이어지는 파일은 앞 파일 음성 끝이 더 이르면 거기서(같은 기준일 때 · 한 프레임 안쪽)
    let mut origin = i128::MAX;
    for p in &pend {
        if let Some(t) = g.trk(p.kind) {
            origin = origin.min(t.abs(p.ticks, utc));
        }
    }
    if cont && let Some((b, e)) = g.audio.as_ref().and_then(|a| a.last_end) && b == utc {
        origin = origin.min(e.max(origin - 100_000_000));
    }
    let defs: Vec<TrackDef> = kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| {
            let first = pend.iter().find(|p| p.kind == *k).map(|p| p.data.clone()).unwrap_or_default();
            let t = match k {
                Kind::Video => g.video.as_ref(),
                Kind::Audio => g.audio.as_ref(),
            }?;
            Some(TrackDef { num: i as u64 + 1, codec: t.codec, first })
        })
        .collect();
    let index = g.n;
    g.n += 1;
    let nth = g.talk_n;
    g.talk_n += 1;
    let path = match &g.talk {
        Some((t, who)) if g.owner.is_none() => match nth {
            0 => dir.join(format!("{}-{}-{}.mkv", safe(&g.room), safe(t), safe(who))),
            k => dir.join(format!("{}-{}-{}-{:04}.mkv", safe(&g.room), safe(t), safe(who), k)),
        },
        _ => dir.join(format!("{}-{:04}.mkv", safe(&g.key), index)),
    };
    let start_ns = origin.max(0) as u64;
    let m = match Mkv::create(&path, &defs, index + 1, start_ns, faults) {
        Ok(m) => m,
        Err(e) => {
            g.pend = pend;
            return Err(e);
        }
    };
    let path = m.path.clone();
    let tracks = kinds
        .iter()
        .enumerate()
        .filter_map(|(i, k)| {
            let t = match k {
                Kind::Video => g.video.as_ref(),
                Kind::Audio => g.audio.as_ref(),
            }?;
            Some((*k, MkvTrack { kind: if *k == Kind::Video { "video".into() } else { "audio".into() }, track_id: t.track_id.clone(), codec: t.codec_name.clone(), num: i as u64 + 1, ..MkvTrack::default() }))
        })
        .collect();
    // 길이 정책의 시작 = 모은 더미의 첫 이끄는 트랙(영상 · 없으면 음성) 프레임 — 보류 동안 쌓인 프레임도 셈에 든다
    let lead = if kinds.contains(&Kind::Video) { Kind::Video } else { Kind::Audio };
    let start = pend.iter().find(|p| p.kind == lead).map(|p| (lead, p.ticks));
    g.file = Some(OpenFile { m, path, index, origin, utc, source: g.pend_source, user_id: g.pend_user.clone(), tracks, keyed: first_vkey.is_some(), start, asked: false, vshift: None, vshift_at: 0 });
    g.pend_since = None;
    let mut rest = pend.into_iter();
    let failed = rest.by_ref().map(|p| write_one(g, p, late_key)).find(Result::is_err);
    if let Some(e) = failed.and_then(Result::err) {
        g.pend = rest.collect();
        return Err(e);
    }
    Ok(())
}

/// 블록 하나를 연 파일에 쓴다
fn write_one(g: &mut Group, p: Pend, late_key: &mut u64) -> io::Result<()> {
    let utc = g.file.as_ref().is_some_and(|f| f.utc);
    let (abs, codec) = match g.trk(p.kind) {
        Some(t) => (t.abs(p.ticks, utc), t.codec),
        None => return Ok(()),
    };
    let dur_ns: i128 = match p.kind {
        Kind::Audio => i128::from(super::ogg::opus_samples(&p.data)) * 1_000_000_000 / 48_000,
        Kind::Video => 1_000_000_000 / 30,
    };
    let shift = match (g.video.as_ref(), g.audio.as_ref()) {
        (Some(v), Some(a)) => match (v.first_utc, a.first_utc) {
            (Some((vu, vat)), Some((au, aat))) => Some(super::pipeline::late_sr_shift_ns((v.first_log, vu, vat, v.codec.clock()), (a.first_log, au, aat, a.codec.clock()))),
            _ => None,
        },
        _ => None,
    };
    let Some(fl) = g.file.as_mut() else { return Ok(()) };
    if !fl.utc
        && fl.vshift.is_none()
        && fl.tracks.iter().any(|(k, _)| *k == Kind::Video)
        && fl.tracks.iter().any(|(k, _)| *k == Kind::Audio)
    {
        fl.vshift = shift;
    }
    if p.kind == Kind::Video && !fl.keyed {
        if !p.key {
            *late_key += 1;
            return Ok(());
        }
        fl.keyed = true;
    }
    let abs = if p.kind == Kind::Video && !fl.utc {
        let now = fl.vshift_at;
        if let Some(target) = fl.vshift {
            fl.vshift_at = super::pipeline::late_sr_step(now, target, super::pipeline::LATE_SR_STEP_NS);
        }
        abs + now
    } else {
        abs
    };
    let ms = ((abs - fl.origin).max(0) / 1_000_000) as u64;
    let end_ms = ((abs + dur_ns - fl.origin).max(0) / 1_000_000) as u64;
    let Some((_, tr)) = fl.tracks.iter_mut().find(|(k, _)| *k == p.kind) else { return Ok(()) };
    let num = tr.num;
    if tr.frames == 0 {
        tr.first_ms = ms;
        tr.first_rtp = Some(p.rtp);
    }
    tr.frames += 1;
    tr.last_ms = ms;
    tr.keyframes += u64::from(p.key && p.kind == Kind::Video);
    tr.synced += u64::from(p.utc);
    tr.filled += u64::from(p.filled);
    tr.last_rtp = Some(p.rtp);
    tr.rebased += u64::from(p.rebased);
    fl.m.push(num, codec, ms, p.key, &p.data, end_ms)?;
    if let Some(t) = g.trk(p.kind) {
        t.last_end = Some((utc, abs + dur_ns));
    }
    Ok(())
}

/// 연 파일을 닫는다(없으면 아무것도 안 한다)
fn close_file(_dir: &Path, g: &mut Group, reason: &str, done: &mut Vec<MkvFile>) -> io::Result<()> {
    let Some(mut fl) = g.file.take() else { return Ok(()) };
    let duration_ms = fl.m.end_ms;
    // log_time 기준으로 연 파일도 닫기 전에 SR(UTC)을 받았으면 DateUTC 를 UTC 로 고쳐 쓴다 — 영상(없으면 음성) 트랙의 두 기준 차만큼 원점을 옮긴다
    let mut date_ns = fl.origin;
    let mut date = if fl.utc { "utc" } else { "log_time" };
    let mut date_track = None;
    let mut r: io::Result<()> = Ok(());
    if !fl.utc {
        let pick = [Kind::Audio, Kind::Video].into_iter().find(|k| fl.tracks.iter().any(|(x, _)| x == k) && g.trk(*k).is_some_and(|t| t.first_utc.is_some()));
        if let Some(k) = pick
            && let Some(t) = g.trk(k)
        {
            date_ns = fl.origin + (t.abs(0, true) - t.abs(0, false));
            r = fl.m.set_date(date_ns.max(0) as u64);
            date = "utc";
            date_track = Some(if k == Kind::Video { "video" } else { "audio" });
        }
    }
    let r = match r {
        Ok(()) => fl.m.finish(),
        e => e,
    };
    let utc = fl.utc;
    done.push(MkvFile {
        path: fl.path,
        group: g.key.clone(),
        index: fl.index,
        room: g.room.clone(),
        owner: g.owner.clone(),
        user_id: fl.user_id,
        source: fl.source,
        tracks: fl.tracks.into_iter().map(|(_, t)| t).collect(),
        base: if utc { "utc" } else { "log_time" },
        start_ms: (date_ns.max(0) / 1_000_000) as u64,
        date,
        date_track,
        video_shift_us: fl.vshift.map(|x| (x / 1_000) as i64),
        duration_ms,
        reason: Some(if r.is_ok() { reason } else { "disk_low" }.to_string()),
        talk: if g.owner.is_none() { g.talk.as_ref().map(|t| t.0.clone()) } else { None },
    });
    r
}

fn kind_of(s: &Segment) -> Option<Kind> {
    match s.kind.as_str() {
        "video" => Some(Kind::Video),
        "audio" => Some(Kind::Audio),
        _ => None,
    }
}

pub(super) fn read_id(b: &[u8], at: usize) -> Option<(u32, usize)> {
    let n = b.get(at)?.leading_zeros() as usize + 1;
    if n > 4 {
        return None;
    }
    let s = b.get(at..at + n)?;
    Some((s.iter().fold(0u32, |a, x| (a << 8) | u32::from(*x)), n))
}

pub(super) fn read_size(b: &[u8], at: usize) -> Option<(Option<u64>, usize)> {
    let n = b.get(at)?.leading_zeros() as usize + 1;
    if n > 8 {
        return None;
    }
    let s = b.get(at..at + n)?;
    let mask = 0xFFu8.checked_shr(n as u32).unwrap_or(0);
    let v = s[1..].iter().fold(u64::from(s[0] & mask), |a, x| (a << 8) | u64::from(*x));
    let unknown = (1u64 << (7 * n)) - 1;
    Some((if v == unknown { None } else { Some(v) }, n))
}

fn head(b: &[u8], at: usize) -> Option<(u32, Option<u64>, usize)> {
    let (id, n) = read_id(b, at)?;
    let (size, k) = read_size(b, at + n)?;
    Some((id, size, at + n + k))
}

fn known_end(b: &[u8], body: usize, size: Option<u64>) -> Option<usize> {
    let end = body.checked_add(usize::try_from(size?).ok()?)?;
    (end <= b.len()).then_some(end)
}

fn rd_uint(b: &[u8]) -> u64 {
    b.iter().fold(0, |a, x| (a << 8) | u64::from(*x))
}

fn children(b: &[u8], from: usize, to: usize) -> Vec<(u32, usize, usize)> {
    let mut out = Vec::new();
    let mut at = from;
    while at < to {
        let Some((id, size, body)) = head(b, at) else { break };
        let Some(end) = known_end(b, body, size).filter(|e| *e <= to) else { break };
        out.push((id, body, end));
        at = end;
    }
    out
}

fn info_fields(b: &[u8], from: usize, to: usize) -> Option<(u64, u64)> {
    let kids = children(b, from, to);
    let at = |id: u32| kids.iter().find(|(x, body, end)| *x == id && end - body == 8).map(|(_, body, _)| *body as u64);
    Some((at(0x4461)?, at(0x4489)?))
}

fn video_track(b: &[u8], from: usize, to: usize) -> Option<u64> {
    children(b, from, to).into_iter().filter(|(id, _, _)| *id == 0xAE).find_map(|(_, body, end)| {
        let kids = children(b, body, end);
        let num = kids.iter().find(|(id, _, _)| *id == 0xD7).map(|(_, s, e)| rd_uint(&b[*s..*e]))?;
        let kind = kids.iter().find(|(id, _, _)| *id == 0x83).map(|(_, s, e)| rd_uint(&b[*s..*e]))?;
        (kind == 1).then_some(num)
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MkvScan {
    pub(super) cut: u64,
    pub(super) seg_size_at: u64,
    pub(super) seg_data: u64,
    pub(super) info_at: u64,
    pub(super) tracks_at: u64,
    pub(super) date_at: u64,
    pub(super) duration_at: u64,
    pub(super) video: Option<u64>,
    pub(super) cues: Vec<(u64, u64)>,
    pub(super) cluster_open: Option<(u64, u64)>,
    pub(super) cluster_at: Option<u64>,
    pub(super) fixes: Vec<(u64, u64)>,
    pub(super) end_ms: u64,
    pub(super) blocks: u64,
}

struct ClusterScan {
    tc: u64,
    first: Option<(u64, bool)>,
    blocks: u64,
    max_ms: u64,
    end: usize,
    whole: bool,
}

fn scan_cluster(b: &[u8], body: usize, size: Option<u64>) -> ClusterScan {
    let limit = size.and_then(|z| body.checked_add(usize::try_from(z).ok()?)).map_or(b.len(), |e| e.min(b.len()));
    let mut c = ClusterScan { tc: 0, first: None, blocks: 0, max_ms: 0, end: body, whole: true };
    let mut at = body;
    while at < limit {
        let Some((id, csize, cbody)) = head(b, at) else {
            c.whole = false;
            break;
        };
        if size.is_none() && id > 0x00FF_FFFF {
            break;
        }
        let Some(cend) = known_end(b, cbody, csize).filter(|e| *e <= limit) else {
            c.whole = false;
            break;
        };
        match id {
            0xE7 => c.tc = rd_uint(&b[cbody..cend]),
            0xA3 => {
                let Some((Some(track), n)) = read_size(b, cbody) else {
                    c.whole = false;
                    break;
                };
                let Some([hi, lo, flags]) = b.get(cbody + n..(cbody + n + 3).min(cend)).and_then(|x| <[u8; 3]>::try_from(x).ok()) else {
                    c.whole = false;
                    break;
                };
                let ms = (c.tc as i64 + i64::from(i16::from_be_bytes([hi, lo]))).max(0) as u64;
                c.first.get_or_insert((track, flags & 0x80 != 0));
                c.blocks += 1;
                c.max_ms = c.max_ms.max(ms);
            }
            _ => {}
        }
        at = cend;
        c.end = cend;
    }
    if size.is_some_and(|z| (body as u64).saturating_add(z) > b.len() as u64) {
        c.whole = false;
    }
    c
}

pub(super) fn scan_mkv(b: &[u8]) -> Option<MkvScan> {
    let (id, size, body) = head(b, 0)?;
    if id != ID_EBML {
        return None;
    }
    let seg_at = known_end(b, body, size)?;
    let (id, n) = read_id(b, seg_at)?;
    if id != ID_SEGMENT || b.get(seg_at + n) != Some(&0x01) || seg_at + n + 8 > b.len() {
        return None;
    }
    let seg_size_at = seg_at + n;
    let seg_data = seg_size_at + 8;
    let mut s = MkvScan { seg_size_at: seg_size_at as u64, seg_data: seg_data as u64, ..MkvScan::default() };
    let (mut info, mut tracks) = (None, None);
    let mut at = seg_data;
    while let Some((id, size, body)) = head(b, at) {
        if ![ID_SEEKHEAD, ID_VOID, ID_INFO, ID_TRACKS].contains(&id) {
            break;
        }
        let Some(end) = known_end(b, body, size) else { break };
        if id == ID_INFO {
            info = Some((at, info_fields(b, body, end)?));
        }
        if id == ID_TRACKS {
            tracks = Some(at);
            s.video = video_track(b, body, end);
        }
        at = end;
    }
    let (info_at, (date_at, duration_at)) = info?;
    s.info_at = info_at as u64;
    s.tracks_at = tracks? as u64;
    s.date_at = date_at;
    s.duration_at = duration_at;
    let mut clusters: Vec<(u64, u64, u64, bool)> = Vec::new();
    while let Some((id, size, body)) = head(b, at) {
        if id != ID_CLUSTER {
            break;
        }
        let c = scan_cluster(b, body, size);
        let Some((track, key)) = c.first else { break };
        if s.video.is_none() || (s.video == Some(track) && key) {
            s.cues.push((c.tc, (at - seg_data) as u64));
        }
        s.cluster_at = Some(c.tc);
        s.blocks += c.blocks;
        s.end_ms = s.end_ms.max(c.max_ms);
        let size_at = at + read_id(b, at).map_or(4, |x| x.1);
        clusters.push((size_at as u64, body as u64, c.end as u64, size.is_some() && body - size_at == 8));
        at = c.end;
        if !c.whole {
            break;
        }
    }
    s.cut = at as u64;
    if let Some((size_at, data_at, _, _)) = clusters.pop() {
        s.cluster_open = Some((size_at, data_at));
    }
    s.fixes = clusters.into_iter().filter(|c| !c.3).map(|(size_at, data_at, end, _)| (size_at, end - data_at)).collect();
    Some(s)
}

pub fn recover_mkv(path: &Path) -> io::Result<Recovered> {
    let b = std::fs::read(path)?;
    let s = scan_mkv(&b).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Matroska head is not readable"))?;
    let mut f = OpenOptions::new().read(true).write(true).open(path)?;
    f.set_len(s.cut)?;
    for (at, len) in &s.fixes {
        f.seek(SeekFrom::Start(*at))?;
        f.write_all(&size8(*len))?;
    }
    f.seek(SeekFrom::Start(s.cut))?;
    let m = Mkv {
        f: Dest::Plain(f),
        path: path.to_path_buf(),
        date_at: s.date_at,
        duration_at: s.duration_at,
        seg_size_at: s.seg_size_at,
        seg_data: s.seg_data,
        seekhead_at: s.seg_data,
        info_at: s.info_at,
        tracks_at: s.tracks_at,
        cluster_open: s.cluster_open,
        cues: s.cues,
        cluster_at: s.cluster_at,
        video: s.video,
        end_ms: s.end_ms,
    };
    m.finish()?;
    Ok(Recovered { path: path.to_path_buf(), media_frames: s.blocks, duration_ms: s.end_ms, reason: "crash".into() })
}

#[cfg(test)]
impl MkvSink {
    pub(super) fn inject(&mut self, faults: Faults) {
        self.faults = faults;
    }
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_and_ids() {
        assert_eq!(vint_size(1), vec![0x81]);
        assert_eq!(vint_size(200), vec![0x40, 200]);
        assert_eq!(id_bytes(ID_EBML), vec![0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(uint(0xD7, 1), vec![0xD7, 0x81, 0x01]);
    }

    #[test]
    fn annexb_to_avcc_and_lengths() {
        let f = [0, 0, 0, 1, 0x67, 0x42, 0xe0, 0x1f, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 1, 0x65, 0x88];
        let ns = nals(&f);
        assert_eq!(ns, vec![&[0x67, 0x42, 0xe0, 0x1f][..], &[0x68, 0xce][..], &[0x65, 0x88][..]]);
        assert_eq!(length_prefixed(&f[8..]), vec![0, 0, 0, 2, 0x68, 0xce, 0, 0, 0, 2, 0x65, 0x88]);
        let c = avcc(Some(ns[0]), Some(ns[1]));
        assert_eq!(&c[..6], &[1, 0x42, 0xe0, 0x1f, 0xff, 0xe1]);
        assert_eq!(avcc(None, None), vec![1, 0x42, 0xe0, 0x1f, 0xff, 0xe0, 0]);
    }

    #[test]
    fn dims() {
        // 640x480 VP8 키프레임 머리
        let k = [0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01];
        assert_eq!(vp8_dims(&k), Some((640, 480)));
        // 1280x720 baseline SPS(지수 골롬으로 지은 것 — 80 x 45 매크로블록 · 자르기 없음)
        let sps = [0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe4];
        assert_eq!(h264_dims(&sps), Some((1280, 720)));
        // 2층 봇 H264 시뮬캐스트 층 SPS(oxe2epy `rtp_tx.h264_sps` — 자르기 있음 · 202611/20261110d) — 쓰는 쪽과 읽는 쪽 교차 대조
        assert_eq!(h264_dims(&[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x02, 0x80, 0xbd, 0xe5, 0x40]), Some((640, 360)));
        assert_eq!(h264_dims(&[0x67, 0x42, 0xc0, 0x1f, 0xda, 0x05, 0x06, 0x5e, 0x74]), Some((320, 180)));
    }

    #[test]
    fn writes_two_tracks_with_numbered_blocks_and_date() {
        // 사람마다 영상 · 음성 한 파일(202611/20261110e) — TrackEntry 둘 · SimpleBlock 마다 트랙 번호 · DateUTC = 연 때 준 원점
        let dir = std::env::temp_dir().join(format!("oxpeer-mkv-av-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("av.mkv");
        let key = [0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01];
        let t0 = 1_800_000_000_000_000_000u64;
        let defs = [TrackDef { num: 1, codec: Codec::Vp8, first: key.to_vec() }, TrackDef { num: 2, codec: Codec::Opus, first: vec![0xf8] }];
        let mut m = Mkv::create(&p, &defs, 7, t0, Faults::default()).unwrap();
        m.push(2, Codec::Opus, 0, true, &[0xf8, 1], 20).unwrap();
        m.push(1, Codec::Vp8, 5, true, &key, 38).unwrap();
        m.push(2, Codec::Opus, 20, true, &[0xf8, 2], 40).unwrap();
        m.finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        assert!(b.windows(5).any(|w| w == b"V_VP8") && b.windows(6).any(|w| w == b"A_OPUS"), "트랙 둘");
        assert!(b.windows(5).any(|w| w == [0x82, 0x00, 0x00, 0x80, 0xf8]), "음성 블록 — 트랙 2 · 상대 0 · 키");
        assert!(b.windows(4).any(|w| w == [0x81, 0x00, 0x00, 0x80]), "영상 키프레임 블록 — 트랙 1 · 새 클러스터(5ms) 기준 상대 0");
        assert!(b.windows(3).any(|w| w == [0xE7, 0x81, 0x05]), "클러스터 시각 5ms");
        let date = ((i128::from(t0) - i128::from(MKV_EPOCH) * 1_000_000_000) as i64).to_be_bytes();
        assert!(b.windows(8).any(|w| w == date), "DateUTC = 원점");
        std::fs::remove_dir_all(&dir).ok();
    }

    fn rd_id(b: &[u8], at: usize) -> (u32, usize) {
        let n = b[at].leading_zeros() as usize + 1;
        (b[at..at + n].iter().fold(0u32, |a, x| (a << 8) | u32::from(*x)), n)
    }

    fn rd_size(b: &[u8], at: usize) -> (u64, usize) {
        let n = b[at].leading_zeros() as usize + 1;
        let v = b[at + 1..at + n].iter().fold(u64::from(b[at] & 0xFFu8.checked_shr(n as u32).unwrap_or(0)), |a, x| (a << 8) | u64::from(*x));
        (v, n)
    }

    fn rd_uint(b: &[u8]) -> u64 {
        b.iter().fold(0, |a, x| (a << 8) | u64::from(*x))
    }

    /// 닫은 파일의 뼈대 — Segment 크기 = 파일 끝 · 클러스터 크기를 따라 걸으면 다음 요소에 닿는다 · SeekHead → Cues · Cue → Cluster · Duration
    #[test]
    fn finished_file_is_sized_indexed_and_has_duration() {
        let dir = std::env::temp_dir().join(format!("oxpeer-mkv-idx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("v.mkv");
        let key = [0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01];
        let mut m = Mkv::create(&p, &[TrackDef { num: 1, codec: Codec::Vp8, first: key.to_vec() }], 1, 0, Faults::default()).unwrap();
        for i in 0..90u64 {
            let k = i % 30 == 0;
            let ms = i * 1000 / 30;
            m.push(1, Codec::Vp8, ms, k, if k { &key } else { &[0x01, 0x00] }, (i + 1) * 1000 / 30).unwrap();
        }
        m.finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        let (_, n) = rd_id(&b, 0);
        let (sz, k) = rd_size(&b, n);
        let seg = n + k + sz as usize;
        let (sid, n) = rd_id(&b, seg);
        assert_eq!(sid, ID_SEGMENT);
        let (ssz, k) = rd_size(&b, seg + n);
        let data = seg + n + k;
        assert_eq!(data + ssz as usize, b.len(), "Segment 크기 = 파일 끝");
        let (mut at, mut clusters, mut cues_at, mut cue_pos, mut dur) = (data, Vec::new(), None, Vec::new(), None);
        while at < b.len() {
            let (id, n) = rd_id(&b, at);
            let (size, k) = rd_size(&b, at + n);
            let body = &b[at + n + k..at + n + k + size as usize];
            match id {
                ID_SEEKHEAD => {
                    let mut i = 0;
                    while i < body.len() {
                        let (_, n1) = rd_id(body, i);
                        let (s1, k1) = rd_size(body, i + n1);
                        let seek = &body[i + n1 + k1..i + n1 + k1 + s1 as usize];
                        let (_, n2) = rd_id(seek, 0);
                        let (s2, k2) = rd_size(seek, n2);
                        let sid = rd_uint(&seek[n2 + k2..n2 + k2 + s2 as usize]) as u32;
                        let rest = &seek[n2 + k2 + s2 as usize..];
                        let (_, n3) = rd_id(rest, 0);
                        let (s3, k3) = rd_size(rest, n3);
                        if sid == ID_CUES {
                            cues_at = Some(data + rd_uint(&rest[n3 + k3..n3 + k3 + s3 as usize]) as usize);
                        }
                        i += n1 + k1 + s1 as usize;
                    }
                }
                ID_INFO => {
                    if let Some(i) = body.windows(3).position(|w| w == [0x44, 0x89, 0x88]) {
                        dur = Some(f64::from_be_bytes(body[i + 3..i + 11].try_into().unwrap()));
                    }
                }
                ID_CLUSTER => clusters.push(at),
                ID_CUES => {
                    for w in body.windows(2).enumerate().filter(|(_, w)| w[0] == 0xF1) {
                        let (s, k) = rd_size(body, w.0 + 1);
                        cue_pos.push(data + rd_uint(&body[w.0 + 1 + k..w.0 + 1 + k + s as usize]) as usize);
                    }
                }
                _ => {}
            }
            at += n + k + size as usize;
        }
        assert_eq!(at, b.len(), "요소 크기를 따라 걸으면 파일 끝에 정확히 닿는다");
        assert_eq!(clusters.len(), 3, "키프레임마다 클러스터");
        let ca = cues_at.expect("SeekHead 에 Cues 자리");
        assert_eq!(rd_id(&b, ca).0, ID_CUES);
        assert_eq!(cue_pos, clusters, "Cue 마다 클러스터 자리");
        assert_eq!(dur, Some(3_000.0), "Duration = (마지막 블록 + 한 간격) RTP 틱 → ms");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn recovery_of_a_header_only_file_says_zero_frames() {
        let dir = std::env::temp_dir().join(format!("oxpeer-mkv-head-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = [0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01];
        let m = Mkv::create(&dir.join("h.mkv"), &[TrackDef { num: 1, codec: Codec::Vp8, first: key.to_vec() }], 1, 0, Faults::default()).unwrap();
        let p = m.path.clone();
        drop(m);
        let r = recover_mkv(&p).unwrap();
        assert_eq!((r.media_frames, r.duration_ms, r.reason.as_str()), (0, 0, "crash"), "SDK:RULE-PLAT-REC-API 회수 — 미디어 프레임이 0 이면 그렇다고 돌려준다");
        let b = std::fs::read(&p).unwrap();
        let s = scan_mkv(&b).unwrap();
        assert_eq!(read_size(&b, s.seg_size_at as usize).unwrap().0, Some(b.len() as u64 - s.seg_data), "Segment 크기를 고쳐 썼다");
        assert_eq!(rd_id(&b, s.cut as usize).0, ID_CUES, "빈 Cues 를 붙였다");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scan_finds_cut_block_open_cluster_and_earlier_unknown_size() {
        let dir = std::env::temp_dir().join(format!("oxpeer-mkv-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = [0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a, 0x80, 0x02, 0xe0, 0x01];
        let defs = [TrackDef { num: 1, codec: Codec::Vp8, first: key.to_vec() }, TrackDef { num: 2, codec: Codec::Opus, first: vec![0xf8] }];
        let mut m = Mkv::create(&dir.join("s.mkv"), &defs, 1, 0, Faults::default()).unwrap();
        for i in 0..60u64 {
            m.push(2, Codec::Opus, i * 20, true, &[0xf8, i as u8], i * 20 + 20).unwrap();
            if i % 25 == 0 {
                m.push(1, Codec::Vp8, i * 20, true, &key, i * 20 + 33).unwrap();
            } else if i % 5 == 0 {
                m.push(1, Codec::Vp8, i * 20, false, &[0x01, 0x00], i * 20 + 33).unwrap();
            }
        }
        let first_cluster = m.cues[0].1 + m.seg_data;
        let p = m.path.clone();
        drop(m);
        let mut b = std::fs::read(&p).unwrap();
        let s = scan_mkv(&b).unwrap();
        assert_eq!(s.video, Some(1), "Tracks 에서 영상 트랙 번호");
        assert_eq!(s.cues.len(), 3, "영상 키 클러스터만 Cue(0 · 500 · 1000ms)");
        assert_eq!(s.cues.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 500, 1000]);
        assert_eq!(s.blocks, 60 + 12, "블록 수");
        assert_eq!(s.end_ms, 1180, "마지막 블록 시각");
        assert_eq!(s.cut, b.len() as u64, "온전하면 끝까지");
        assert!(s.fixes.is_empty());
        let open = s.cluster_open.unwrap();
        assert_eq!(read_size(&b, open.0 as usize).unwrap().0, None, "마지막 클러스터는 열린 채");
        let at = first_cluster as usize + 4;
        b[at..at + 8].copy_from_slice(&UNKNOWN_SIZE);
        let s2 = scan_mkv(&b).unwrap();
        assert_eq!(s2.fixes.len(), 1, "앞 클러스터의 크기 모름은 고칠 목록에");
        assert_eq!(s2.fixes[0].0, at as u64);
        assert_eq!(s2.blocks, s.blocks);
        let cut = &b[..b.len() - 2];
        let s3 = scan_mkv(cut).unwrap();
        assert_eq!(s3.blocks, s.blocks - 1, "잘린 블록 앞에서 멈춘다");
        assert!(s3.cut < cut.len() as u64);
        assert!(scan_mkv(&b[..40]).is_none(), "Info · Tracks 가 없으면 회수하지 못한다");
        std::fs::remove_dir_all(&dir).ok();
    }
}
