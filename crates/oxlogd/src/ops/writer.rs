// author: kodeholic (powered by Claude)
// spec: v1.3 @d4cc756 · 운영:RULE-LOG-LOGGER 정책:logging.rotate_max_bytes 정책:logging.retention_days · model: claude-opus-5-5
//! 파일 쓰기 — 도착순 · 회전(날짜 경계 ∨ 크기 상한 — 쓰는 순간 판정) · 보존(회전으로 닫힌 파일과 spool 만 센다) · fsync 없음.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime};

pub struct Out {
    dir: PathBuf,
    max: u64,
    retention_days: u16,
    date: String,
    idx: u32,
    size: u64,
    file: Option<File>,
    path: Option<PathBuf>,
}

impl Out {
    pub fn new(dir: PathBuf, max: u64, retention_days: u16) -> Out {
        Out {
            dir,
            max,
            retention_days,
            date: String::new(),
            idx: 0,
            size: 0,
            file: None,
            path: None,
        }
    }

    fn name(&self) -> PathBuf {
        if self.idx == 0 {
            self.dir.join(format!("ox.log.{}", self.date))
        } else {
            self.dir.join(format!("ox.log.{}.{}", self.date, self.idx))
        }
    }

    fn open(&mut self) {
        let p = self.name();
        self.size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        self.file = OpenOptions::new().create(true).append(true).open(&p).ok();
        self.path = Some(p);
    }

    pub fn write(&mut self, line: &[u8]) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let n = line.len() as u64 + 1;
        if today != self.date {
            self.date = today;
            self.idx = 0;
            self.open();
            while self.size > 0 && self.size + n > self.max {
                self.idx += 1;
                self.open();
            }
            self.sweep();
        } else if self.size + n > self.max {
            self.idx += 1;
            self.open();
            self.sweep();
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line).and_then(|_| f.write_all(b"\n")).is_ok()
        {
            self.size += n;
        }
    }

    fn sweep(&self) {
        if self.retention_days == 0 {
            return;
        }
        let keep = Duration::from_secs(u64::from(self.retention_days) * 86_400);
        let now = SystemTime::now();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if Some(&p) == self.path.as_ref() || !closed_kind(&p) {
                continue;
            }
            let old = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > keep);
            if old {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

fn closed_kind(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    n.starts_with("ox.log.") || n.ends_with(".spool.log")
}

pub fn run(rx: Receiver<Vec<u8>>, mut out: Out) {
    while let Ok(l) = rx.recv() {
        if l.is_empty() {
            break;
        }
        let mut end = l.len();
        while end > 0 && (l[end - 1] == b'\n' || l[end - 1] == b'\r') {
            end -= 1;
        }
        if end > 0 {
            out.write(&l[..end]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_on_size() {
        let dir = std::env::temp_dir().join(format!("oxlogd-test-{}", common::id::random_hex(4)));
        std::fs::create_dir_all(&dir).expect("dir");
        let mut o = Out::new(dir.clone(), 64, 0);
        for _ in 0..10 {
            o.write(b"0123456789012345678901234567890");
        }
        let n = std::fs::read_dir(&dir).expect("rd").count();
        assert!(n >= 4, "files {n}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
