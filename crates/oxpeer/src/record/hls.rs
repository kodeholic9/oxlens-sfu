// author: kodeholic (powered by Claude)
//! 내장 결과물 — HLS(fMP4 · CMAF — SDK:RULE-PLAT-REC-HLS · 202611/20261110b). ★재포장만 한다(디코딩 · 인코딩 없음).
//! ★프레임(지점 ②)과 세그먼트 사건(지점 ③)만 쓴다(SDK:RULE-PLAT-REC-API) — 파이프라인은 잇는 길이 정책(끝없음)으로 돌린다.
//! 묶음(HLS 스트림 하나) = 개인 트랙은 주인마다 · 무전 슬롯은 방마다 · 종류마다 트랙 하나(영상 H264 + 음성 Opus).
//! 조각은 목표 T 가 지난 뒤 영상 첫 키프레임에서 자른다(영상이 없으면 아무 음성 프레임) · T − 1초에 키프레임 요청 목록을 낸다(앱이 SDK 에 요청).
//! 시각 — 첫 조각을 쓸 때 기준(모든 트랙에 UTC 가 있으면 UTC · 아니면 전부 `log_time`)을 정하고 그 뒤는 트랙마다 RTP 경과(넘침은 펼친다).
//! 끊김(새 init + `EXT-X-DISCONTINUITY`) — H264 매개변수가 바뀜 · 트랙이 들거나 빠짐.
//! ★결정론(SDK:RULE-PLAT-REC-PIPE) — 시스템 시계를 읽지 않는다 · 파일 이름은 묶음 · 번호에서 셈한다.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::mkv::{avcc, h264_dims, nals};
use super::ogg::opus_samples;
use super::pipeline::{Frame, Output, Segment, SegmentKind};

/// HLS 결과물 손잡이
#[derive(Clone, Debug)]
pub struct HlsConfig {
    /// 조각 목표 길이(기본 4초 — LiveKit 기본)
    pub target: Duration,
    /// `live.m3u8` 조각 창(기본 6)
    pub window: usize,
    /// 운영 로그 — 주면 이 결과물이 로거를 갖고 `TARGETDURATION` 을 넘은 조각마다 `diag`(warn) 한 줄을 낸다 · 없으면 세기만(`over_target`)
    pub log: Option<crate::log::LogConfig>,
}

impl Default for HlsConfig {
    fn default() -> HlsConfig {
        HlsConfig { target: Duration::from_secs(4), window: 6, log: None }
    }
}

/// 조각 하나
#[derive(Clone, Debug, PartialEq)]
pub struct HlsSegment {
    pub file: PathBuf,
    /// 이 조각이 쓰는 init
    pub init: PathBuf,
    /// 앞 조각과 끊김(`EXT-X-DISCONTINUITY`)
    pub discontinuity: bool,
    /// 길이(초 — 이끄는 트랙의 시각)
    pub duration: f64,
}

/// 묶음 하나(HLS 스트림 하나)의 결과
#[derive(Clone, Debug, PartialEq)]
pub struct HlsStream {
    pub key: String,
    pub dir: PathBuf,
    pub room: String,
    /// 개인 트랙 주인 — 무전 슬롯은 없다
    pub owner: Option<String>,
    pub segments: Vec<HlsSegment>,
    pub inits: Vec<PathBuf>,
    pub event_playlist: PathBuf,
    pub live_playlist: PathBuf,
    /// `TARGETDURATION` 을 넘은 조각 수
    pub over_target: u64,
    /// 시각 기준 — `utc` · `log_time`
    pub base: Option<&'static str>,
    /// 거절 사유(H264 아닌 영상 — 트랜스코딩은 도메인 몫)
    pub refused: Option<String>,
    /// 묶음에 든 적 있는 트랙(종류 · track_id · 코덱)
    pub tracks: Vec<(String, String, String)>,
    /// 키프레임을 요청한 횟수
    pub keyframe_requests: u64,
    /// 같은 시각의 새 층 프레임으로 바꿔 낀 영상 견본 수(시뮬캐스트 층 바꿈 — 서버가 같은 그림을 층만 바꿔 다시 보낸다)
    pub replaced: u64,
    /// 늦은 SR 맞춤의 영상 목표(µs) — `log_time` 기준 묶음에서 두 트랙이 UTC 를 받은 뒤 정한다 · 없으면 맞춤 없음
    pub video_shift_us: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Video,
    Audio,
}

impl Kind {
    fn timescale(self) -> u64 {
        match self {
            Kind::Video => 90_000,
            Kind::Audio => 48_000,
        }
    }

    fn track_id(self) -> u32 {
        match self {
            Kind::Video => 1,
            Kind::Audio => 2,
        }
    }
}

