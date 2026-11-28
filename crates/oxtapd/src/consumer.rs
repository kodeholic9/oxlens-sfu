// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · SDK:RULE-PLAT-REC-API · 정:RULE-OPS-REC-MKV · model: claude-opus-5-5
//! 결과물 소비자 — trait `RecConsumer`(지점 셋 `record` · `frame` · `segment` 의 형 그대로 · `finish` · `recover`) · 등록부 `Registry`(기본 셋 `talk` · `tracks` · `raw` 를 든 채 시작 · `Registry::register` 로 같은 이름은 바꾸고 새 이름은 더한다 · 빼지 못한다) · 기본 소비자 셋(발언 파일 · 개인 트랙 녹화 · 원시 기록) · 기동 회수(살아 있는 실행 폴더는 건너뜀 · 제 종류의 쓰는 중 조각만 마감해 닫힌 경로로 · 녹취를 지우지 않는다 — 미디어 0 조각만).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use common::talk::Report;
use oxpeer::record::{Frame, Output, Record, Segment, SegmentKind};
use tokio::sync::mpsc;

use crate::files::{self, LOCK_FILE, Layout};
use crate::sink::{MkvFile, MkvSink, OggFile, OggSink, Recovered};

pub const SPEC: &str = "SV-IMPL-OPS-REC-UNIT";
pub const OGG_MIME: &str = "audio/ogg";
pub const MKV_MIME: &str = "video/x-matroska";

pub struct RoomCtx {
    pub room: String,
    pub run: String,
    pub layout: Layout,
    pub reports: mpsc::UnboundedSender<Report>,
}

impl RoomCtx {
    pub fn staging(&self) -> PathBuf {
        self.layout.run_dir(&self.run)
    }
}

pub struct RecoverCtx {
    pub layout: Layout,
    pub runs: Vec<PathBuf>,
    pub reports: mpsc::UnboundedSender<Report>,
    pub reason: &'static str,
}

pub trait RecConsumer: Send {
    fn record(&mut self, _r: &Record) -> Result<(), String> {
        Ok(())
    }

    fn frame(&mut self, _f: &Frame) -> Result<(), String> {
        Ok(())
    }

    fn segment(&mut self, _s: &Segment) -> Result<(), String> {
        Ok(())
    }

    fn finish(&mut self, reason: &str) -> Result<(), String>;

    fn keyframe_requests(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn raw_dir(&self) -> Option<PathBuf> {
        None
    }

    fn recover(_ctx: &RecoverCtx)
    where
        Self: Sized,
    {
    }
}

type Make = Box<dyn Fn(&RoomCtx) -> io::Result<Box<dyn RecConsumer>> + Send + Sync>;

struct Kind {
    make: Make,
    recover: fn(&RecoverCtx),
}

pub struct Registry {
    kinds: BTreeMap<String, Kind>,
}

impl Registry {
    pub fn defaults() -> Registry {
        let mut r = Registry { kinds: BTreeMap::new() };
        r.register::<TalkFiles>("talk", TalkFiles::open);
        r.register::<TrackFiles>("tracks", TrackFiles::open);
        r.register::<RawTap>("raw", RawTap::open);
        r
    }

    pub fn register<C: RecConsumer + 'static>(&mut self, name: &str, make: impl Fn(&RoomCtx) -> io::Result<C> + Send + Sync + 'static) {
        if name.is_empty() {
            return;
        }
        let make: Make = Box::new(move |c| make(c).map(|x| Box::new(x) as Box<dyn RecConsumer>));
        self.kinds.insert(name.to_string(), Kind { make, recover: C::recover });
    }

    pub fn names(&self) -> Vec<String> {
        self.kinds.keys().cloned().collect()
    }

    pub fn open(&self, name: &str, ctx: &RoomCtx) -> Option<io::Result<Box<dyn RecConsumer>>> {
        self.kinds.get(name).map(|k| (k.make)(ctx))
    }

