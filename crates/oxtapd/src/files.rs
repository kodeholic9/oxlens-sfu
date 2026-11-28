// author: kodeholic (powered by Claude)
// spec: v1.14 · 정:IMPL-OPS-REC-UNIT · 정:IMPL-CHAT-TALK · 연:RULE-CHAT-REC-FILE · model: claude-opus-5-5
//! 녹음 유닛 파일 자리 — 쓰는 중 `<root>/.rec-part/{node}/{방 작업이 연 시각}/` 아래 닫힐 경로 그대로 · 닫힌 `rec/` · `tracks/` · `raw/` · 마감 실패 `.rec-part/failed/` · 닫기는 있으면 실패하는 링크 걸기(덮어쓰지 않고 `-NNNN` 으로 비킨다) · 빈 폴더 걷기 · 보관 지우기(네 폴더).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PART_DIR: &str = ".rec-part";
pub const FAILED_DIR: &str = "failed";
pub const DAY: Duration = Duration::from_secs(86_400);
pub const KEPT: [&str; 3] = ["rec", "tracks", "raw"];
pub const LOCK_FILE: &str = ".lock";

pub fn lock_run(run: &Path) -> io::Result<Option<std::fs::File>> {
    let f = std::fs::File::options().create(true).truncate(false).write(true).open(run.join(LOCK_FILE))?;
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub node: String,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>, node: impl Into<String>) -> Layout {
        Layout { root: root.into(), node: node.into() }
    }

    pub fn staging(&self) -> PathBuf {
        self.root.join(PART_DIR)
    }

    pub fn node_staging(&self) -> PathBuf {
        self.staging().join(&self.node)
    }

    pub fn run_dir(&self, run: &str) -> PathBuf {
        self.node_staging().join(run)
    }

    pub fn failed(&self) -> PathBuf {
        self.staging().join(FAILED_DIR)
    }

    pub fn closed(&self, kind_dir: &str, room: &str) -> PathBuf {
        self.root.join(kind_dir).join(room)
    }

    pub fn runs(&self) -> Vec<PathBuf> {
        let Ok(it) = std::fs::read_dir(self.node_staging()) else { return Vec::new() };
        let mut v: Vec<PathBuf> = it.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
        v.sort();
        v
    }
}

pub fn run_stamp(now: SystemTime) -> String {
    now.duration_since(UNIX_EPOCH).map(|d| d.as_millis().to_string()).unwrap_or_else(|_| "0".into())
}

pub fn numbered_name(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}-{n:04}.{ext}"),
        None => format!("{name}-{n:04}"),
    }
}

pub fn link_no_overwrite(from: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 0..10_000u32 {
        let to = dir.join(numbered_name(name, n));
        match std::fs::hard_link(from, &to) {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(from) {
                    tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", path = %from.display(), "rec staged file not removed after closing: {e} — recovery may close it once more");
                }
                return Ok(to);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("no free name for {name}")))
}

pub fn tracks_closed_name(staged: &str, run: &str) -> String {
    let (stem, ext) = staged.rsplit_once('.').unwrap_or((staged, "mkv"));
    match stem.rsplit_once('-') {
        Some((key, idx)) if !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()) => format!("{key}-{run}-{idx}.{ext}"),
        _ => format!("{stem}-{run}.{ext}"),
    }
}

pub fn talk_of(room: &str, file: &str) -> Option<String> {
    let rest = file.strip_prefix(room)?.strip_prefix('-')?;
    let (talk, _) = rest.split_once('-')?;
    (!talk.is_empty() && common::keyspace::valid_piece(talk)).then(|| talk.to_string())
}

pub const NAME_MAX: usize = 255;

pub fn room_name_ok(room: &str, run: &str) -> bool {
    room != "." && room != ".." && common::keyspace::valid_piece(room) && format!("{room}-{run}-0000.mcap").len() <= NAME_MAX
}

pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(it) = std::fs::read_dir(&d) else { continue };
        for e in it.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => out.push(e.path()),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

pub fn prune_empty(dir: &Path) -> bool {
    let Ok(it) = std::fs::read_dir(dir) else { return false };
    let mut empty = true;
    for e in it.flatten() {
        let gone = e.file_type().is_ok_and(|t| t.is_dir()) && prune_empty(&e.path());
        empty &= gone;
    }
    empty && std::fs::remove_dir(dir).is_ok()
}

pub fn to_failed(layout: &Layout, path: &Path) -> io::Result<PathBuf> {
    let rel = path.strip_prefix(layout.staging()).unwrap_or(path);
    let to = layout.failed().join(rel);
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    link_no_overwrite(path, to.parent().unwrap_or(&layout.failed()), &name)
}

pub fn sweep(root: &Path, keep_days: u32, now: SystemTime) -> io::Result<usize> {
    let cut = now.checked_sub(DAY * keep_days).unwrap_or(UNIX_EPOCH);
    let mut dirs: Vec<PathBuf> = KEPT.iter().map(|k| root.join(k)).collect();
    dirs.push(root.join(PART_DIR).join(FAILED_DIR));
    let mut n = 0;
    for d in dirs {
        for f in files_under(&d) {
            let old = std::fs::metadata(&f).and_then(|m| m.modified()).map(|t| t < cut);
            match old.and_then(|o| if o { std::fs::remove_file(&f).map(|_| true) } else { Ok(false) }) {
                Ok(true) => n += 1,
                Ok(false) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!(kind = "diag", spec = "SV-IMPL-OPS-REC-UNIT", path = %f.display(), "rec sweep: {e}"),
            }
        }
    }
    Ok(n)
}