/// 조각 안 트랙 하나 — 종류 · 첫 샘플 디코드 시각 · [(길이, 크기, 키)] · 자료
struct Traf {
    kind: Kind,
    tfdt: u64,
    rows: Vec<(u32, u32, bool)>,
    data: Vec<u8>,
}

struct Sample {
    ticks: u64,
    key: bool,
    data: Vec<u8>,
}

/// 묶음 안 트랙 하나(파이프라인 세그먼트 하나)
struct Trk {
    track_id: String,
    seg: u64,
    first_ts: Option<u32>,
    last_ts: u32,
    ticks: u64,
    first_log: u64,
    /// 첫 UTC 프레임 — (UTC ns, 그때의 트랙 경과 틱)
    first_utc: Option<(u64, u64)>,
    /// 묶음 기준 위 첫 프레임 시각(ns) — 처음 쓸 때 정한다
    t0: Option<i128>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    dims: (u64, u64),
    buf: Vec<Sample>,
    /// 마지막으로 쓴 샘플의 틱 · 길이(다음 조각 길이 셈)
    last_dur: u64,
    /// ★음성은 견본 길이 누적이 시각축이다(Ogg 그래뉼과 같다) — 다음 조각의 디코드 시각. RTP 위치와의 차는 PLC 나머지(< 한 프레임 — SDK:RULE-PLAT-REC-GAP)
    next_dts: Option<u64>,
    /// 이 init 에 쓰였나
    in_init: bool,
}

impl Trk {
    fn new(track_id: &str, seg: u64) -> Trk {
        Trk { track_id: track_id.to_string(), seg, first_ts: None, last_ts: 0, ticks: 0, first_log: 0, first_utc: None, t0: None, sps: None, pps: None, dims: (0, 0), buf: Vec::new(), last_dur: 0, next_dts: None, in_init: false }
    }

    /// RTP 시각 → 첫 프레임 뒤 경과 틱(넘침은 펼친다)
    fn advance(&mut self, ts: u32) -> u64 {
        match self.first_ts {
            None => {
                self.first_ts = Some(ts);
                self.ticks = 0;
            }
            Some(_) => {
                let d = ts.wrapping_sub(self.last_ts) as i32;
                self.ticks = self.ticks.saturating_add_signed(i64::from(d));
            }
        }
        self.last_ts = ts;
        self.ticks
    }
}

struct Group {
    dir: PathBuf,
    video: Option<Trk>,
    audio: Option<Trk>,
    /// 시각 기준(정해지면) — utc 인가 · 묶음 원점(ns) · utc 기준일 때 `log_time` → 기준 차
    base: Option<(bool, i128, i128)>,
    /// 조각 시작 — 이끄는 트랙의 종류 · 틱
    seg_start: Option<(Kind, u64)>,
    asked: bool,
    reinit: bool,
    disc_next: bool,
    init_n: u64,
    init: Option<PathBuf>,
    out: HlsStream,
    ended: bool,
    /// 늦은 SR 맞춤 — 영상 목표 · 다음 영상 견본에 줄 양(영상 틱)
    vshift: Option<i64>,
    vshift_at: i64,
}

/// 묶음마다 HLS 하나(H264 · Opus)
pub struct HlsSink {
    dir: PathBuf,
    cfg: HlsConfig,
    groups: BTreeMap<String, Group>,
    /// 파이프라인 세그먼트 → (묶음, 종류)
    segs: HashMap<u64, (String, Kind)>,
    wants: Vec<String>,
    done: Vec<HlsStream>,
    log: Option<Arc<crate::log::Logger>>,
}