    pub fn recover(&self, ctx: &RecoverCtx) {
        let mut held = Vec::new();
        let mut runs = Vec::new();
        for run in &ctx.runs {
            match files::lock_run(run) {
                Ok(Some(l)) => {
                    held.push(l);
                    runs.push(run.clone());
                }
                Ok(None) => tracing::info!(kind = "diag", spec = SPEC, path = %run.display(), "rec run folder in use by a live unit — not recovered"),
                Err(e) => {
                    tracing::warn!(kind = "diag", spec = SPEC, path = %run.display(), "rec run folder lock not supported here: {e} — recovered without a lock");
                    runs.push(run.clone());
                }
            }
        }
        let mine = RecoverCtx { layout: ctx.layout.clone(), runs: runs.clone(), reports: ctx.reports.clone(), reason: ctx.reason };
        for (name, k) in &self.kinds {
            if catch_unwind(AssertUnwindSafe(|| (k.recover)(&mine))).is_err() {
                tracing::error!(kind = "diag", spec = SPEC, rec_kind = %name, "rec recovery panicked — its fragments stay for the next start");
            }
        }
        for run in &runs {
            let _ = std::fs::remove_file(run.join(LOCK_FILE));
        }
        drop(held);
        for run in &runs {
            for f in files::files_under(run) {
                tracing::warn!(kind = "diag", spec = SPEC, path = %rel(&ctx.layout, &f), "rec fragment left after recovery — no kind closed it");
            }
            files::prune_empty(run);
        }
    }
}

pub fn closed_line(room: &str, rec_kind: &str, talk: Option<&str>, path: &str, bytes: u64, reason: &str) {
    tracing::info!(kind = "diag", spec = SPEC, room = %room, rec_kind = %rec_kind, talk_id = talk.unwrap_or("-"), path = %path, bytes, reason = %reason, "rec closed");
}

fn reason_of(r: Option<&str>) -> String {
    r.unwrap_or("end").to_string()
}

pub fn rel(layout: &Layout, p: &Path) -> String {
    p.strip_prefix(&layout.root).unwrap_or(p).to_string_lossy().into_owned()
}

fn send(reports: &mpsc::UnboundedSender<Report>, r: Report) {
    let talk = r.talk_id().to_string();
    if reports.send(r).is_err() {
        tracing::warn!(kind = "diag", spec = SPEC, talk_id = %talk, "rec report not queued — the report line is gone");
    }
}

