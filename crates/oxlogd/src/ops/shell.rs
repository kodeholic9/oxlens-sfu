// author: kodeholic (powered by Claude)
// spec: v1.2 @36d1dfd · 정:RULE-OPS-LOGGER · 정:RULE-BUS-FATE · 정:RULE-OPS-OBSERVE · 운영 규격서 §7-1 · model: claude-opus-5-5
//! 로거의 I/O — DGRAM 소켓(수신 버퍼를 정책값으로) · 파일 하나 · 쓰는 순간 회전 · 보존 · 자기 줄은 같은 파일에.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::log::line::{self, Record};
use common::policy::Logging;

use super::core::{self, Rotate};

const RECV_MAX: usize = 65_536;
const POLL_MS: u64 = 200;

pub struct Writer {
    dir: PathBuf,
    max: u64,
    retention_days: u16,
    date: String,
    file: Option<BufWriter<File>>,
    size: u64,
    seq: u32,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Writer {
    pub fn open(dir: &Path, policy: &Logging) -> std::io::Result<Writer> {
        std::fs::create_dir_all(dir)?;
        let mut w = Writer { dir: dir.to_path_buf(), max: policy.rotate_max_bytes, retention_days: policy.retention_days, date: today(), file: None, size: 0, seq: 0 };
        w.reopen()?;
        w.sweep();
        Ok(w)
    }

    fn reopen(&mut self) -> std::io::Result<()> {
        let path = self.dir.join(core::active_name(&self.date));
        let f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        self.size = f.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(BufWriter::new(f));
        Ok(())
    }

    fn rotate_size(&mut self) -> std::io::Result<()> {
        self.flush();
        self.file = None;
        loop {
            self.seq += 1;
            let closed = self.dir.join(core::closed_name(&self.date, self.seq));
            if !closed.exists() {
                std::fs::rename(self.dir.join(core::active_name(&self.date)), closed)?;
                break;
            }
        }
        self.reopen()
    }

    pub fn write(&mut self, line: &[u8]) {
        let now = today();
        match core::rotate(&self.date, &now, self.size, line.len() as u64 + 1, self.max) {
            Rotate::Date => {
                self.flush();
                self.date = now;
                self.seq = 0;
                let _ = self.reopen();
                self.sweep();
            }
            Rotate::Size => {
                let _ = self.rotate_size();
                self.sweep();
            }
            Rotate::No => {}
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line).and_then(|_| f.write_all(b"\n")).is_ok()
        {
            self.size += line.len() as u64 + 1;
        }
    }

    pub fn flush(&mut self) {
        if let Some(f) = self.file.as_mut() {
            let _ = f.flush();
        }
    }

    pub fn sweep(&self) {
        let active = core::active_name(&self.date);
        let now_s = common::time::wall_ms() / 1000;
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !core::counted(&name, &active) {
                continue;
            }
            let mtime = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(now_s);
            if core::expired(mtime, now_s, self.retention_days) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub struct Own {
    pub node: String,
    pub line_max: usize,
}

impl Own {
    pub fn line(&self, fields: &[(&str, String)]) -> Vec<u8> {
        let mut r = Record::default();
        for (k, v) in fields {
            r.set(k, v.clone());
        }
        line::render(&line::now_ts(), common::time::mono_us(), &self.node, "log", &r, self.line_max).into_bytes()
    }
}

pub fn bind(path: &Path, rcvbuf: u32) -> std::io::Result<UnixDatagram> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let sock = UnixDatagram::bind(path)?;
    let want = rcvbuf as usize;
    let _ = nix::sys::socket::setsockopt(std::os::fd::AsRawFd::as_raw_fd(&sock), nix::sys::socket::sockopt::RcvBuf, &want);
    sock.set_read_timeout(Some(Duration::from_millis(POLL_MS)))?;
    Ok(sock)
}

pub struct Stats {
    pub lines: AtomicU64,
    pub bytes: AtomicU64,
}

pub fn serve(sock: UnixDatagram, mut w: Writer, stop: Arc<AtomicBool>, stats: Arc<Stats>) -> Writer {
    let mut buf = vec![0u8; RECV_MAX];
    while !stop.load(Ordering::Relaxed) {
        match sock.recv(&mut buf) {
            Ok(n) => {
                w.write(&buf[..n]);
                stats.lines.fetch_add(1, Ordering::Relaxed);
                stats.bytes.fetch_add(n as u64, Ordering::Relaxed);
                let _ = sock.set_nonblocking(true);
                while let Ok(m) = sock.recv(&mut buf) {
                    w.write(&buf[..m]);
                    stats.lines.fetch_add(1, Ordering::Relaxed);
                    stats.bytes.fetch_add(m as u64, Ordering::Relaxed);
                }
                let _ = sock.set_nonblocking(false);
                w.flush();
            }
            Err(_) => w.flush(),
        }
    }
    let _ = sock.set_nonblocking(true);
    while let Ok(m) = sock.recv(&mut buf) {
        w.write(&buf[..m]);
    }
    w.flush();
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_rotates_by_size_and_keeps_one_active() {
        let dir = std::env::temp_dir().join(format!("oxlogd-t-{}", common::identity::fresh_hex(4)));
        let p = Logging { rotate_max_bytes: 64, retention_days: 14, ..Logging::default() };
        let mut w = Writer::open(&dir, &p).expect("open");
        for _ in 0..10 {
            w.write(b"0123456789012345678901234567890");
        }
        w.flush();
        let names: Vec<String> = std::fs::read_dir(&dir).expect("rd").flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(names.iter().filter(|n| n.starts_with("ox.log.")).count() > 1);
        assert!(names.contains(&core::active_name(&today())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