impl HlsSink {
    pub fn new(dir: impl Into<PathBuf>, cfg: HlsConfig) -> io::Result<HlsSink> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let log = cfg.log.clone().map(crate::log::Logger::start);
        Ok(HlsSink { dir, cfg, groups: BTreeMap::new(), segs: HashMap::new(), wants: Vec::new(), done: Vec::new(), log })
    }

    /// `TARGETDURATION`(초) = ⌈1.5 T⌉
    fn target_duration(&self) -> u64 {
        (self.cfg.target.as_millis() as u64 * 3).div_ceil(2000)
    }

    /// 키프레임을 요청할 영상 트랙(꺼내면 비운다) — 앱이 `Client::request_keyframe` 로 넘긴다. ★결과물 바이트는 받은 프레임만으로 정해진다
    pub fn take_keyframe_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.wants)
    }

    /// 끝난 묶음(끝난 차례) · 아직 열린 묶음은 `finish` 뒤에 든다
    pub fn streams(&self) -> &[HlsStream] {
        &self.done
    }

    /// 파이프라인 출력 하나
    pub fn write(&mut self, o: &Output) -> io::Result<()> {
        match o {
            Output::Segment(s) if s.event == SegmentKind::Opened => self.opened(s),
            Output::Segment(s) => self.closed(s),
            Output::Frame(f) => self.frame(f),
            Output::Orphan(_) => Ok(()),
        }
    }

    fn opened(&mut self, s: &Segment) -> io::Result<()> {
        let kind = match s.kind.as_str() {
            "video" => Kind::Video,
            "audio" => Kind::Audio,
            _ => return Ok(()),
        };
        let base = match &s.owner {
            Some(o) => format!("{}-{}", s.room, o),
            None => format!("{}-slot", s.room),
        };
        // 같은 종류의 다른 트랙이 이미 묶음에 있으면 따로 묶음
        let busy = self.groups.get(&base).is_some_and(|g| match kind {
            Kind::Video => g.video.as_ref().is_some_and(|t| t.track_id != s.track_id),
            Kind::Audio => g.audio.as_ref().is_some_and(|t| t.track_id != s.track_id),
        });
        let key = if busy { format!("{base}-{}", s.track_id) } else { base };
        let target = self.target_duration();
        let dir = self.dir.join(safe(&key));
        let g = self.groups.entry(key.clone()).or_insert_with(|| Group {
            dir: dir.clone(),
            video: None,
            audio: None,
            base: None,
            seg_start: None,
            asked: false,
            reinit: false,
            disc_next: false,
            init_n: 0,
            init: None,
            out: HlsStream {
                key: key.clone(),
                dir: dir.clone(),
                room: s.room.clone(),
                owner: s.owner.clone(),
                segments: Vec::new(),
                inits: Vec::new(),
                event_playlist: dir.join("event.m3u8"),
                live_playlist: dir.join("live.m3u8"),
                over_target: 0,
                base: None,
                refused: None,
                tracks: Vec::new(),
                keyframe_requests: 0,
                replaced: 0,
                video_shift_us: None,
            },
            ended: false,
            vshift: None,
            vshift_at: 0,
        });
        g.out.tracks.push((s.kind.clone(), s.track_id.clone(), s.codec.clone()));
        let ok = match kind {
            Kind::Video => s.codec.eq_ignore_ascii_case("h264"),
            Kind::Audio => s.codec.eq_ignore_ascii_case("opus"),
        };
        if !ok && g.out.refused.is_none() {
            g.out.refused = Some(format!("{} {} — HLS 는 H264 · Opus 만 재포장한다(트랜스코딩은 도메인 몫 · SDK:RULE-PLAT-REC-HLS)", s.kind, s.codec));
        }
        self.segs.insert(s.id, (key.clone(), kind));
        if g.out.refused.is_some() {
            return Ok(());
        }
        // 이미 쓴 묶음에 트랙이 들거나 다시 열리면 — 지금까지를 조각으로 내고 끊김
        if g.init.is_some() {
            flush(g, &self.cfg, self.log.as_deref(), target, None)?;
            g.reinit = true;
        }
        let t = Trk::new(&s.track_id, s.id);
        match kind {
            Kind::Video => g.video = Some(t),
            Kind::Audio => g.audio = Some(t),
        }
        Ok(())
    }

    fn closed(&mut self, s: &Segment) -> io::Result<()> {
        let Some((key, kind)) = self.segs.remove(&s.id) else { return Ok(()) };
        let target = self.target_duration();
        let Some(g) = self.groups.get_mut(&key) else { return Ok(()) };
        if g.out.refused.is_some() {
            if self.segs.values().all(|(k, _)| *k != key)
                && let Some(g) = self.groups.remove(&key)
            {
                self.done.push(g.out);
            }
            return Ok(());
        }
        let same = match kind {
            Kind::Video => g.video.as_ref().is_some_and(|t| t.seg == s.id),
            Kind::Audio => g.audio.as_ref().is_some_and(|t| t.seg == s.id),
        };
        if !same {
            return Ok(());
        }
        flush(g, &self.cfg, self.log.as_deref(), target, None)?;
        match kind {
            Kind::Video => g.video = None,
            Kind::Audio => g.audio = None,
        }
        if g.video.is_none() && g.audio.is_none() {
            g.ended = true;
            playlists(g, &self.cfg, target)?;
            if let Some(g) = self.groups.remove(&key) {
                self.done.push(g.out);
            }
        } else {
            g.reinit = true;
        }
        Ok(())
    }

    fn frame(&mut self, f: &Frame) -> io::Result<()> {
        let Some(id) = f.segment else { return Ok(()) };
        let Some((key, kind)) = self.segs.get(&id).cloned() else { return Ok(()) };
        let target = self.target_duration();
        let tgt_ticks = |k: Kind| self.cfg.target.as_millis() as u64 * k.timescale() / 1000;
        let Some(g) = self.groups.get_mut(&key) else { return Ok(()) };
        if g.out.refused.is_some() {
            return Ok(());
        }
        let lead = if g.video.is_some() { Kind::Video } else { Kind::Audio };
        // H264 — 키프레임의 매개변수(M4 — 늘 키프레임 안). init 과 달라지면 그 자리에서 끊는다
        let mut params: Option<(Vec<u8>, Vec<u8>)> = None;
        if kind == Kind::Video && f.keyframe {
            let ns = nals(&f.data);
            let sps = ns.iter().find(|n| n[0] & 0x1f == 7).map(|n| n.to_vec());
            let pps = ns.iter().find(|n| n[0] & 0x1f == 8).map(|n| n.to_vec());
            if let (Some(s), Some(p)) = (sps, pps) {
                params = Some((s, p));
            }
        }
        // ★이미 가진 매개변수와 다르면(첫 조각을 쓰기 전이라도) 그 자리에서 끊는다 — 앞 프레임은 앞 매개변수로 init 에 든다
        let changed = match (&params, g.video.as_ref()) {
            (Some((s, p)), Some(t)) => t.sps.is_some() && (t.sps.as_deref() != Some(s) || t.pps.as_deref() != Some(p)),
            _ => false,
        };
        let (ticks, sample_key) = {
            let Some(t) = (match kind {
                Kind::Video => g.video.as_mut(),
                Kind::Audio => g.audio.as_mut(),
            }) else {
                return Ok(());
            };
            if t.first_ts.is_none() {
                t.first_log = f.log_time;
            }
            let ticks = t.advance(f.rtp_ts);
            // ★같은 순간의 그림이 층만 바뀌어 다시 오면(시뮬캐스트 층 바꿈 — 서버가 새 층 키프레임을 앞 프레임 바로 뒤 시각으로 잇는다 · 2층 실측 +90 틱 = 1ms)
            //   앞 견본을 버리고 새 키프레임으로 바꿔 낀다 — 판정은 「한 프레임 간격의 절반보다 가까이 붙은 키프레임」.
            //   두지 않으면 견본 하나짜리 1ms 조각이 생긴다(2층 peer_record_hls_simulcast 실측 20261110 seg-00001 0.001초 — 202611/20261110d T4)
            let gap = t.buf.len().checked_sub(2).map(|i| t.buf[i + 1].ticks.saturating_sub(t.buf[i].ticks)).unwrap_or(if t.last_dur > 0 { t.last_dur } else { 3000 });
            if kind == Kind::Video && f.keyframe && t.buf.last().is_some_and(|s| ticks.saturating_sub(s.ticks) * 2 < gap) {
                t.buf.pop();
                g.out.replaced += 1;
            }
            if t.first_utc.is_none()
                && let Some(u) = f.utc_ns
            {
                t.first_utc = Some((u, ticks));
            }
            (ticks, kind == Kind::Audio || f.keyframe)
        };
        // 자르기 — 이끄는 트랙의 이 프레임이 조각 시작에서 T 지난 키프레임이면(또는 매개변수가 바뀌었으면) 이 프레임 앞에서
        if kind == lead {
            // 이끄는 트랙이 바뀌었으면(음성만이던 묶음에 영상이 듦) 그 트랙 틱으로 다시 잰다
            if g.seg_start.is_none_or(|(k, _)| k != lead) {
                g.seg_start = Some((lead, ticks));
            }
            let start = g.seg_start.map(|(_, t)| t).unwrap_or(ticks);
            let elapsed = ticks.saturating_sub(start);
            if (sample_key && elapsed >= tgt_ticks(lead)) || changed {
                flush(g, &self.cfg, self.log.as_deref(), target, Some((lead, ticks)))?;
                if changed {
                    g.reinit = true;
                }
            }
            // T − 1초에 키프레임 요청(영상이 이끌 때 · 조각마다 한 번)
            if kind == Kind::Video && !g.asked && elapsed + Kind::Video.timescale() >= tgt_ticks(Kind::Video) {
                g.asked = true;
                g.out.keyframe_requests += 1;
                self.wants.push(f.track_id.clone());
            }
        }
        let Some(t) = (match kind {
            Kind::Video => g.video.as_mut(),
            Kind::Audio => g.audio.as_mut(),
        }) else {
            return Ok(());
        };
        if let Some((s, p)) = params
            && (changed || t.sps.is_none())
        {
            t.dims = h264_dims(&s).unwrap_or(t.dims);
            t.sps = Some(s);
            t.pps = Some(p);
        }
        let data = match kind {
            Kind::Video => avc_sample(&f.data),
            Kind::Audio => f.data.clone(),
        };
        t.buf.push(Sample { ticks, key: sample_key, data });
        Ok(())
    }

    /// 끝 — 열린 묶음을 마지막 조각까지 쓰고 `ENDLIST` 를 단다(파이프라인 `finish` 의 세그먼트 닫힘을 먼저 넣는다)
    pub fn finish(&mut self) -> io::Result<()> {
        let target = self.target_duration();
        let keys: Vec<String> = self.groups.keys().cloned().collect();
        for k in keys {
            if let Some(mut g) = self.groups.remove(&k) {
                if g.out.refused.is_none() {
                    flush(&mut g, &self.cfg, self.log.as_deref(), target, None)?;
                    g.ended = true;
                    playlists(&mut g, &self.cfg, target)?;
                }
                self.done.push(g.out);
            }
        }
        Ok(())
    }
}