fn drop_file(p: &Path, why: &str) {
    match std::fs::remove_file(p) {
        Ok(()) => tracing::info!(kind = "diag", spec = SPEC, path = %p.display(), "rec discarded: {why}"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(kind = "diag", spec = SPEC, path = %p.display(), "rec discard ({why}): {e}"),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn settle_talk(layout: &Layout, room: &str, path: &Path, name: &str, talk: Option<&str>, mime: &str, duration_ms: u64, reason: &str, reported: &mut HashSet<String>) -> io::Result<Option<Report>> {
    let to = files::link_no_overwrite(path, &layout.closed("rec", room), name)?;
    let size = std::fs::metadata(&to)?.len();
    let at = rel(layout, &to);
    closed_line(room, "talk", talk, &at, size, reason);
    let Some(talk) = talk else {
        tracing::warn!(kind = "diag", spec = SPEC, room = %room, path = %at, "rec talk file without a talk id — kept on disk, not reported");
        return Ok(None);
    };
    if !reported.insert(talk.to_string()) {
        tracing::warn!(kind = "diag", spec = SPEC, room = %room, talk_id = %talk, path = %at, "rec second file of one talk — kept on disk only, the first is the talk's record");
        return Ok(None);
    }
    Ok(Some(Report::Rec { talk_id: talk.to_string(), path: at, size, mime: mime.into(), duration_ms }))
}

pub struct TalkFiles {
    room: String,
    layout: Layout,
    reports: mpsc::UnboundedSender<Report>,
    ogg: OggSink,
    mkv: MkvSink,
    slot: HashMap<u64, String>,
    open: HashMap<String, u32>,
    video_talks: HashSet<String>,
    video_kept: HashSet<String>,
    gap: HashSet<String>,
    pending: Vec<OggFile>,
    reported: HashSet<String>,
    ogg_seen: usize,
    mkv_seen: usize,
}

impl TalkFiles {
    pub fn open(ctx: &RoomCtx) -> io::Result<TalkFiles> {
        let dir = ctx.staging().join("rec").join(&ctx.room);
        Ok(TalkFiles {
            room: ctx.room.clone(),
            layout: ctx.layout.clone(),
            reports: ctx.reports.clone(),
            ogg: OggSink::new(&dir)?,
            mkv: MkvSink::new(&dir)?,
            slot: HashMap::new(),
            open: HashMap::new(),
            video_talks: HashSet::new(),
            video_kept: HashSet::new(),
            gap: HashSet::new(),
            pending: Vec::new(),
            reported: HashSet::new(),
            ogg_seen: 0,
            mkv_seen: 0,
        })
    }

    fn write(&mut self, o: &Output) -> Result<(), String> {
        let a = self.ogg.write(o).map_err(|e| format!("talk ogg: {e}"));
        let b = self.mkv.write(o).map_err(|e| format!("talk mkv: {e}"));
        a.and(b)
    }

    fn opened(&mut self, t: &str, delta: i64) {
        let n = self.open.get(t).copied().unwrap_or(0);
        let next = if delta >= 0 { n.saturating_add(1) } else { n.saturating_sub(1) };
        self.open.insert(t.to_string(), next);
    }

    fn settle(&mut self) {
        let mkv: Vec<MkvFile> = self.mkv.files()[self.mkv_seen..].to_vec();
        self.mkv_seen += mkv.len();
        for f in mkv {
            let Some(t) = f.talk.clone() else {
                drop_file(&f.path, "slot file without a talk id");
                continue;
            };
            if f.owner.is_none() && f.tracks.iter().any(|x| x.kind == "video") {
                self.video_kept.insert(t.clone());
                let reason = reason_of(f.reason.as_deref());
                self.close(&f.path, Some(t), MKV_MIME, f.duration_ms, &reason);
            } else {
                if self.video_talks.contains(&t) {
                    self.gap.insert(t.clone());
                }
                drop_file(&f.path, "audio-only matroska of a talk — the talk's ogg holds the audio");
            }
        }
        let ogg: Vec<OggFile> = self.ogg.files()[self.ogg_seen..].to_vec();
        self.ogg_seen += ogg.len();
        self.pending.extend(ogg);
        let pending = std::mem::take(&mut self.pending);
        for f in pending {
            let Some(t) = f.talk.clone() else {
                drop_file(&f.path, "slot file without a talk id");
                continue;
            };
            if self.open.get(&t).copied().unwrap_or(0) > 0 {
                self.pending.push(f);
                continue;
            }
            if self.video_kept.contains(&t) && !self.gap.contains(&t) {
                drop_file(&f.path, "the talk's matroska holds its audio");
                continue;
            }
            let reason = reason_of(f.reason.as_deref());
            self.close(&f.path, Some(t), OGG_MIME, f.samples / 48, &reason);
        }
    }

    fn close(&mut self, path: &Path, talk: Option<String>, mime: &str, duration_ms: u64, reason: &str) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match settle_talk(&self.layout, &self.room, path, &name, talk.as_deref(), mime, duration_ms, reason, &mut self.reported) {
            Ok(Some(r)) => send(&self.reports, r),
            Ok(None) => {}
            Err(e) => tracing::warn!(kind = "diag", spec = SPEC, room = %self.room, path = %path.display(), "rec settle: {e} — left for recovery"),
        }
    }
}

impl RecConsumer for TalkFiles {
    fn segment(&mut self, s: &Segment) -> Result<(), String> {
        if s.event == SegmentKind::Opened {
            if s.owner.is_some() {
                return Ok(());
            }
            let Some(t) = s.talk.clone() else {
                tracing::warn!(kind = "diag", spec = SPEC, room = %self.room, track_id = %s.track_id, "rec slot segment without a talk id — not recorded");
                return Ok(());
            };
            if s.kind == "video" {
                self.video_talks.insert(t.clone());
            }
            self.opened(&t, 1);
            self.slot.insert(s.id, t);
        }
        let Some(t) = self.slot.get(&s.id).cloned() else { return Ok(()) };
        let r = self.write(&Output::Segment(s.clone()));
        if s.event == SegmentKind::Closed {
            self.slot.remove(&s.id);
            self.opened(&t, -1);
            self.settle();
        }
        r
    }

    fn frame(&mut self, f: &Frame) -> Result<(), String> {
        match f.segment {
            Some(id) if self.slot.contains_key(&id) => self.write(&Output::Frame(f.clone())),
            _ => Ok(()),
        }
    }

    fn finish(&mut self, reason: &str) -> Result<(), String> {
        let a = self.ogg.close(Some(reason)).map_err(|e| e.to_string());
        let b = self.mkv.close(Some(reason)).map_err(|e| e.to_string());
        self.slot.clear();
        self.open.clear();
        self.settle();
        a.and(b)
    }

    fn keyframe_requests(&mut self) -> Vec<String> {
        self.mkv.take_keyframe_requests()
    }

    fn recover(ctx: &RecoverCtx) {
        let mut reported = HashSet::new();
        for run in &ctx.runs {
            let mut groups: BTreeMap<(String, Option<String>), Vec<PathBuf>> = BTreeMap::new();
            for p in files::files_under(&run.join("rec")) {
                if !p.extension().is_some_and(|e| e == "ogg" || e == "mkv") {
                    continue;
                }
                let Some(room) = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()) else { continue };
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let talk = files::talk_of(&crate::sink::safe_name(&room), &name);
                groups.entry((room, talk)).or_default().push(p);
            }
            for ((room, talk), paths) in groups {
                let mut done: Vec<(PathBuf, Recovered, u64)> = Vec::new();
                for p in paths {
                    let r = match p.extension().and_then(|e| e.to_str()) {
                        Some("ogg") => crate::sink::recover_ogg(&p),
                        _ => crate::sink::recover_mkv(&p),
                    };
                    if let Some((rec, size)) = recovered(&ctx.layout, &room, "talk", &p, r) {
                        done.push((p, rec, size));
                    }
                }
                let has_ogg = done.iter().any(|(p, _, _)| p.extension().is_some_and(|e| e == "ogg"));
                done.sort_by_key(|(p, rec, _)| (!rec.video, p.extension().is_none_or(|e| e != "ogg"), p.clone()));
                for (p, rec, size) in done {
                    let is_mkv = p.extension().is_some_and(|e| e == "mkv");
                    if is_mkv && !rec.video && has_ogg {
                        drop_file(&p, "audio-only matroska of a talk — the talk's ogg holds the audio");
                        continue;
                    }
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mime = if is_mkv { MKV_MIME } else { OGG_MIME };
                    match settle_talk(&ctx.layout, &room, &p, &name, talk.as_deref(), mime, rec.duration_ms, ctx.reason, &mut reported) {
                        Ok(rep) => {
                            recovered_line(&ctx.layout, &room, "talk", talk.as_deref(), &p, size, ctx.reason);
                            if let Some(r) = rep {
                                send(&ctx.reports, r);
                            }
                        }
                        Err(e) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, path = %p.display(), "rec recover settle: {e}"),
                    }
                }
            }
        }
    }
}