/// 버퍼를 조각 하나로 쓴다 — `next` = 다음 조각을 여는 이끄는 트랙의 틱(없으면 마지막 샘플 끝까지)
fn flush(g: &mut Group, cfg: &HlsConfig, log: Option<&crate::log::Logger>, target: u64, next: Option<(Kind, u64)>) -> io::Result<()> {
    let has = g.video.as_ref().is_some_and(|t| !t.buf.is_empty()) || g.audio.as_ref().is_some_and(|t| !t.buf.is_empty());
    if !has {
        g.seg_start = next;
        g.asked = false;
        return Ok(());
    }
    std::fs::create_dir_all(&g.dir)?;
    // 시각 기준 — 처음 쓸 때(SDK:RULE-PLAT-REC-HLS 시각)
    if g.base.is_none() {
        let tracks: Vec<&Trk> = [g.video.as_ref(), g.audio.as_ref()].into_iter().flatten().filter(|t| t.first_ts.is_some()).collect();
        let utc = !tracks.is_empty() && tracks.iter().all(|t| t.first_utc.is_some());
        let t0 = |t: &Trk, k: Kind| -> i128 {
            match (utc, t.first_utc) {
                (true, Some((u, at))) => i128::from(u) - i128::from(at) * 1_000_000_000 / i128::from(k.timescale()),
                _ => i128::from(t.first_log),
            }
        };
        let mut origin = i128::MAX;
        let mut lt = 0i128;
        for (k, t) in [(Kind::Video, g.video.as_ref()), (Kind::Audio, g.audio.as_ref())] {
            if let Some(t) = t.filter(|t| t.first_ts.is_some()) {
                let v = t0(t, k);
                if v < origin {
                    origin = v;
                    lt = v - i128::from(t.first_log);
                }
            }
        }
        g.base = Some((utc, origin, if utc { lt } else { 0 }));
        g.out.base = Some(if utc { "utc" } else { "log_time" });
    }
    let (utc, origin, lt) = g.base.unwrap_or((false, 0, 0));
    if !utc
        && g.vshift.is_none()
        && let (Some(v), Some(a)) = (g.video.as_ref(), g.audio.as_ref())
        && let (Some((vu, vat)), Some((au, aat))) = (v.first_utc, a.first_utc)
    {
        let ns = super::pipeline::late_sr_shift_ns((v.first_log, vu, vat, Kind::Video.timescale()), (a.first_log, au, aat, Kind::Audio.timescale()));
        g.vshift = Some((ns * i128::from(Kind::Video.timescale()) / 1_000_000_000) as i64);
        g.out.video_shift_us = Some((ns / 1_000) as i64);
    }
    for (k, t) in [(Kind::Video, g.video.as_mut()), (Kind::Audio, g.audio.as_mut())] {
        if let Some(t) = t
            && t.t0.is_none()
            && t.first_ts.is_some()
        {
            t.t0 = Some(match (utc, t.first_utc) {
                (true, Some((u, at))) => i128::from(u) - i128::from(at) * 1_000_000_000 / i128::from(k.timescale()),
                (true, None) => i128::from(t.first_log) + lt,
                _ => i128::from(t.first_log),
            });
        }
    }
    // init — 처음 · 트랙이 들거나 빠짐 · 매개변수가 바뀜
    if g.init.is_none() || g.reinit {
        let disc = g.init.is_some();
        let path = g.dir.join(format!("init-{:03}.mp4", g.init_n));
        g.init_n += 1;
        write_atomic(&path, &init_segment(g.video.as_ref(), g.audio.as_ref()))?;
        if let Some(t) = g.video.as_mut() {
            t.in_init = true;
        }
        if let Some(t) = g.audio.as_mut() {
            t.in_init = true;
        }
        g.out.inits.push(path.clone());
        g.init = Some(path);
        g.reinit = false;
        g.disc_next |= disc;
    }
    // 이끄는 트랙의 길이
    let lead_video = g.video.as_ref().is_some_and(|t| !t.buf.is_empty());
    let n = g.out.segments.len();
    let file = g.dir.join(format!("seg-{n:05}.m4s"));
    let mut trafs: Vec<Traf> = Vec::new();
    let mut lead_dur = 0u64;
    for (k, t) in [(Kind::Video, g.video.as_mut()), (Kind::Audio, g.audio.as_mut())] {
        let Some(t) = t else { continue };
        if t.buf.is_empty() {
            continue;
        }
        let off = ((t.t0.unwrap_or(origin) - origin).max(0) * i128::from(k.timescale()) / 1_000_000_000) as u64;
        let buf = std::mem::take(&mut t.buf);
        let step = (super::pipeline::LATE_SR_STEP_NS * i128::from(Kind::Video.timescale()) / 1_000_000_000) as i64;
        let shift0 = g.vshift_at;
        let mut rows = Vec::with_capacity(buf.len());
        let mut data = Vec::new();
        for (i, s) in buf.iter().enumerate() {
            let dur = match k {
                Kind::Audio => opus_samples(&s.data),
                Kind::Video => {
                    let base = match buf.get(i + 1) {
                        Some(nx) => nx.ticks.saturating_sub(s.ticks),
                        None => match next.filter(|(nk, _)| *nk == Kind::Video) {
                            Some((_, nt)) => nt.saturating_sub(s.ticks),
                            None => t.last_dur.max(1),
                        },
                    };
                    let now = g.vshift_at;
                    if let Some(target) = g.vshift {
                        g.vshift_at = super::pipeline::late_sr_step(i128::from(now), i128::from(target), i128::from(step)) as i64;
                    }
                    base.saturating_add_signed(g.vshift_at - now).max(1)
                }
            };
            if dur > 0 {
                t.last_dur = dur;
            }
            rows.push((dur as u32, s.data.len() as u32, s.key));
            data.extend_from_slice(&s.data);
        }
        let span: u64 = rows.iter().map(|r| u64::from(r.0)).sum();
        if (k == Kind::Video) == lead_video {
            lead_dur = span * 1_000_000 / k.timescale();
        }
        let tfdt = match (k, t.next_dts) {
            (Kind::Audio, Some(n)) => n,
            _ => (off + buf[0].ticks).saturating_add_signed(shift0),
        };
        if k == Kind::Audio {
            t.next_dts = Some(tfdt + span);
        }
        trafs.push(Traf { kind: k, tfdt, rows, data });
    }
    write_atomic(&file, &media_segment(n as u32 + 1, &trafs))?;
    let duration = lead_dur as f64 / 1_000_000.0;
    if duration.round() as u64 > target {
        g.out.over_target += 1;
        if let Some(l) = log {
            l.emit(
                crate::log::Line::new(crate::log::Kind::Diag)
                    .level(crate::log::Level::Warn)
                    .spec("SD-RULE-PLAT-REC-HLS")
                    .key("in.stream", &g.out.key)
                    .key("in.segment", n)
                    .key("in.duration_ms", lead_dur / 1_000)
                    .key("in.target_s", target)
                    .msg("hls segment over target duration - video keyframe came late"),
            );
        }
    }
    g.out.segments.push(HlsSegment { file, init: g.init.clone().unwrap_or_default(), discontinuity: std::mem::take(&mut g.disc_next), duration });
    g.seg_start = next;
    g.asked = false;
    playlists(g, cfg, target)
}