fn recovered_line(layout: &Layout, room: &str, rec_kind: &str, talk: Option<&str>, from: &Path, bytes: u64, reason: &str) {
    tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %rec_kind, talk_id = talk.unwrap_or("-"), path = %rel(layout, from), bytes, reason = %reason, "rec recovered");
}

fn recovered(layout: &Layout, room: &str, rec_kind: &str, p: &Path, r: io::Result<Recovered>) -> Option<(Recovered, u64)> {
    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    match r {
        Ok(done) if done.media_frames == 0 => {
            tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %rec_kind, path = %rel(layout, p), bytes = size, reason = "empty", "rec empty removed");
            let _ = std::fs::remove_file(p);
            None
        }
        Ok(done) => Some((done, size)),
        Err(e) => {
            match files::to_failed(layout, p) {
                Ok(to) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %rec_kind, path = %rel(layout, &to), "rec recover failed: {e} — moved to failed"),
                Err(m) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, rec_kind = %rec_kind, path = %rel(layout, p), "rec recover failed: {e} — not moved: {m}"),
            }
            None
        }
    }
}

pub struct TrackFiles {
    room: String,
    run: String,
    layout: Layout,
    mkv: MkvSink,
    own: HashSet<u64>,
    seen: usize,
}

impl TrackFiles {
    pub fn open(ctx: &RoomCtx) -> io::Result<TrackFiles> {
        let dir = ctx.staging().join("tracks").join(&ctx.room);
        Ok(TrackFiles { room: ctx.room.clone(), run: ctx.run.clone(), layout: ctx.layout.clone(), mkv: MkvSink::new(&dir)?, own: HashSet::new(), seen: 0 })
    }