/// 재생 목록 둘 — 전량(EVENT) · 최근 창(live)
fn playlists(g: &mut Group, cfg: &HlsConfig, target: u64) -> io::Result<()> {
    if g.out.segments.is_empty() {
        return Ok(());
    }
    let name = |p: &Path| p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    let body = |segs: &[HlsSegment], first: usize, disc_seq: usize, event: bool| -> String {
        let mut s = format!("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-INDEPENDENT-SEGMENTS\n");
        if event {
            s.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n");
        }
        s.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-DISCONTINUITY-SEQUENCE:{disc_seq}\n"));
        let mut init: Option<&PathBuf> = None;
        for x in segs {
            if x.discontinuity {
                s.push_str("#EXT-X-DISCONTINUITY\n");
            }
            if init != Some(&x.init) {
                s.push_str(&format!("#EXT-X-MAP:URI=\"{}\"\n", name(&x.init)));
                init = Some(&x.init);
            }
            s.push_str(&format!("#EXTINF:{:.3},\n{}\n", x.duration, name(&x.file)));
        }
        if g.ended {
            s.push_str("#EXT-X-ENDLIST\n");
        }
        s
    };
    let all = &g.out.segments;
    write_atomic(&g.out.event_playlist, body(all, 0, 0, true).as_bytes())?;
    let first = all.len().saturating_sub(cfg.window.max(1));
    // 창 밖으로 빠진 조각의 끊김 표시 수(RFC 8216 §6.2.2)
    let disc_seq = all[..first].iter().filter(|x| x.discontinuity).count();
    write_atomic(&g.out.live_playlist, body(&all[first..], first, disc_seq, false).as_bytes())
}

/// 다 쓴 뒤 바꿔 단다(읽는 쪽이 반쯤 쓴 목록을 보지 않는다)
fn write_atomic(path: &Path, b: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("x")));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(b)?;
    }
    std::fs::rename(tmp, path)
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect()
}

/// Annex-B 프레임 → 길이 앞붙임 샘플 — SPS · PPS · AUD 는 뺀다(매개변수는 init 의 avcC 에만 — `avc1`)
fn avc_sample(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    for n in nals(b) {
        if matches!(n[0] & 0x1f, 7..=9) {
            continue;
        }
        out.extend_from_slice(&(n.len() as u32).to_be_bytes());
        out.extend_from_slice(n);
    }
    out
}

// ── ISO BMFF 상자(ISO/IEC 14496-12 · CMAF) ──

fn bx(t: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(t);
    v.extend_from_slice(body);
    v
}

fn fbx(t: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version];
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    bx(t, &b)
}

fn u16b(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn u32b(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// init 조각 — ftyp + moov(트랙마다 trak · mvex/trex)
fn init_segment(video: Option<&Trk>, audio: Option<&Trk>) -> Vec<u8> {
    let ftyp = bx(b"ftyp", &[b"iso6".as_slice(), &u32b(0), b"iso6", b"cmfc", b"mp41"].concat());
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&[0u8; 8]);
    mvhd.extend_from_slice(&u32b(1000));
    mvhd.extend_from_slice(&u32b(0));
    mvhd.extend_from_slice(&u32b(0x0001_0000));
    mvhd.extend_from_slice(&u16b(0x0100));
    mvhd.extend_from_slice(&[0u8; 10]);
    for m in MATRIX {
        mvhd.extend_from_slice(&u32b(m));
    }
    mvhd.extend_from_slice(&[0u8; 24]);
    mvhd.extend_from_slice(&u32b(3));
    let mut moov = fbx(b"mvhd", 0, 0, &mvhd);
    let mut trex = Vec::new();
    if let Some(t) = video.filter(|t| t.first_ts.is_some()) {
        moov.extend(trak(Kind::Video, t));
        trex.extend(fbx(b"trex", 0, 0, &[u32b(Kind::Video.track_id()), u32b(1), u32b(0), u32b(0), u32b(0)].concat()));
    }
    if let Some(t) = audio.filter(|t| t.first_ts.is_some()) {
        moov.extend(trak(Kind::Audio, t));
        trex.extend(fbx(b"trex", 0, 0, &[u32b(Kind::Audio.track_id()), u32b(1), u32b(0), u32b(0), u32b(0)].concat()));
    }
    moov.extend(bx(b"mvex", &trex));
    [ftyp, bx(b"moov", &moov)].concat()
}

fn trak(k: Kind, t: &Trk) -> Vec<u8> {
    let (w, h) = t.dims;
    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&[0u8; 8]);
    tkhd.extend_from_slice(&u32b(k.track_id()));
    tkhd.extend_from_slice(&[0u8; 4]);
    tkhd.extend_from_slice(&u32b(0));
    tkhd.extend_from_slice(&[0u8; 8]);
    tkhd.extend_from_slice(&u16b(0));
    tkhd.extend_from_slice(&u16b(0));
    tkhd.extend_from_slice(&u16b(if k == Kind::Audio { 0x0100 } else { 0 }));
    tkhd.extend_from_slice(&[0u8; 2]);
    for m in MATRIX {
        tkhd.extend_from_slice(&u32b(m));
    }
    tkhd.extend_from_slice(&u32b((w as u32) << 16));
    tkhd.extend_from_slice(&u32b((h as u32) << 16));
    let tkhd = fbx(b"tkhd", 0, 3, &tkhd);
    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&[0u8; 8]);
    mdhd.extend_from_slice(&u32b(k.timescale() as u32));
    mdhd.extend_from_slice(&u32b(0));
    mdhd.extend_from_slice(&u16b(0x55C4));
    mdhd.extend_from_slice(&u16b(0));
    let mdhd = fbx(b"mdhd", 0, 0, &mdhd);
    let (htype, hname): (&[u8; 4], &[u8]) = match k {
        Kind::Video => (b"vide", b"oxpeer video\0"),
        Kind::Audio => (b"soun", b"oxpeer audio\0"),
    };
    let hdlr = fbx(b"hdlr", 0, 0, &[&u32b(0)[..], htype, &[0u8; 12], hname].concat());
    let mhd = match k {
        Kind::Video => fbx(b"vmhd", 0, 1, &[0u8; 8]),
        Kind::Audio => fbx(b"smhd", 0, 0, &[0u8; 4]),
    };
    let dinf = bx(b"dinf", &fbx(b"dref", 0, 0, &[u32b(1).as_slice(), &fbx(b"url ", 0, 1, &[])].concat()));
    let entry = match k {
        Kind::Video => {
            let mut e = vec![0u8; 6];
            e.extend_from_slice(&u16b(1));
            e.extend_from_slice(&[0u8; 16]);
            e.extend_from_slice(&u16b(w as u16));
            e.extend_from_slice(&u16b(h as u16));
            e.extend_from_slice(&u32b(0x0048_0000));
            e.extend_from_slice(&u32b(0x0048_0000));
            e.extend_from_slice(&u32b(0));
            e.extend_from_slice(&u16b(1));
            e.extend_from_slice(&[0u8; 32]);
            e.extend_from_slice(&u16b(0x0018));
            e.extend_from_slice(&u16b(0xFFFF));
            e.extend(bx(b"avcC", &avcc(t.sps.as_deref(), t.pps.as_deref())));
            bx(b"avc1", &e)
        }
        Kind::Audio => {
            let mut e = vec![0u8; 6];
            e.extend_from_slice(&u16b(1));
            e.extend_from_slice(&[0u8; 8]);
            e.extend_from_slice(&u16b(1));
            e.extend_from_slice(&u16b(16));
            e.extend_from_slice(&[0u8; 4]);
            e.extend_from_slice(&u32b(48_000 << 16));
            // dOps(Opus in ISOBMFF §4.3.2) — 판 0 · 출력 채널 1 · pre-skip 0 · 입력 표본률 48000 · 이득 0 · 채널 사상 0
            let mut d = vec![0u8, 1];
            d.extend_from_slice(&u16b(0));
            d.extend_from_slice(&u32b(48_000));
            d.extend_from_slice(&u16b(0));
            d.push(0);
            e.extend(bx(b"dOps", &d));
            bx(b"Opus", &e)
        }
    };
    let stsd = fbx(b"stsd", 0, 0, &[u32b(1).as_slice(), &entry].concat());
    let empty = |t: &[u8; 4]| fbx(t, 0, 0, &u32b(0));
    let stsz = fbx(b"stsz", 0, 0, &[u32b(0), u32b(0)].concat());
    let stbl = bx(b"stbl", &[stsd, empty(b"stts"), empty(b"stsc"), stsz, empty(b"stco")].concat());
    let minf = bx(b"minf", &[mhd, dinf, stbl].concat());
    let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
    bx(b"trak", &[tkhd, mdia].concat())
}