    fn settle(&mut self) {
        let done: Vec<MkvFile> = self.mkv.files()[self.seen..].to_vec();
        self.seen += done.len();
        for f in done {
            let reason = reason_of(f.reason.as_deref());
            if let Err(e) = settle_track(&self.layout, &self.room, &self.run, &f.path, &reason) {
                tracing::warn!(kind = "diag", spec = SPEC, room = %self.room, path = %f.path.display(), "rec settle: {e} — left for recovery");
            }
        }
    }
}

pub fn settle_track(layout: &Layout, room: &str, run: &str, path: &Path, reason: &str) -> io::Result<PathBuf> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let to = files::link_no_overwrite(path, &layout.closed("tracks", room), &files::tracks_closed_name(&name, run))?;
    let size = std::fs::metadata(&to)?.len();
    closed_line(room, "tracks", None, &rel(layout, &to), size, reason);
    Ok(to)
}

impl RecConsumer for TrackFiles {
    fn record(&mut self, r: &Record) -> Result<(), String> {
        self.mkv.record(r);
        Ok(())
    }

    fn segment(&mut self, s: &Segment) -> Result<(), String> {
        if s.event == SegmentKind::Opened {
            if s.owner.is_none() {
                return Ok(());
            }
            self.own.insert(s.id);
        }
        if !self.own.contains(&s.id) {
            return Ok(());
        }
        let r = self.mkv.write(&Output::Segment(s.clone())).map_err(|e| format!("tracks mkv: {e}"));
        if s.event == SegmentKind::Closed {
            self.own.remove(&s.id);
            self.settle();
        }
        r
    }

    fn frame(&mut self, f: &Frame) -> Result<(), String> {
        match f.segment {
            Some(id) if self.own.contains(&id) => self.mkv.write(&Output::Frame(f.clone())).map_err(|e| format!("tracks mkv: {e}")),
            _ => Ok(()),
        }
    }

    fn finish(&mut self, reason: &str) -> Result<(), String> {
        let r = self.mkv.close(Some(reason)).map_err(|e| e.to_string());
        self.settle();
        r
    }

    fn keyframe_requests(&mut self) -> Vec<String> {
        self.mkv.take_keyframe_requests()
    }

    fn recover(ctx: &RecoverCtx) {
        for run in &ctx.runs {
            let stamp = run.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for p in files::files_under(&run.join("tracks")) {
                if p.extension().is_none_or(|e| e != "mkv") {
                    continue;
                }
                let Some(room) = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()) else { continue };
                if let Some((_, size)) = recovered(&ctx.layout, &room, "tracks", &p, crate::sink::recover_mkv(&p)) {
                    match settle_track(&ctx.layout, &room, &stamp, &p, ctx.reason) {
                        Ok(_) => recovered_line(&ctx.layout, &room, "tracks", None, &p, size, ctx.reason),
                        Err(e) => tracing::warn!(kind = "diag", spec = SPEC, room = %room, path = %p.display(), "rec recover settle: {e}"),
                    }
                }
            }
        }
    }
}

pub struct RawTap {
    dir: PathBuf,
}

impl RawTap {
    pub fn open(ctx: &RoomCtx) -> io::Result<RawTap> {
        Ok(RawTap { dir: ctx.layout.closed("raw", &ctx.room) })
    }
}

impl RecConsumer for RawTap {
    fn finish(&mut self, _reason: &str) -> Result<(), String> {
        Ok(())
    }

    fn raw_dir(&self) -> Option<PathBuf> {
        Some(self.dir.clone())
    }
}