/// 미디어 조각 — styp + moof(트랙마다 traf: tfhd · tfdt · trun) + mdat
fn media_segment(seq: u32, trafs: &[Traf]) -> Vec<u8> {
    let styp = bx(b"styp", &[b"msdh".as_slice(), &u32b(0), b"msdh", b"msix"].concat());
    // moof 크기를 먼저 알아야 trun 의 data_offset 을 쓴다 — 두 번 짓는다(자리 값만 다르다)
    let build = |offsets: &[u32]| -> Vec<u8> {
        let mut moof = fbx(b"mfhd", 0, 0, &u32b(seq));
        for (i, t) in trafs.iter().enumerate() {
            let k = t.kind;
            let tfhd = fbx(b"tfhd", 0, 0x02_0000, &u32b(k.track_id()));
            let tfdt = fbx(b"tfdt", 1, 0, &t.tfdt.to_be_bytes());
            let mut tr = u32b(t.rows.len() as u32).to_vec();
            tr.extend_from_slice(&u32b(offsets[i]));
            for (d, s, key) in &t.rows {
                tr.extend_from_slice(&u32b(*d));
                tr.extend_from_slice(&u32b(*s));
                let flags: u32 = if k == Kind::Audio || *key { 0x0200_0000 } else { 0x0101_0000 };
                tr.extend_from_slice(&u32b(flags));
            }
            let trun = fbx(b"trun", 0, 0x00_0001 | 0x00_0100 | 0x00_0200 | 0x00_0400, &tr);
            moof.extend(bx(b"traf", &[tfhd, tfdt, trun].concat()));
        }
        bx(b"moof", &moof)
    };
    let probe = build(&vec![0; trafs.len()]);
    let mut offsets = Vec::with_capacity(trafs.len());
    let mut at = probe.len() as u32 + 8;
    for t in trafs {
        offsets.push(at);
        at += t.data.len() as u32;
    }
    let moof = build(&offsets);
    let mdat: Vec<u8> = trafs.iter().flat_map(|t| t.data.iter().copied()).collect();
    [styp, moof, bx(b"mdat", &mdat)].concat()
}
